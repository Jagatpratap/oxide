//! The runtime NavMesh (Unity's `NavMesh` class): every loaded NavMeshData instance's tiles in
//! world space, stitched across tile borders, plus the off-mesh links (NavMeshLinks).

use crate::build::Nei;
use crate::data::{NavMeshData, NavMeshTile};
use oxide_math::{Quaternion, Vector3};
use std::collections::HashMap;
use std::sync::Arc;

/// A polygon of the runtime NavMesh. Invalid once its tile is removed or rebuilt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PolyRef {
    pub(crate) tile: u32,
    pub(crate) poly: u32,
    pub(crate) salt: u32,
}

/// Unity `NavMeshDataInstance`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NavMeshDataInstance(pub(crate) u32);

/// A connection from one polygon to another (through a shared edge).
#[derive(Clone, Copy, Debug)]
pub(crate) struct EdgeLink {
    pub to: PolyRef,
    pub edge: u8,
    /// The part of the edge that is open, as fractions along it.
    pub tmin: f32,
    pub tmax: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct RtPoly {
    pub verts: Vec<u16>,
    pub area: u8,
    pub links: Vec<EdgeLink>,
    /// Off-mesh links that start (or, if bidirectional, end) on this polygon.
    pub offmesh: Vec<u32>,
    pub center: Vector3,
    pub bmin: Vector3,
    pub bmax: Vector3,
}

pub(crate) struct Tile {
    pub salt: u32,
    pub instance: u32,
    pub verts: Vec<Vector3>,
    pub polys: Vec<RtPoly>,
    pub bmin: Vector3,
    pub bmax: Vector3,
    /// The tile's data (for floor heights), in instance-local space.
    pub source: Arc<NavMeshTile>,
}

pub(crate) struct Instance {
    pub agent_type: i32,
    pub data: Arc<NavMeshData>,
    pub position: Vector3,
    pub rotation: Quaternion,
    pub tiles: HashMap<(i32, i32), u32>,
}

impl Instance {
    pub fn to_world(&self, p: Vector3) -> Vector3 {
        self.position + self.rotation * p
    }
    pub fn to_local(&self, p: Vector3) -> Vector3 {
        self.rotation.inverse() * (p - self.position)
    }
}

/// Unity `OffMeshLinkData` / a NavMeshLink registered on the NavMesh.
#[derive(Clone, Debug)]
pub struct OffMeshLink {
    pub start: Vector3,
    pub end: Vector3,
    /// Width of the link (0 = a point).
    pub width: f32,
    pub bidirectional: bool,
    pub area: u8,
    /// Multiplies the area cost when ≥ 0 (Unity `costModifier`).
    pub cost_modifier: f32,
    pub agent_type: i32,
    pub activated: bool,
    /// Who registered it (the NavMeshLink's GameObject id), for updates and removal.
    pub owner: u64,
    pub(crate) start_poly: Option<PolyRef>,
    pub(crate) end_poly: Option<PolyRef>,
}

impl OffMeshLink {
    pub fn is_connected(&self) -> bool {
        self.start_poly.is_some() && self.end_poly.is_some()
    }
}

/// Unity `NavMesh.AllAreas`.
pub const ALL_AREAS: u32 = u32::MAX;
const HASH_CELL: f32 = 32.0;

/// World resource: the NavMesh all agents and queries use.
#[derive(Default)]
pub struct NavMesh {
    pub(crate) instances: HashMap<u32, Instance>,
    pub(crate) tiles: Vec<Option<Tile>>,
    free_tiles: Vec<u32>,
    pub(crate) links: HashMap<u32, OffMeshLink>,
    next_instance: u32,
    next_link: u32,
    next_salt: u32,
    hash: HashMap<(i32, i32), Vec<u32>>,
    /// Bumped whenever polygons change (agents replan).
    pub version: u64,
    /// Area costs (Unity `NavMesh.GetAreaCost`), indexed by area.
    pub area_costs: [f32; 32],
    /// Area names, for debug display.
    pub area_names: Vec<String>,
    /// Agent type settings (id, radius, height, step, slope) for queries that need them.
    pub agent_types: Vec<(i32, crate::build::BuildSettings)>,
}

impl NavMesh {
    pub fn new() -> Self {
        Self {
            area_costs: [1.0; 32],
            ..Default::default()
        }
    }

    /// Unity `NavMesh.AddNavMeshData`.
    pub fn add_data(
        &mut self,
        data: Arc<NavMeshData>,
        position: Vector3,
        rotation: Quaternion,
    ) -> NavMeshDataInstance {
        self.next_instance += 1;
        let id = self.next_instance;
        self.instances.insert(
            id,
            Instance {
                agent_type: data.agent_type_id,
                data: data.clone(),
                position,
                rotation,
                tiles: HashMap::new(),
            },
        );
        for t in &data.tiles {
            self.insert_tile(id, Arc::new(t.clone()));
        }
        let keys: Vec<(i32, i32)> = data.tiles.iter().map(|t| (t.x, t.z)).collect();
        for k in keys {
            self.stitch(id, k);
        }
        self.after_change();
        NavMeshDataInstance(id)
    }

    /// Unity `NavMesh.RemoveNavMeshData`.
    pub fn remove_data(&mut self, inst: NavMeshDataInstance) {
        let Some(i) = self.instances.remove(&inst.0) else {
            return;
        };
        for (_, t) in i.tiles {
            self.free_tile(t);
        }
        self.after_change();
    }

    pub fn instance_data(&self, inst: NavMeshDataInstance) -> Option<&Arc<NavMeshData>> {
        self.instances.get(&inst.0).map(|i| &i.data)
    }

    pub fn is_valid(&self, inst: NavMeshDataInstance) -> bool {
        self.instances.contains_key(&inst.0)
    }

    /// Replaces (or with `None` removes) one tile of an instance, e.g. after carving.
    pub fn replace_tile(
        &mut self,
        inst: NavMeshDataInstance,
        key: (i32, i32),
        tile: Option<NavMeshTile>,
    ) {
        let Some(i) = self.instances.get_mut(&inst.0) else {
            return;
        };
        if let Some(old) = i.tiles.remove(&key) {
            self.free_tile(old);
        }
        match tile {
            Some(t) => {
                self.insert_tile(inst.0, Arc::new(t));
                self.stitch(inst.0, key);
            }
            // Drop the neighbours' links into the removed tile.
            None => {
                for d in 0..4 {
                    let nk = (key.0 + crate::build_dx(d), key.1 + crate::build_dz(d));
                    self.restitch_edge(inst.0, nk, ((d + 2) % 4) as u8);
                }
            }
        }
        self.after_change();
    }

    fn insert_tile(&mut self, inst: u32, src: Arc<NavMeshTile>) -> u32 {
        let i = &self.instances[&inst];
        let verts: Vec<Vector3> = src.verts.iter().map(|&v| i.to_world(v)).collect();
        let wide = i.data.settings.agent_height.max(2.0) * 4.0;
        let hg = &src.heights;
        // The ground under a polygon can bulge far from its corners (a mound in a big
        // polygon): widen its height range with the height grid so queries find it.
        let ground_range = |p: &crate::data::NavPoly| -> Option<(f32, f32)> {
            if hg.cells.is_empty() {
                return None;
            }
            let lp: Vec<Vector3> = p.verts.iter().map(|&v| src.verts[v as usize]).collect();
            let (mut lmin, mut lmax) = (lp[0], lp[0]);
            for q in &lp {
                lmin = lmin.min(*q);
                lmax = lmax.max(*q);
            }
            let cell = |v: f32, o: f32| ((v - o) / hg.cs).floor() as i32;
            let n = hg.size as i32;
            let (x0, x1) = (
                cell(lmin.x, hg.origin.x).max(0),
                cell(lmax.x, hg.origin.x).min(n - 1),
            );
            let (z0, z1) = (
                cell(lmin.z, hg.origin.z).max(0),
                cell(lmax.z, hg.origin.z).min(n - 1),
            );
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for z in (z0..=z1).step_by(2) {
                for x in (x0..=x1).step_by(2) {
                    let (s, c) = hg.cells[(x + z * n) as usize];
                    for k in s..s + c as u32 {
                        let y = hg.origin.y + hg.ys[k as usize] as f32 * hg.ch;
                        if y >= lmin.y - wide && y <= lmax.y + wide {
                            lo = lo.min(y);
                            hi = hi.max(y);
                        }
                    }
                }
            }
            (lo <= hi).then(|| {
                let a = i.to_world(Vector3::new(lmin.x, lo, lmin.z)).y;
                let b = i.to_world(Vector3::new(lmin.x, hi, lmin.z)).y;
                (a.min(b), a.max(b))
            })
        };
        let mut bmin = Vector3::splat(f32::MAX);
        let mut bmax = Vector3::splat(f32::MIN);
        let polys: Vec<RtPoly> = src
            .polys
            .iter()
            .map(|p| {
                let pts: Vec<Vector3> = p.verts.iter().map(|&v| verts[v as usize]).collect();
                let (mut pmin, mut pmax) = (pts[0], pts[0]);
                let mut c = Vector3::ZERO;
                for &q in &pts {
                    pmin = pmin.min(q);
                    pmax = pmax.max(q);
                    c += q;
                }
                if let Some((lo, hi)) = ground_range(p) {
                    pmin.y = pmin.y.min(lo);
                    pmax.y = pmax.y.max(hi);
                }
                bmin = bmin.min(pmin);
                bmax = bmax.max(pmax);
                RtPoly {
                    verts: p.verts.clone(),
                    area: p.area,
                    links: Vec::new(),
                    offmesh: Vec::new(),
                    center: c / pts.len() as f32,
                    bmin: pmin,
                    bmax: pmax,
                }
            })
            .collect();
        self.next_salt = self.next_salt.wrapping_add(1).max(1);
        let salt = self.next_salt;
        let tile = Tile {
            salt,
            instance: inst,
            verts,
            polys,
            bmin,
            bmax,
            source: src.clone(),
        };
        let idx = match self.free_tiles.pop() {
            Some(f) => {
                self.tiles[f as usize] = Some(tile);
                f
            }
            None => {
                self.tiles.push(Some(tile));
                self.tiles.len() as u32 - 1
            }
        };
        // Links inside the tile.
        let t = self.tiles[idx as usize]
            .as_mut()
            .unwrap_or_else(|| unreachable!());
        for (pi, p) in src.polys.iter().enumerate() {
            for (e, n) in p.neis.iter().enumerate() {
                if let Nei::Poly(o) = n {
                    t.polys[pi].links.push(EdgeLink {
                        to: PolyRef {
                            tile: idx,
                            poly: *o,
                            salt,
                        },
                        edge: e as u8,
                        tmin: 0.0,
                        tmax: 1.0,
                    });
                }
            }
        }
        self.instances
            .get_mut(&inst)
            .map(|i| i.tiles.insert((src.x, src.z), idx));
        idx
    }

    fn free_tile(&mut self, t: u32) {
        if self.tiles.get(t as usize).is_some_and(|x| x.is_some()) {
            self.tiles[t as usize] = None;
            self.free_tiles.push(t);
        }
    }

    /// Links tile `key` with its four neighbours (both directions).
    fn stitch(&mut self, inst: u32, key: (i32, i32)) {
        for d in 0..4u8 {
            self.connect_edge(inst, key, d);
            let nk = (
                key.0 + crate::build_dx(d as usize),
                key.1 + crate::build_dz(d as usize),
            );
            self.restitch_edge(inst, nk, (d + 2) % 4);
        }
    }

    /// Rebuilds tile `key`'s links through its border in direction `d`.
    fn restitch_edge(&mut self, inst: u32, key: (i32, i32), d: u8) {
        let Some(&ti) = self.instances.get(&inst).and_then(|i| i.tiles.get(&key)) else {
            return;
        };
        let Some(t) = self.tiles[ti as usize].as_mut() else {
            return;
        };
        let src = t.source.clone();
        for (pi, p) in src.polys.iter().enumerate() {
            let portal_edges: Vec<u8> = p
                .neis
                .iter()
                .enumerate()
                .filter(|(_, n)| **n == Nei::Portal(d))
                .map(|(e, _)| e as u8)
                .collect();
            if !portal_edges.is_empty() {
                t.polys[pi]
                    .links
                    .retain(|l| !portal_edges.contains(&l.edge));
            }
        }
        self.connect_edge(inst, key, d);
    }

    /// Adds links from tile `key`'s portal edges in direction `d` to the neighbouring tile.
    fn connect_edge(&mut self, inst: u32, key: (i32, i32), d: u8) {
        let Some(i) = self.instances.get(&inst) else {
            return;
        };
        let climb = i.data.settings.agent_climb.max(0.1);
        let nk = (
            key.0 + crate::build_dx(d as usize),
            key.1 + crate::build_dz(d as usize),
        );
        let (Some(&a), Some(&b)) = (i.tiles.get(&key), i.tiles.get(&nk)) else {
            return;
        };
        let (Some(ta), Some(tb)) = (
            self.tiles[a as usize].as_ref(),
            self.tiles[b as usize].as_ref(),
        ) else {
            return;
        };
        let opp = Nei::Portal((d + 2) % 4);
        // Axis along the shared border in local space: z for ±x borders, x for ±z.
        let along = |v: Vector3| if d == 0 || d == 2 { v.z } else { v.x };
        let mut new_links = Vec::new();
        for (pi, p) in ta.source.polys.iter().enumerate() {
            let n = p.verts.len();
            for e in 0..n {
                if p.neis[e] != Nei::Portal(d) {
                    continue;
                }
                let va = ta.source.verts[p.verts[e] as usize];
                let vb = ta.source.verts[p.verts[(e + 1) % n] as usize];
                let (a0, a1) = (along(va), along(vb));
                let (amin, amax) = (a0.min(a1), a0.max(a1));
                if amax - amin < 1e-4 {
                    continue;
                }
                for (qi, q) in tb.source.polys.iter().enumerate() {
                    let m = q.verts.len();
                    for f in 0..m {
                        if q.neis[f] != opp {
                            continue;
                        }
                        let wa = tb.source.verts[q.verts[f] as usize];
                        let wb = tb.source.verts[q.verts[(f + 1) % m] as usize];
                        let (b0, b1) = (along(wa), along(wb));
                        let lo = amin.max(b0.min(b1));
                        let hi = amax.min(b0.max(b1));
                        if hi - lo < 0.01 {
                            continue;
                        }
                        // Heights at the overlap's middle must match within a step.
                        let mid = (lo + hi) * 0.5;
                        let ya = va.y + (vb.y - va.y) * ((mid - a0) / (a1 - a0)).clamp(0.0, 1.0);
                        let yb = if (b1 - b0).abs() > 1e-6 {
                            wa.y + (wb.y - wa.y) * ((mid - b0) / (b1 - b0)).clamp(0.0, 1.0)
                        } else {
                            wa.y
                        };
                        if (ya - yb).abs() > climb {
                            continue;
                        }
                        let ta0 = ((lo - a0) / (a1 - a0)).clamp(0.0, 1.0);
                        let ta1 = ((hi - a0) / (a1 - a0)).clamp(0.0, 1.0);
                        new_links.push((
                            pi,
                            EdgeLink {
                                to: PolyRef {
                                    tile: b,
                                    poly: qi as u32,
                                    salt: tb.salt,
                                },
                                edge: e as u8,
                                tmin: ta0.min(ta1),
                                tmax: ta0.max(ta1),
                            },
                        ));
                    }
                }
            }
        }
        if let Some(ta) = self.tiles[a as usize].as_mut() {
            for (pi, l) in new_links {
                ta.polys[pi].links.push(l);
            }
        }
    }

    fn after_change(&mut self) {
        self.version += 1;
        self.hash.clear();
        for (i, t) in self.tiles.iter().enumerate() {
            let Some(t) = t else { continue };
            let (x0, z0) = hash_cell(t.bmin);
            let (x1, z1) = hash_cell(t.bmax);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    self.hash.entry((x, z)).or_default().push(i as u32);
                }
            }
        }
        self.reconnect_links();
    }

    pub(crate) fn tile(&self, r: PolyRef) -> Option<&Tile> {
        self.tiles
            .get(r.tile as usize)?
            .as_ref()
            .filter(|t| t.salt == r.salt)
    }

    pub(crate) fn poly(&self, r: PolyRef) -> Option<(&Tile, &RtPoly)> {
        let t = self.tile(r)?;
        Some((t, t.polys.get(r.poly as usize)?))
    }

    /// Whether `r` still refers to a polygon (Unity's agents replan when theirs vanish).
    pub fn is_valid_poly(&self, r: PolyRef) -> bool {
        self.poly(r).is_some()
    }

    pub(crate) fn agent_type_of(&self, t: &Tile) -> i32 {
        self.instances.get(&t.instance).map_or(0, |i| i.agent_type)
    }

    /// Tiles (of `agent_type`) whose bounds overlap the box.
    pub(crate) fn tiles_in(&self, min: Vector3, max: Vector3, agent_type: i32) -> Vec<u32> {
        let (x0, z0) = hash_cell(min);
        let (x1, z1) = hash_cell(max);
        let mut out: Vec<u32> = Vec::new();
        if (x1 - x0 + 1) as i64 * (z1 - z0 + 1) as i64 > 4096 {
            // Huge query: scan every tile.
            out = (0..self.tiles.len() as u32).collect();
        } else {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    if let Some(v) = self.hash.get(&(x, z)) {
                        out.extend_from_slice(v);
                    }
                }
            }
            out.sort_unstable();
            out.dedup();
        }
        out.retain(|&i| {
            self.tiles[i as usize].as_ref().is_some_and(|t| {
                t.bmin.x <= max.x
                    && t.bmax.x >= min.x
                    && t.bmin.y <= max.y
                    && t.bmax.y >= min.y
                    && t.bmin.z <= max.z
                    && t.bmax.z >= min.z
                    && self.agent_type_of(t) == agent_type
            })
        });
        out
    }

    pub fn polygon_count(&self) -> usize {
        self.tiles.iter().flatten().map(|t| t.polys.len()).sum()
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.iter().flatten().count()
    }

    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Every polygon as (world vertices, area, agent type), for drawing and export
    /// (Unity `NavMesh.CalculateTriangulation`).
    pub fn polygons(&self) -> impl Iterator<Item = (Vec<Vector3>, u8, i32)> + '_ {
        self.tiles.iter().flatten().flat_map(move |t| {
            let at = self.agent_type_of(t);
            t.polys.iter().map(move |p| {
                (
                    p.verts.iter().map(|&v| t.verts[v as usize]).collect(),
                    p.area,
                    at,
                )
            })
        })
    }

    /// Unity `NavMesh.CalculateTriangulation`: vertices, triangle indices and per-triangle areas.
    pub fn calculate_triangulation(&self) -> (Vec<Vector3>, Vec<u32>, Vec<u8>) {
        let mut verts = Vec::new();
        let mut idx = Vec::new();
        let mut areas = Vec::new();
        for (pts, area, _) in self.polygons() {
            let base = verts.len() as u32;
            verts.extend_from_slice(&pts);
            for k in 1..pts.len() as u32 - 1 {
                idx.extend_from_slice(&[base, base + k, base + k + 1]);
                areas.push(area);
            }
        }
        (verts, idx, areas)
    }

    /// The NavMesh draped over the ground for drawing: polygons subdivided to about
    /// `step` metres with heights from the floor height grid. Returns vertices, triangles
    /// and the area of each triangle.
    pub fn display_mesh(&self, step: f32) -> (Vec<Vector3>, Vec<[u32; 3]>, Vec<u8>) {
        let step = step.max(0.1);
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        let mut areas = Vec::new();
        for (ti, t) in self.tiles.iter().enumerate() {
            let Some(t) = t else { continue };
            for (pi, p) in t.polys.iter().enumerate() {
                let r = PolyRef {
                    tile: ti as u32,
                    poly: pi as u32,
                    salt: t.salt,
                };
                let pts: Vec<Vector3> = p.verts.iter().map(|&v| t.verts[v as usize]).collect();
                for k in 1..pts.len() - 1 {
                    let (a, b, c) = (pts[0], pts[k], pts[k + 1]);
                    let longest = a.distance(b).max(b.distance(c)).max(c.distance(a));
                    let n = ((longest / step).ceil() as usize).clamp(1, 64);
                    // Barycentric grid: row i has n - i + 1 points.
                    let base = verts.len() as u32;
                    let mut row_start = Vec::with_capacity(n + 1);
                    for i in 0..=n {
                        row_start.push(verts.len() as u32 - base);
                        for j in 0..=n - i {
                            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
                            let q = a + (b - a) * u + (c - a) * v;
                            verts.push(Vector3::new(q.x, self.poly_height(r, q), q.z));
                        }
                    }
                    for i in 0..n {
                        for j in 0..n - i {
                            let v0 = base + row_start[i] + j as u32;
                            let v1 = base + row_start[i + 1] + j as u32;
                            tris.push([v0, v1, v0 + 1]);
                            areas.push(p.area);
                            if j + 1 < n - i {
                                tris.push([v0 + 1, v1, v1 + 1]);
                                areas.push(p.area);
                            }
                        }
                    }
                }
            }
        }
        (verts, tris, areas)
    }

    /// Every polygon edge as (a, b, is_boundary) for drawing, split into pieces of about
    /// `step` metres that follow the ground. Boundary edges have nothing walkable across.
    pub fn debug_edges(&self, step: f32) -> Vec<(Vector3, Vector3, bool)> {
        let step = step.max(0.1);
        let mut out = Vec::new();
        for (ti, t) in self.tiles.iter().enumerate() {
            let Some(t) = t else { continue };
            for (pi, p) in t.polys.iter().enumerate() {
                let r = PolyRef {
                    tile: ti as u32,
                    poly: pi as u32,
                    salt: t.salt,
                };
                let n = p.verts.len();
                for e in 0..n {
                    let a = t.verts[p.verts[e] as usize];
                    let b = t.verts[p.verts[(e + 1) % n] as usize];
                    let open = p.links.iter().any(|l| l.edge as usize == e);
                    // Shared inner edges appear twice; draw them once.
                    if open && (a.x, a.z) > (b.x, b.z) {
                        continue;
                    }
                    let k = ((a.distance(b) / step).ceil() as usize).clamp(1, 256);
                    let at = |i: usize| {
                        let q = a.lerp(b, i as f32 / k as f32);
                        Vector3::new(q.x, self.poly_height(r, q), q.z)
                    };
                    for i in 0..k {
                        out.push((at(i), at(i + 1), !open));
                    }
                }
            }
        }
        out
    }

    /// Registered off-mesh links.
    pub fn links(&self) -> impl Iterator<Item = (u32, &OffMeshLink)> {
        self.links.iter().map(|(k, v)| (*k, v))
    }

    // ---- Off-mesh links (NavMeshLink) ----

    /// Unity `NavMesh.AddLink`. Returns the link id.
    pub fn add_link(&mut self, link: OffMeshLink) -> u32 {
        self.next_link += 1;
        let id = self.next_link;
        self.links.insert(id, link);
        self.reconnect_links();
        self.version += 1;
        id
    }

    pub fn remove_link(&mut self, id: u32) {
        if self.links.remove(&id).is_some() {
            self.reconnect_links();
            self.version += 1;
        }
    }

    pub fn link(&self, id: u32) -> Option<&OffMeshLink> {
        self.links.get(&id)
    }

    pub fn set_link_activated(&mut self, id: u32, on: bool) {
        if let Some(l) = self.links.get_mut(&id)
            && l.activated != on
        {
            l.activated = on;
            self.version += 1;
        }
    }

    /// Re-attaches every link to the polygons under its ends.
    fn reconnect_links(&mut self) {
        for t in self.tiles.iter_mut().flatten() {
            for p in &mut t.polys {
                p.offmesh.clear();
            }
        }
        let ids: Vec<u32> = self.links.keys().copied().collect();
        for id in ids {
            let l = &self.links[&id];
            let (at, s, e, w) = (l.agent_type, l.start, l.end, l.width);
            let filter = crate::QueryFilter::new(at);
            let ext = Vector3::new(0.5 + w * 0.5, 1.0, 0.5 + w * 0.5);
            let sp = self.find_nearest_poly(s, ext, &filter).map(|(r, _)| r);
            let ep = self.find_nearest_poly(e, ext, &filter).map(|(r, _)| r);
            let bidir = l.bidirectional;
            if let Some(l) = self.links.get_mut(&id) {
                l.start_poly = sp;
                l.end_poly = ep;
            }
            if let (Some(sp), Some(ep)) = (sp, ep) {
                for (r, add) in [(sp, true), (ep, bidir)] {
                    if !add {
                        continue;
                    }
                    if let Some(t) = self.tiles[r.tile as usize].as_mut()
                        && let Some(p) = t.polys.get_mut(r.poly as usize)
                        && !p.offmesh.contains(&id)
                    {
                        p.offmesh.push(id);
                    }
                }
            }
        }
    }

    /// Ground height of polygon `r` under `p`.
    pub(crate) fn poly_height(&self, r: PolyRef, p: Vector3) -> f32 {
        let Some((t, poly)) = self.poly(r) else {
            return p.y;
        };
        let pts: Vec<Vector3> = poly.verts.iter().map(|&v| t.verts[v as usize]).collect();
        let guess = crate::query::interpolate_height(&pts, p).unwrap_or(poly.center.y);
        let Some(inst) = self.instances.get(&t.instance) else {
            return guess;
        };
        let local = inst.to_local(Vector3::new(p.x, guess, p.z));
        // Large polygons over hills can be metres off the ground, so widen the search before
        // falling back to the polygon's plane (the closest floor to the guess still wins).
        let tol = inst.data.settings.agent_climb.max(0.25) + 0.5;
        let h = t.source.heights.height(local, local.y, tol).or_else(|| {
            let wide = inst.data.settings.agent_height.max(2.0) * 4.0;
            t.source.heights.height(local, local.y, wide)
        });
        match h {
            Some(h) => inst.to_world(Vector3::new(local.x, h, local.z)).y,
            None => guess,
        }
    }
}

fn hash_cell(p: Vector3) -> (i32, i32) {
    (
        (p.x / HASH_CELL).floor() as i32,
        (p.z / HASH_CELL).floor() as i32,
    )
}
