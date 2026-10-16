//! NavMesh queries (Unity's `NavMesh.CalculatePath`, `SamplePosition`, `Raycast`,
//! `FindClosestEdge`), built like Detour's: A* over polygons, then the funnel algorithm
//! for the corners.

use crate::navmesh::{NavMesh, PolyRef};
use oxide_math::Vector3;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

/// Unity `NavMeshQueryFilter`: which agent type's NavMesh, which areas, and their costs.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryFilter {
    pub agent_type: i32,
    pub area_mask: u32,
    /// Cost multiplier per area; `None` uses the NavMesh's (project) costs.
    pub costs: Option<[f32; 32]>,
}

impl QueryFilter {
    pub fn new(agent_type: i32) -> Self {
        Self {
            agent_type,
            area_mask: crate::ALL_AREAS,
            costs: None,
        }
    }
    pub fn with_mask(mut self, mask: u32) -> Self {
        self.area_mask = mask;
        self
    }
    pub fn passes(&self, area: u8) -> bool {
        area < 32 && self.area_mask & (1 << area) != 0
    }
    fn cost(&self, nav: &NavMesh, area: u8) -> f32 {
        let a = (area as usize).min(31);
        self.costs.map_or(nav.area_costs[a], |c| c[a]).max(1e-3)
    }
}

/// One step of a path through the NavMesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathNode {
    Poly(PolyRef),
    /// Crossing off-mesh link `id` from its start to its end (`forward`) or back.
    Link {
        id: u32,
        forward: bool,
    },
}

/// Unity `NavMeshPathStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NavMeshPathStatus {
    PathComplete,
    PathPartial,
    #[default]
    PathInvalid,
}

/// Unity `NavMeshPath`.
#[derive(Clone, Debug, Default)]
pub struct NavMeshPath {
    pub corners: Vec<Vector3>,
    pub status: NavMeshPathStatus,
    pub(crate) nodes: Vec<PathNode>,
}

impl NavMeshPath {
    pub fn length(&self) -> f32 {
        self.corners.windows(2).map(|w| w[0].distance(w[1])).sum()
    }
}

/// A path corner; `link` is set on the corner where an off-mesh link starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    pub pos: Vector3,
    pub link: Option<(u32, bool)>,
}

/// Unity `NavMeshHit`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NavMeshHit {
    pub position: Vector3,
    pub normal: Vector3,
    pub distance: f32,
    /// Area mask bit of the polygon at `position`.
    pub mask: u32,
    pub hit: bool,
}

pub(crate) fn cross2(o: Vector3, a: Vector3, b: Vector3) -> f32 {
    (a.x - o.x) * (b.z - o.z) - (a.z - o.z) * (b.x - o.x)
}

fn dist2_xz(a: Vector3, b: Vector3) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    dx * dx + dz * dz
}

/// Closest point on segment `a..b` to `p` in x/z: (squared distance, t).
fn dist_pt_seg2_xz(p: Vector3, a: Vector3, b: Vector3) -> (f32, f32) {
    let ab = Vector3::new(b.x - a.x, 0.0, b.z - a.z);
    let ap = Vector3::new(p.x - a.x, 0.0, p.z - a.z);
    let d = ab.x * ab.x + ab.z * ab.z;
    let t = if d > 0.0 {
        ((ap.x * ab.x + ap.z * ab.z) / d).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = a + (b - a) * t;
    (dist2_xz(p, q), t)
}

fn signed_area_xz(pts: &[Vector3]) -> f32 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.x * b.z - b.x * a.z
        })
        .sum::<f32>()
        * 0.5
}

fn point_on_or_in_poly_xz(pts: &[Vector3], p: Vector3) -> bool {
    let n = pts.len();
    let sgn = signed_area_xz(pts).signum();
    (0..n).all(|i| cross2(pts[i], pts[(i + 1) % n], p) * sgn >= -1e-3)
}

pub(crate) fn point_in_poly_xz(pts: &[Vector3], p: Vector3) -> bool {
    let n = pts.len();
    let s = signed_area_xz(pts).signum();
    (0..n).all(|i| cross2(pts[i], pts[(i + 1) % n], p) * s >= -1e-6)
}

/// Height of the polygon's surface at `p` (x/z), from a triangle fan.
pub(crate) fn interpolate_height(pts: &[Vector3], p: Vector3) -> Option<f32> {
    for k in 1..pts.len() - 1 {
        let (a, b, c) = (pts[0], pts[k], pts[k + 1]);
        let v0 = c - a;
        let v1 = b - a;
        let v2 = p - a;
        let d00 = v0.x * v0.x + v0.z * v0.z;
        let d01 = v0.x * v1.x + v0.z * v1.z;
        let d02 = v0.x * v2.x + v0.z * v2.z;
        let d11 = v1.x * v1.x + v1.z * v1.z;
        let d12 = v1.x * v2.x + v1.z * v2.z;
        let den = d00 * d11 - d01 * d01;
        if den.abs() < 1e-12 {
            continue;
        }
        let u = (d11 * d02 - d01 * d12) / den;
        let v = (d00 * d12 - d01 * d02) / den;
        const EPS: f32 = 1e-4;
        if u >= -EPS && v >= -EPS && u + v <= 1.0 + EPS {
            return Some(a.y + v0.y * u + v1.y * v);
        }
    }
    None
}

#[derive(Clone, Copy)]
struct Open {
    f: f32,
    r: PolyRef,
}
impl PartialEq for Open {
    fn eq(&self, o: &Self) -> bool {
        self.f == o.f
    }
}
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
    }
}

/// How A* reached a polygon.
#[derive(Clone, Copy, Debug)]
enum Via {
    Start,
    Edge(PolyRef),
    Link(PolyRef, u32, bool),
}

const MAX_NODES: usize = 8192;

impl NavMesh {
    fn poly_points(&self, r: PolyRef) -> Option<Vec<Vector3>> {
        let (t, p) = self.poly(r)?;
        Some(p.verts.iter().map(|&v| t.verts[v as usize]).collect())
    }

    /// Area of a polygon.
    pub fn poly_area(&self, r: PolyRef) -> Option<u8> {
        self.poly(r).map(|(_, p)| p.area)
    }

    /// Closest point on polygon `r` to `p`, and whether `p` is over it.
    pub fn closest_point_on_poly(&self, r: PolyRef, p: Vector3) -> Option<(Vector3, bool)> {
        let pts = self.poly_points(r)?;
        if point_in_poly_xz(&pts, p) {
            let y = self.poly_height(r, p);
            return Some((Vector3::new(p.x, y, p.z), true));
        }
        let n = pts.len();
        let mut best = (f32::MAX, pts[0]);
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (d, t) = dist_pt_seg2_xz(p, a, b);
            if d < best.0 {
                best = (d, a + (b - a) * t);
            }
        }
        let q = best.1;
        Some((Vector3::new(q.x, self.poly_height(r, q), q.z), false))
    }

    /// The polygon nearest `center` within the box `center ± extents` (Detour's
    /// `findNearestPoly`), with the nearest point on it.
    pub fn find_nearest_poly(
        &self,
        center: Vector3,
        extents: Vector3,
        filter: &QueryFilter,
    ) -> Option<(PolyRef, Vector3)> {
        let min = center - extents;
        let max = center + extents;
        let climb = self
            .agent_types
            .iter()
            .find(|(id, _)| *id == filter.agent_type)
            .map_or(0.4, |(_, s)| s.agent_climb);
        let mut best: Option<(f32, PolyRef, Vector3)> = None;
        for ti in self.tiles_in(min, max, filter.agent_type) {
            let Some(t) = self.tiles[ti as usize].as_ref() else {
                continue;
            };
            for (pi, p) in t.polys.iter().enumerate() {
                if !filter.passes(p.area)
                    || p.bmin.x > max.x
                    || p.bmax.x < min.x
                    || p.bmin.z > max.z
                    || p.bmax.z < min.z
                    || p.bmin.y > max.y + climb
                    || p.bmax.y < min.y - climb
                {
                    continue;
                }
                let r = PolyRef {
                    tile: ti,
                    poly: pi as u32,
                    salt: t.salt,
                };
                let Some((q, over)) = self.closest_point_on_poly(r, center) else {
                    continue;
                };
                let d = if over {
                    let dy = ((center.y - q.y).abs() - climb).max(0.0);
                    dy * dy
                } else {
                    center.distance_squared(q)
                };
                if (q - center)
                    .abs()
                    .cmple(extents + Vector3::new(0.0, climb, 0.0))
                    .all()
                    && best.is_none_or(|b| d < b.0)
                {
                    best = Some((d, r, q));
                }
            }
        }
        best.map(|(_, r, q)| (r, q))
    }

    /// Unity `NavMesh.SamplePosition`: the nearest NavMesh point within `max_distance`.
    pub fn sample_position(
        &self,
        p: Vector3,
        max_distance: f32,
        filter: &QueryFilter,
    ) -> Option<NavMeshHit> {
        let (r, q) = self.find_nearest_poly(p, Vector3::splat(max_distance.max(0.0)), filter)?;
        let d = p.distance(q);
        (d <= max_distance + 1e-4).then(|| NavMeshHit {
            position: q,
            normal: Vector3::Y,
            distance: d,
            mask: self.poly_area(r).map_or(0, |a| 1 << a.min(31)),
            hit: true,
        })
    }

    /// The open part of the edge between `from` and `to`, as (left, right) seen from `from`.
    pub(crate) fn portal(&self, from: PolyRef, to: PolyRef) -> Option<(Vector3, Vector3)> {
        let (t, p) = self.poly(from)?;
        let l = p.links.iter().find(|l| l.to == to)?;
        let n = p.verts.len();
        let va = t.verts[p.verts[l.edge as usize] as usize];
        let vb = t.verts[p.verts[(l.edge as usize + 1) % n] as usize];
        let a = va + (vb - va) * l.tmin;
        let b = va + (vb - va) * l.tmax;
        // Seen from inside the polygon, the edge runs right-to-left on a counter-clockwise
        // (x/z) polygon.
        let pts: Vec<Vector3> = p.verts.iter().map(|&v| t.verts[v as usize]).collect();
        if signed_area_xz(&pts) > 0.0 {
            Some((b, a))
        } else {
            Some((a, b))
        }
    }

    fn portal_mid(&self, from: PolyRef, to: PolyRef) -> Vector3 {
        self.portal(from, to).map_or_else(
            || self.poly(to).map_or(Vector3::ZERO, |(_, p)| p.center),
            |(a, b)| (a + b) * 0.5,
        )
    }

    /// A* from `start` to `end`. Returns the polygons (and links) and whether `end` was
    /// reached (otherwise the path leads to the polygon closest to it).
    pub fn find_path(
        &self,
        start: PolyRef,
        end: PolyRef,
        start_pos: Vector3,
        end_pos: Vector3,
        filter: &QueryFilter,
    ) -> (Vec<PathNode>, bool) {
        if !self.is_valid_poly(start) || !self.is_valid_poly(end) {
            return (Vec::new(), false);
        }
        if start == end {
            return (vec![PathNode::Poly(start)], true);
        }
        struct NodeState {
            g: f32,
            pos: Vector3,
            via: Via,
            closed: bool,
        }
        let mut nodes: HashMap<PolyRef, NodeState> = HashMap::new();
        let mut open = BinaryHeap::new();
        let h0 = start_pos.distance(end_pos);
        nodes.insert(
            start,
            NodeState {
                g: 0.0,
                pos: start_pos,
                via: Via::Start,
                closed: false,
            },
        );
        open.push(Open { f: h0, r: start });
        let mut best = (h0, start);
        let mut reached = false;
        while let Some(Open { r, .. }) = open.pop() {
            let Some(cur) = nodes.get_mut(&r) else {
                continue;
            };
            if cur.closed {
                continue;
            }
            cur.closed = true;
            let (g, pos) = (cur.g, cur.pos);
            if r == end {
                reached = true;
                best = (0.0, r);
                break;
            }
            let Some((_, poly)) = self.poly(r) else {
                continue;
            };
            let cur_cost = filter.cost(self, poly.area);
            let mut succ: Vec<(PolyRef, Vector3, f32, Via)> = Vec::new();
            for l in &poly.links {
                let Some((_, np)) = self.poly(l.to) else {
                    continue;
                };
                if !filter.passes(np.area) {
                    continue;
                }
                let npos = self.portal_mid(r, l.to);
                let mut c = pos.distance(npos) * cur_cost;
                if l.to == end {
                    c += npos.distance(end_pos) * filter.cost(self, np.area);
                }
                succ.push((l.to, npos, c, Via::Edge(r)));
            }
            for &id in &poly.offmesh {
                let Some(link) = self.links.get(&id) else {
                    continue;
                };
                if !link.activated
                    || !filter.passes(link.area)
                    || link.agent_type != filter.agent_type
                {
                    continue;
                }
                let (forward, target, from_p, to_p) = if link.start_poly == Some(r) {
                    (true, link.end_poly, link.start, link.end)
                } else if link.bidirectional && link.end_poly == Some(r) {
                    (false, link.start_poly, link.end, link.start)
                } else {
                    continue;
                };
                let Some(target) = target else { continue };
                let Some((_, tp)) = self.poly(target) else {
                    continue;
                };
                if !filter.passes(tp.area) {
                    continue;
                }
                let link_cost = if link.cost_modifier >= 0.0 {
                    link.cost_modifier
                } else {
                    filter.cost(self, link.area)
                };
                let mut c = pos.distance(from_p) * cur_cost + from_p.distance(to_p) * link_cost;
                if target == end {
                    c += to_p.distance(end_pos) * filter.cost(self, tp.area);
                }
                succ.push((target, to_p, c, Via::Link(r, id, forward)));
            }
            for (nr, npos, c, via) in succ {
                let ng = g + c;
                let h = if nr == end {
                    0.0
                } else {
                    npos.distance(end_pos) * 0.999
                };
                match nodes.get_mut(&nr) {
                    Some(n) if n.closed || n.g <= ng => continue,
                    Some(n) => {
                        n.g = ng;
                        n.pos = npos;
                        n.via = via;
                    }
                    None => {
                        if nodes.len() >= MAX_NODES {
                            continue;
                        }
                        nodes.insert(
                            nr,
                            NodeState {
                                g: ng,
                                pos: npos,
                                via,
                                closed: false,
                            },
                        );
                    }
                }
                if h < best.0 {
                    best = (h, nr);
                }
                open.push(Open { f: ng + h, r: nr });
            }
        }
        // Walk back from the goal (or the node closest to it).
        let mut out = Vec::new();
        let mut r = best.1;
        let mut guard = 0;
        loop {
            out.push(PathNode::Poly(r));
            guard += 1;
            if guard > MAX_NODES {
                break;
            }
            match nodes.get(&r).map(|n| n.via) {
                Some(Via::Edge(p)) => r = p,
                Some(Via::Link(p, id, fwd)) => {
                    out.push(PathNode::Link { id, forward: fwd });
                    r = p;
                }
                _ => break,
            }
        }
        out.reverse();
        (out, reached)
    }

    /// Corners along a polygon path (the funnel algorithm), up to `max` corners. The first
    /// corner is the start; the last is the end (clamped onto the last polygon).
    pub fn straight_path(
        &self,
        start: Vector3,
        end: Vector3,
        path: &[PathNode],
        max: usize,
    ) -> Vec<Corner> {
        let mut out = vec![Corner {
            pos: start,
            link: None,
        }];
        let mut seg_start = start;
        let mut i = 0;
        while i < path.len() && out.len() < max {
            // Polygons up to the next link.
            let mut j = i;
            while j < path.len() && matches!(path[j], PathNode::Poly(_)) {
                j += 1;
            }
            let polys: Vec<PolyRef> = path[i..j]
                .iter()
                .filter_map(|n| match n {
                    PathNode::Poly(r) => Some(*r),
                    _ => None,
                })
                .collect();
            let (seg_end, link) = match path.get(j) {
                Some(PathNode::Link { id, forward }) => {
                    let Some(l) = self.links.get(id) else { break };
                    let (a, b) = if *forward {
                        (l.start, l.end)
                    } else {
                        (l.end, l.start)
                    };
                    (a, Some((*id, *forward, b)))
                }
                _ => (end, None),
            };
            let mut portals = Vec::with_capacity(polys.len() + 1);
            portals.push((seg_start, seg_start));
            for w in polys.windows(2) {
                match self.portal(w[0], w[1]) {
                    Some(p) => portals.push(p),
                    None => break,
                }
            }
            portals.push((seg_end, seg_end));
            funnel(&portals, &mut out, max);
            match link {
                Some((id, fwd, b)) => {
                    if let Some(last) = out.last_mut() {
                        last.link = Some((id, fwd));
                    }
                    if out.len() < max {
                        out.push(Corner { pos: b, link: None });
                    }
                    seg_start = b;
                    i = j + 1;
                }
                None => break,
            }
        }
        out.truncate(max);
        out
    }

    /// Unity `NavMesh.CalculatePath`.
    pub fn calculate_path(
        &self,
        source: Vector3,
        target: Vector3,
        filter: &QueryFilter,
    ) -> NavMeshPath {
        let ext = Vector3::new(2.0, 4.0, 2.0);
        let (Some((s, sp)), Some((e, ep))) = (
            self.find_nearest_poly(source, ext, filter),
            self.find_nearest_poly(target, ext, filter),
        ) else {
            return NavMeshPath::default();
        };
        self.path_between(s, sp, e, ep, filter)
    }

    pub(crate) fn path_between(
        &self,
        s: PolyRef,
        sp: Vector3,
        e: PolyRef,
        ep: Vector3,
        filter: &QueryFilter,
    ) -> NavMeshPath {
        let (nodes, complete) = self.find_path(s, e, sp, ep, filter);
        if nodes.is_empty() {
            return NavMeshPath::default();
        }
        let nodes = if complete {
            self.shortcut(nodes, sp, ep, filter)
        } else {
            nodes
        };
        let goal = if complete {
            ep
        } else {
            let last = match nodes.last() {
                Some(PathNode::Poly(r)) => *r,
                _ => s,
            };
            self.closest_point_on_poly(last, ep).map_or(ep, |(q, _)| q)
        };
        let corners = self
            .straight_path(sp, goal, &nodes, usize::MAX)
            .into_iter()
            .map(|c| c.pos)
            .collect();
        NavMeshPath {
            corners,
            status: if complete {
                NavMeshPathStatus::PathComplete
            } else {
                NavMeshPathStatus::PathPartial
            },
            nodes,
        }
    }

    /// Walks the NavMesh surface from `start` towards `end` (Detour's `moveAlongSurface`):
    /// slides along walls instead of crossing them. Returns the reached position and the
    /// polygons visited, from `start_ref` to the one under the result.
    pub fn move_along_surface(
        &self,
        start_ref: PolyRef,
        start: Vector3,
        end: Vector3,
        filter: &QueryFilter,
    ) -> (Vector3, Vec<PolyRef>) {
        let mut best_pos = start;
        let mut best_dist = f32::MAX;
        let mut best = start_ref;
        let search_pos = (start + end) * 0.5;
        let rad = start.distance(end) * 0.5 + 0.001;
        let rad2 = rad * rad;
        let mut parent: HashMap<PolyRef, Option<PolyRef>> = HashMap::new();
        parent.insert(start_ref, None);
        let mut queue = VecDeque::from([start_ref]);
        while let Some(cur) = queue.pop_front() {
            let Some((t, p)) = self.poly(cur) else {
                continue;
            };
            let pts: Vec<Vector3> = p.verts.iter().map(|&v| t.verts[v as usize]).collect();
            if point_in_poly_xz(&pts, end) {
                best = cur;
                best_pos = end;
                break;
            }
            let n = pts.len();
            for e in 0..n {
                let (a, b) = (pts[e], pts[(e + 1) % n]);
                let neis: Vec<PolyRef> = p
                    .links
                    .iter()
                    .filter(|l| l.edge as usize == e)
                    .filter(|l| {
                        self.poly(l.to)
                            .is_some_and(|(_, np)| filter.passes(np.area))
                    })
                    .map(|l| l.to)
                    .collect();
                if neis.is_empty() {
                    let (d, tt) = dist_pt_seg2_xz(end, a, b);
                    if d < best_dist {
                        best_dist = d;
                        best_pos = a + (b - a) * tt;
                        best = cur;
                    }
                } else {
                    for nr in neis {
                        if parent.contains_key(&nr) || parent.len() >= 64 {
                            continue;
                        }
                        if dist_pt_seg2_xz(search_pos, a, b).0 > rad2 {
                            continue;
                        }
                        parent.insert(nr, Some(cur));
                        queue.push_back(nr);
                    }
                }
            }
        }
        let mut visited = vec![best];
        let mut r = best;
        while let Some(Some(p)) = parent.get(&r) {
            visited.push(*p);
            r = *p;
        }
        visited.reverse();
        let y = self.poly_height(best, best_pos);
        (Vector3::new(best_pos.x, y, best_pos.z), visited)
    }

    /// Unity `NavMesh.Raycast`: traces from `source` towards `target` along the NavMesh.
    /// `hit.hit` is true if something blocked it before `target`.
    pub fn raycast(&self, source: Vector3, target: Vector3, filter: &QueryFilter) -> NavMeshHit {
        let Some((start, sp)) = self.find_nearest_poly(source, Vector3::new(1.0, 2.0, 1.0), filter)
        else {
            return NavMeshHit {
                position: source,
                hit: true,
                ..Default::default()
            };
        };
        self.raycast_walk(start, sp, target, filter).0
    }

    /// Walks the polygons along `sp → target` from `start`; returns the hit and the polygons
    /// crossed (Detour's `raycast`).
    pub(crate) fn raycast_walk(
        &self,
        start: PolyRef,
        sp: Vector3,
        target: Vector3,
        filter: &QueryFilter,
    ) -> (NavMeshHit, Vec<PolyRef>) {
        let dir = target - sp;
        let mut cur = start;
        let mut visited = HashSet::new();
        let mut path = Vec::new();
        loop {
            visited.insert(cur);
            path.push(cur);
            let Some((t, p)) = self.poly(cur) else { break };
            let pts: Vec<Vector3> = p.verts.iter().map(|&v| t.verts[v as usize]).collect();
            let n = pts.len();
            let c = p.center;
            let mut tmax = 1.0f32;
            let mut exit: Option<usize> = None;
            for e in 0..n {
                let (a, b) = (pts[e], pts[(e + 1) % n]);
                let edge = b - a;
                let mut nin = Vector3::new(-edge.z, 0.0, edge.x);
                if nin.x * (c.x - a.x) + nin.z * (c.z - a.z) < 0.0 {
                    nin = -nin;
                }
                let num = nin.x * (sp.x - a.x) + nin.z * (sp.z - a.z);
                let den = nin.x * dir.x + nin.z * dir.z;
                if den.abs() < 1e-9 {
                    continue;
                }
                let tt = -num / den;
                if den < 0.0 && tt < tmax {
                    tmax = tt;
                    exit = Some(e);
                }
            }
            let Some(e) = exit else {
                // Target is inside this polygon.
                let hit = NavMeshHit {
                    position: Vector3::new(target.x, self.poly_height(cur, target), target.z),
                    normal: Vector3::ZERO,
                    distance: sp.distance(target),
                    mask: 1 << p.area.min(31),
                    hit: false,
                };
                return (hit, path);
            };
            let tmax = tmax.max(0.0);
            let hitp = sp + dir * tmax;
            let (a, b) = (pts[e], pts[(e + 1) % n]);
            let along = dist_pt_seg2_xz(hitp, a, b).1;
            let next = p
                .links
                .iter()
                .filter(|l| {
                    l.edge as usize == e && along >= l.tmin - 1e-3 && along <= l.tmax + 1e-3
                })
                .find(|l| {
                    !visited.contains(&l.to)
                        && self
                            .poly(l.to)
                            .is_some_and(|(_, np)| filter.passes(np.area))
                })
                .map(|l| l.to);
            match next {
                Some(nr) => cur = nr,
                None => {
                    let edge = b - a;
                    let mut nrm = Vector3::new(-edge.z, 0.0, edge.x).normalize_or_zero();
                    if nrm.x * (c.x - a.x) + nrm.z * (c.z - a.z) < 0.0 {
                        nrm = -nrm;
                    }
                    let pos = Vector3::new(hitp.x, self.poly_height(cur, hitp), hitp.z);
                    let hit = NavMeshHit {
                        position: pos,
                        normal: nrm,
                        distance: sp.distance(pos),
                        mask: 1 << p.area.min(31),
                        hit: true,
                    };
                    return (hit, path);
                }
            }
        }
        let hit = NavMeshHit {
            position: sp,
            hit: true,
            ..Default::default()
        };
        (hit, path)
    }

    /// Removes needless corners: where a corner's neighbours see each other across the
    /// NavMesh (no wall, no costlier area in between), the corridor is replaced by the
    /// polygons along that line (Detour's path visibility optimization).
    pub(crate) fn shortcut(
        &self,
        mut nodes: Vec<PathNode>,
        start: Vector3,
        goal: Vector3,
        filter: &QueryFilter,
    ) -> Vec<PathNode> {
        if nodes.iter().any(|n| matches!(n, PathNode::Link { .. })) {
            return nodes;
        }
        let polys = |nodes: &[PathNode]| -> Vec<PolyRef> {
            nodes
                .iter()
                .filter_map(|n| match n {
                    PathNode::Poly(r) => Some(*r),
                    _ => None,
                })
                .collect()
        };
        for _ in 0..32 {
            let corners = self.straight_path(start, goal, &nodes, 256);
            if corners.len() < 3 {
                break;
            }
            let path = polys(&nodes);
            let mut changed = false;
            for k in 0..corners.len() - 2 {
                let (a, c) = (corners[k].pos, corners[k + 2].pos);
                // The corridor polygon under corner a (the first that contains it).
                let Some(ia) = path.iter().position(|&r| {
                    self.poly_points(r)
                        .is_some_and(|pts| point_on_or_in_poly_xz(&pts, a))
                }) else {
                    continue;
                };
                let (hit, visited) = self.raycast_walk(path[ia], a, c, filter);
                if hit.hit {
                    continue;
                }
                let Some(&last) = visited.last() else {
                    continue;
                };
                let Some(ic) = path.iter().rposition(|&r| r == last) else {
                    continue;
                };
                if ic <= ia {
                    continue;
                }
                // Same or cheaper cost: every polygon on the line must cost no more than
                // the corridor's most expensive one between a and c.
                let max_cost = path[ia..=ic]
                    .iter()
                    .filter_map(|&r| self.poly_area(r))
                    .map(|ar| filter.cost(self, ar))
                    .fold(0.0, f32::max);
                if visited
                    .iter()
                    .filter_map(|&r| self.poly_area(r))
                    .any(|ar| filter.cost(self, ar) > max_cost + 1e-4)
                {
                    continue;
                }
                let mut np: Vec<PolyRef> = path[..ia].to_vec();
                np.extend_from_slice(&visited);
                np.extend_from_slice(&path[ic + 1..]);
                if np != path {
                    nodes = np.into_iter().map(PathNode::Poly).collect();
                    changed = true;
                    break;
                }
            }
            if !changed {
                break;
            }
        }
        nodes
    }

    /// Unity `NavMesh.FindClosestEdge`: the nearest NavMesh boundary to `p`.
    pub fn find_closest_edge(&self, p: Vector3, filter: &QueryFilter) -> Option<NavMeshHit> {
        let (start, sp) = self.find_nearest_poly(p, Vector3::new(2.0, 4.0, 2.0), filter)?;
        let mut best: Option<(f32, Vector3, Vector3)> = None;
        let mut seen = HashSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(cur) = queue.pop_front() {
            let Some((t, poly)) = self.poly(cur) else {
                continue;
            };
            let pts: Vec<Vector3> = poly.verts.iter().map(|&v| t.verts[v as usize]).collect();
            let n = pts.len();
            for e in 0..n {
                let (a, b) = (pts[e], pts[(e + 1) % n]);
                let (d, tt) = dist_pt_seg2_xz(sp, a, b);
                let open: Vec<PolyRef> = poly
                    .links
                    .iter()
                    .filter(|l| l.edge as usize == e)
                    .filter(|l| {
                        self.poly(l.to)
                            .is_some_and(|(_, np)| filter.passes(np.area))
                    })
                    .map(|l| l.to)
                    .collect();
                if open.is_empty() {
                    if best.is_none_or(|b| d < b.0) {
                        let q = a + (b - a) * tt;
                        let edge = b - a;
                        let mut nrm = Vector3::new(-edge.z, 0.0, edge.x).normalize_or_zero();
                        if nrm.x * (poly.center.x - a.x) + nrm.z * (poly.center.z - a.z) < 0.0 {
                            nrm = -nrm;
                        }
                        best = Some((d, q, nrm));
                    }
                } else if best.is_none_or(|b| d < b.0) && seen.len() < 256 {
                    for nr in open {
                        if seen.insert(nr) {
                            queue.push_back(nr);
                        }
                    }
                }
            }
        }
        let (d, q, n) = best?;
        Some(NavMeshHit {
            position: q,
            normal: n,
            distance: d.sqrt(),
            mask: self.poly_area(start).map_or(0, |a| 1 << a.min(31)),
            hit: true,
        })
    }
}

/// The simple stupid funnel algorithm; appends corners after the first portal (the start).
fn funnel(portals: &[(Vector3, Vector3)], out: &mut Vec<Corner>, max: usize) {
    let n = portals.len();
    let mut apex = portals[0].0;
    let (mut left, mut right) = (portals[0].0, portals[0].1);
    let (mut left_i, mut right_i) = (0usize, 0usize);
    let mut i = 1;
    let push = |out: &mut Vec<Corner>, p: Vector3| {
        if out
            .last()
            .is_none_or(|c| dist2_xz(c.pos, p) > 1e-8 || (c.pos.y - p.y).abs() > 1e-3)
        {
            out.push(Corner { pos: p, link: None });
        }
    };
    while i < n {
        if out.len() >= max {
            return;
        }
        let (l, r) = portals[i];
        // Right side.
        if cross2(apex, right, r) >= 0.0 {
            if dist2_xz(apex, right) < 1e-10 || cross2(apex, left, r) < 0.0 {
                right = r;
                right_i = i;
            } else {
                push(out, left);
                apex = left;
                let apex_i = left_i;
                right = apex;
                right_i = apex_i;
                i = apex_i + 1;
                continue;
            }
        }
        // Left side.
        if cross2(apex, left, l) <= 0.0 {
            if dist2_xz(apex, left) < 1e-10 || cross2(apex, right, l) > 0.0 {
                left = l;
                left_i = i;
            } else {
                push(out, right);
                apex = right;
                let apex_i = right_i;
                left = apex;
                left_i = apex_i;
                i = apex_i + 1;
                continue;
            }
        }
        i += 1;
    }
    push(out, portals[n - 1].0);
}
