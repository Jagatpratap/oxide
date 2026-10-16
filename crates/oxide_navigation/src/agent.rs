//! NavMeshAgent simulation (Unity's crowd): path requests, corner following with braking,
//! off-mesh link traversal, velocity-sampling obstacle avoidance with priorities, and
//! separation, all constrained to the NavMesh surface.

use crate::components::{
    LinkTraversal, NavMeshAgent, NavMeshObstacle, NavMeshObstacleShape, ObstacleAvoidanceType,
};
use crate::navmesh::{NavMesh, PolyRef};
use crate::query::{NavMeshPathStatus, PathNode, QueryFilter};
use oxide_math::{Quaternion, Vector3};
use oxide_scene::{ComponentId, GameObjectId, World};

/// How far ahead (s) avoidance looks for collisions.
const HORIZON: f32 = 2.5;

struct Sim {
    id: GameObjectId,
    cid: ComponentId,
    pos: Vector3,
    vel: Vector3,
    desired: Vector3,
    radius: f32,
    speed: f32,
    accel: f32,
    priority: i32,
    quality: ObstacleAvoidanceType,
    filter: QueryFilter,
    /// Crossing a link or not on the NavMesh: not steered or avoided.
    passive: bool,
}

struct Obstacle {
    pos: Vector3,
    vel: Vector3,
    radius: f32,
}

fn xz(v: Vector3) -> Vector3 {
    Vector3::new(v.x, 0.0, v.z)
}

fn filter_of(a: &NavMeshAgent, nav: &NavMesh) -> QueryFilter {
    let costs = a.state.area_costs.map(|c| {
        let mut out = nav.area_costs;
        for (o, v) in out.iter_mut().zip(c) {
            if !v.is_nan() {
                *o = v;
            }
        }
        out
    });
    QueryFilter {
        agent_type: a.agent_type_id,
        area_mask: a.area_mask,
        costs,
    }
}

/// Puts the agent on the NavMesh near `p`. False if there is no NavMesh close enough.
fn locate(a: &mut NavMeshAgent, nav: &NavMesh, p: Vector3, filter: &QueryFilter) -> bool {
    let ext = Vector3::new(
        a.radius * 2.0 + 0.5,
        a.height.max(1.0),
        a.radius * 2.0 + 0.5,
    );
    match nav.find_nearest_poly(p, ext, filter) {
        Some((r, q)) => {
            let st = &mut a.state;
            st.poly = Some(r);
            st.pos = q;
            st.on_navmesh = true;
            if st.destination.is_some() {
                st.repath = true;
            }
            st.corridor.clear();
            true
        }
        None => {
            a.state.on_navmesh = false;
            a.state.poly = None;
            false
        }
    }
}

/// Detour's `mergeCorridorStartMoved`: re-roots the corridor at the last visited polygon.
fn merge_corridor(corridor: &mut Vec<PathNode>, visited: &[PolyRef]) {
    if visited.is_empty() {
        return;
    }
    let mut found = None;
    'outer: for i in (0..corridor.len()).rev() {
        for j in (0..visited.len()).rev() {
            if corridor[i] == PathNode::Poly(visited[j]) {
                found = Some((i, j));
                break 'outer;
            }
        }
    }
    let Some((fp, fv)) = found else {
        return;
    };
    let mut out: Vec<PathNode> = visited[fv..]
        .iter()
        .rev()
        .map(|&r| PathNode::Poly(r))
        .collect();
    out.extend_from_slice(&corridor[fp + 1..]);
    *corridor = out;
}

/// Moves the agent along the surface by `delta`, keeping its corridor rooted under it.
fn surface_move(a: &mut NavMeshAgent, nav: &NavMesh, delta: Vector3, filter: &QueryFilter) {
    let Some(poly) = a.state.poly else { return };
    let to = a.state.pos + xz(delta);
    let (p, visited) = nav.move_along_surface(poly, a.state.pos, to, filter);
    a.state.pos = p;
    if let Some(&last) = visited.last() {
        a.state.poly = Some(last);
    }
    merge_corridor(&mut a.state.corridor, &visited);
}

pub(crate) fn update_agents(world: &mut World, nav: &NavMesh, dt: f32) {
    let ids = world.query_ids::<NavMeshAgent>();
    if ids.is_empty() {
        return;
    }
    let mut sims = Vec::with_capacity(ids.len());
    for (id, cid) in ids {
        let tpos = world.position(id);
        let Some(a) = world.component_mut::<NavMeshAgent>(id, cid) else {
            continue;
        };
        let filter = filter_of(a, nav);
        let ground = tpos - Vector3::Y * a.base_offset;
        let st = &a.state;
        let invalid = st.poly.is_none_or(|r| !nav.is_valid_poly(r));
        let moved_by_script = a.update_position
            && st.link.is_none()
            && st.written.is_some_and(|w| w.distance(tpos) > 1e-3);
        if let Some(w) = a.state.warp.take() {
            a.state.velocity = Vector3::ZERO;
            a.state.link = None;
            locate(a, nav, w, &filter);
        } else if (invalid || moved_by_script) && a.state.link.is_none() {
            let at = if invalid && a.state.on_navmesh && !moved_by_script {
                a.state.pos
            } else {
                ground
            };
            locate(a, nav, at, &filter);
        }
        if a.state.moves != Vector3::ZERO {
            let m = std::mem::take(&mut a.state.moves);
            surface_move(a, nav, m, &filter);
        }
        if a.state.navmesh_version != nav.version {
            a.state.navmesh_version = nav.version;
            let broken = a.state.corridor.iter().any(|n| match n {
                PathNode::Poly(r) => !nav.is_valid_poly(*r),
                PathNode::Link { id, .. } => nav.link(*id).is_none_or(|l| !l.activated),
            });
            if broken && a.auto_repath && a.state.destination.is_some() {
                a.state.repath = true;
            }
        }
        plan(a, nav, &filter);
        let passive = a.state.link.is_some() || !a.state.on_navmesh;
        if !passive {
            steer(a, nav, dt);
        } else {
            a.state.desired_velocity = Vector3::ZERO;
        }
        sims.push(Sim {
            id,
            cid,
            pos: a.state.pos,
            vel: a.state.velocity,
            desired: a.state.desired_velocity,
            radius: a.radius,
            speed: a.speed,
            accel: a.acceleration,
            priority: a.avoidance_priority,
            quality: a.obstacle_avoidance_type,
            filter,
            passive,
        });
    }
    let obstacles: Vec<Obstacle> = world
        .query::<NavMeshObstacle>()
        .filter(|(_, o)| !(o.carve && o.runtime.carved_at.is_some()))
        .map(|(id, o)| {
            let s = world.lossy_scale(id);
            let radius = match o.shape {
                NavMeshObstacleShape::Capsule => o.radius * s.x.abs().max(s.z.abs()),
                NavMeshObstacleShape::Box => {
                    0.5 * (o.size.x * s.x).abs().max((o.size.z * s.z).abs())
                }
            };
            Obstacle {
                pos: world.transform_point(id, o.center),
                vel: o.runtime.velocity,
                radius,
            }
        })
        .collect();
    // Avoidance, then acceleration-limited velocity change.
    let chosen: Vec<Vector3> = (0..sims.len())
        .map(|i| avoid(&sims, i, &obstacles))
        .collect();
    for (s, c) in sims.iter_mut().zip(chosen) {
        if s.passive {
            continue;
        }
        let dv = c - s.vel;
        let max = s.accel * dt;
        s.vel += if dv.length() > max {
            dv.normalize() * max
        } else {
            dv
        };
        s.vel = xz(s.vel);
    }
    // Separation: push overlapping agents apart (the less important one moves more).
    let mut push = vec![Vector3::ZERO; sims.len()];
    for i in 0..sims.len() {
        for j in i + 1..sims.len() {
            let (a, b) = (&sims[i], &sims[j]);
            if a.passive || b.passive || a.filter.agent_type != b.filter.agent_type {
                continue;
            }
            let d = xz(b.pos - a.pos);
            let dist = d.length();
            let r = a.radius + b.radius;
            if dist >= r || (a.pos.y - b.pos.y).abs() > 2.0 {
                continue;
            }
            let n = if dist > 1e-4 { d / dist } else { Vector3::X };
            let overlap = (r - dist) * 0.5;
            let (wa, wb) = match a.priority.cmp(&b.priority) {
                std::cmp::Ordering::Less => (0.0, 2.0),
                std::cmp::Ordering::Greater => (2.0, 0.0),
                std::cmp::Ordering::Equal => (1.0, 1.0),
            };
            push[i] -= n * overlap * wa;
            push[j] += n * overlap * wb;
        }
    }
    for (k, s) in sims.into_iter().enumerate() {
        let Some(a) = world.component_mut::<NavMeshAgent>(s.id, s.cid) else {
            continue;
        };
        if s.passive {
            traverse_link(a, nav, dt, &s.filter);
        } else {
            a.state.velocity = s.vel;
            let delta = s.vel * dt + push[k] * 0.5;
            if delta.length_squared() > 1e-12 {
                surface_move(a, nav, delta, &s.filter);
            }
        }
        write_transform(world, s.id, s.cid, dt);
    }
}

/// Handles path requests (SetDestination, SetPath, replans).
fn plan(a: &mut NavMeshAgent, nav: &NavMesh, filter: &QueryFilter) {
    let st = &mut a.state;
    if let Some(p) = st.set_path.take() {
        st.corridor = p.nodes.clone();
        st.status = p.status;
        st.target = p.corners.last().copied().unwrap_or(st.pos);
        st.target_poly = match p.nodes.last() {
            Some(PathNode::Poly(r)) => Some(*r),
            _ => None,
        };
        st.path_pending = false;
        st.repath = false;
        return;
    }
    if !st.repath {
        return;
    }
    st.repath = false;
    st.path_pending = false;
    let (Some(dest), Some(start)) = (st.destination, st.poly) else {
        return;
    };
    let ext = Vector3::new(5.0, 10.0, 5.0);
    let Some((tr, tp)) = nav.find_nearest_poly(dest, ext, filter) else {
        st.status = NavMeshPathStatus::PathInvalid;
        st.corridor.clear();
        st.corners.clear();
        return;
    };
    let path = nav.path_between(start, st.pos, tr, tp, filter);
    st.status = path.status;
    st.target = path.corners.last().copied().unwrap_or(tp);
    st.target_poly = Some(tr);
    st.corridor = path.nodes;
    st.remaining = path.corners.windows(2).map(|w| w[0].distance(w[1])).sum();
}

/// Corners, remaining distance and desired velocity; starts link traversal at a link.
fn steer(a: &mut NavMeshAgent, nav: &NavMesh, dt: f32) {
    let (speed, accel, stopping) = (
        a.speed.max(0.0),
        a.acceleration.max(0.01),
        a.stopping_distance,
    );
    let auto_traverse = a.auto_traverse_off_mesh_link;
    let braking = a.auto_braking;
    let st = &mut a.state;
    if st.corridor.is_empty() || st.destination.is_none() {
        st.corners.clear();
        st.desired_velocity = Vector3::ZERO;
        return;
    }
    st.corners = nav.straight_path(st.pos, st.target, &st.corridor, 64);
    let corners = &st.corners;
    st.remaining = corners
        .windows(2)
        .map(|w| xz(w[1].pos - w[0].pos).length())
        .sum();
    if st.stopped {
        st.desired_velocity = Vector3::ZERO;
        return;
    }
    // The next corner that is not where we stand.
    let mut k = 1;
    while k + 1 < corners.len()
        && xz(corners[k].pos - st.pos).length() < 0.01
        && corners[k].link.is_none()
    {
        k += 1;
    }
    let Some(next) = corners.get(k).copied() else {
        st.desired_velocity = Vector3::ZERO;
        return;
    };
    // At a link's start: cross it.
    let at_link = corners[..=k.min(corners.len() - 1)]
        .iter()
        .find_map(|c| c.link.map(|l| (c.pos, l)));
    if let Some((lpos, (id, fwd))) = at_link
        && xz(lpos - st.pos).length() <= (speed * dt).max(0.1)
        && let Some(l) = nav.link(id)
    {
        let e = if fwd { l.end } else { l.start };
        st.link = Some(LinkTraversal {
            id,
            forward: fwd,
            start: st.pos,
            end: e,
            t: 0.0,
            moving: auto_traverse,
        });
        st.desired_velocity = Vector3::ZERO;
        return;
    }
    let to = xz(next.pos - st.pos);
    let dist = to.length();
    if dist < 1e-5 {
        st.desired_velocity = Vector3::ZERO;
        return;
    }
    let ends_here = corners.iter().all(|c| c.link.is_none());
    let mut v = speed;
    if ends_here {
        let left = st.remaining - stopping;
        if left <= 0.0 {
            v = 0.0;
        } else if braking {
            v = v.min((2.0 * accel * left).sqrt());
        }
    }
    st.desired_velocity = to / dist * v;
}

fn traverse_link(a: &mut NavMeshAgent, nav: &NavMesh, dt: f32, filter: &QueryFilter) {
    let speed = a.speed.max(0.1);
    let Some(mut l) = a.state.link else {
        a.state.velocity = Vector3::ZERO;
        return;
    };
    if !l.moving {
        a.state.velocity = Vector3::ZERO;
        return;
    }
    let len = l.start.distance(l.end).max(0.01);
    l.t = (l.t + dt * speed / len).min(1.0);
    a.state.pos = l.start.lerp(l.end, l.t);
    a.state.velocity = (l.end - l.start) / len * speed;
    if l.t >= 1.0 {
        a.state.link = None;
        // Drop the corridor up to and including the link.
        if let Some(i) = a
            .state
            .corridor
            .iter()
            .position(|n| matches!(n, PathNode::Link { id, .. } if *id == l.id))
        {
            a.state.corridor.drain(..=i);
        }
        let ext = Vector3::new(1.0, 2.0, 1.0);
        match nav.find_nearest_poly(l.end, ext, filter) {
            Some((r, q)) => {
                a.state.poly = Some(r);
                a.state.pos = q;
                if a.state.corridor.first() != Some(&PathNode::Poly(r)) {
                    a.state.repath = a.state.destination.is_some();
                }
            }
            None => a.state.on_navmesh = false,
        }
    } else {
        a.state.link = Some(l);
    }
}

fn write_transform(world: &mut World, id: GameObjectId, cid: ComponentId, dt: f32) {
    let Some(a) = world.component::<NavMeshAgent>(id, cid) else {
        return;
    };
    let waiting_on_link = a.state.link.is_some_and(|l| !l.moving);
    if !a.state.on_navmesh || waiting_on_link {
        if let Some(a) = world.component_mut::<NavMeshAgent>(id, cid) {
            a.state.written = None;
        }
        return;
    }
    let (update_pos, update_rot) = (a.update_position, a.update_rotation);
    let p = a.state.pos + Vector3::Y * a.base_offset;
    let v = xz(a.state.velocity);
    let ang = a.angular_speed.max(0.0).to_radians() * dt;
    if update_pos {
        world.set_position(id, p);
    }
    if update_rot && v.length() > 0.1 {
        let rot = world.rotation(id);
        let f = rot * Vector3::Z;
        let cur = f.x.atan2(f.z);
        let want = v.x.atan2(v.z);
        let mut d = want - cur;
        while d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        while d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        let yaw = cur + d.clamp(-ang, ang);
        world.set_rotation(id, Quaternion::from_rotation_y(yaw));
    }
    let written = update_pos.then(|| world.position(id));
    if let Some(a) = world.component_mut::<NavMeshAgent>(id, cid) {
        a.state.written = written;
    }
}

/// Time until a point moving at `v` (relative) touches a circle of radius `r` at `rel`.
fn time_to_hit(rel: Vector3, v: Vector3, r: f32) -> f32 {
    let c = rel.x * rel.x + rel.z * rel.z - r * r;
    let b = v.x * rel.x + v.z * rel.z;
    if c < 0.0 {
        // Already overlapping: only moving apart is free.
        return if b > 0.0 { 0.0 } else { HORIZON };
    }
    let a = v.x * v.x + v.z * v.z;
    if a < 1e-9 {
        return HORIZON;
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return HORIZON;
    }
    let t = (b - disc.sqrt()) / a;
    if t < 0.0 { HORIZON } else { t.min(HORIZON) }
}

/// Picks the velocity closest to the desired one that avoids collisions (sampled like
/// Detour's obstacle avoidance; more samples at higher quality).
fn avoid(sims: &[Sim], i: usize, obstacles: &[Obstacle]) -> Vector3 {
    let me = &sims[i];
    if me.passive {
        return me.vel;
    }
    let (rings, dirs) = match me.quality {
        ObstacleAvoidanceType::NoObstacleAvoidance => return me.desired,
        ObstacleAvoidanceType::LowQualityObstacleAvoidance => (1, 6),
        ObstacleAvoidanceType::MedQualityObstacleAvoidance => (2, 8),
        ObstacleAvoidanceType::GoodQualityObstacleAvoidance => (2, 12),
        ObstacleAvoidanceType::HighQualityObstacleAvoidance => (3, 16),
    };
    let range = me.radius + me.speed * HORIZON + 1.0;
    // (relative position, other velocity, combined radius, reciprocal)
    let mut others: Vec<(Vector3, Vector3, f32, bool)> = Vec::new();
    for (j, o) in sims.iter().enumerate() {
        if j == i || o.filter.agent_type != me.filter.agent_type || o.priority > me.priority {
            continue;
        }
        let rel = xz(o.pos - me.pos);
        if rel.length() > range + o.radius || (o.pos.y - me.pos.y).abs() > 2.0 {
            continue;
        }
        others.push((
            rel,
            o.vel,
            me.radius + o.radius,
            o.priority == me.priority && !o.passive,
        ));
    }
    for o in obstacles {
        let rel = xz(o.pos - me.pos);
        if rel.length() > range + o.radius || (o.pos.y - me.pos.y).abs() > 3.0 {
            continue;
        }
        others.push((rel, o.vel, me.radius + o.radius, false));
    }
    if others.is_empty() {
        return me.desired;
    }
    let vmax = me.speed.max(0.01);
    let score = |vc: Vector3| -> f32 {
        let mut tmin = HORIZON;
        let mut side = 0.0;
        for &(rel, ov, r, recip) in &others {
            let vab = if recip {
                vc * 2.0 - me.vel - ov
            } else {
                vc - ov
            };
            let t = time_to_hit(rel, vab, r);
            if t < tmin {
                tmin = t;
            }
            // Prefer passing on the right of things ahead (consistent sidestepping).
            let dist = rel.length().max(1e-3);
            let d = rel / dist;
            let cross = d.x * vc.z - d.z * vc.x;
            if cross > 0.0 && d.dot(vc) > 0.0 {
                side += cross / vmax / others.len() as f32;
            }
        }
        let vpen = (vc - me.desired).length() / vmax;
        let vcpen = (vc - me.vel).length() / vmax;
        let tpen = 1.0 / (0.1 + tmin / HORIZON);
        2.0 * vpen + 0.75 * vcpen + 0.75 * side + 2.5 * tpen
    };
    let mut best = (score(me.desired), me.desired);
    let base = if me.desired.length() > 1e-3 {
        me.desired.z.atan2(me.desired.x)
    } else if me.vel.length() > 1e-3 {
        me.vel.z.atan2(me.vel.x)
    } else {
        0.0
    };
    let zero = score(Vector3::ZERO);
    if zero < best.0 {
        best = (zero, Vector3::ZERO);
    }
    for ring in 1..=rings {
        let mag = vmax * ring as f32 / rings as f32;
        for k in 0..dirs {
            let a = base + k as f32 / dirs as f32 * std::f32::consts::TAU + ring as f32 * 0.2;
            let vc = Vector3::new(a.cos() * mag, 0.0, a.sin() * mag);
            let s = score(vc);
            if s < best.0 {
                best = (s, vc);
            }
        }
    }
    best.1
}
