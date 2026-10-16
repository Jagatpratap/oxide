//! Scene ⇄ Rapier synchronization and Unity's physics messages, run every fixed step.
//!
//! Before the step: Rigidbodies/CharacterControllers/colliders/joints in the scene are
//! created, updated or removed in Rapier (shapes are rebuilt only when their parameters,
//! scale or layer change); transforms moved by scripts teleport their bodies; queued forces
//! are applied. After the step: dynamic bodies write their pose back (optionally
//! interpolated each frame), and collision/trigger Enter/Stay/Exit messages are delivered.

use crate::components::*;
use crate::physics::{BodyEntry, BodyKind, ColliderEntry, Physics, pose};
use oxide_assets::{AssetServer, Handle};
use oxide_core::Time;
use oxide_math::{Color, Quaternion, Vector3};
use oxide_scene::{
    Collision, ComponentId, ContactPoint, DebugDraw, GameObjectId, MeshFilter, World,
};
use rapier3d::prelude::*;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

const EPS: f32 = 1e-4;

fn moved(a: (Vector3, Quaternion), b: (Vector3, Quaternion)) -> bool {
    a.0.distance_squared(b.0) > EPS * EPS || a.1.dot(b.1).abs() < 1.0 - 1e-6
}

fn groups(phys: &Physics, layer: u8) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_truncate(1 << (layer & 31)),
        Group::from_bits_truncate(phys.layer_collision_matrix[(layer & 31) as usize]),
        InteractionTestMode::And,
    )
}

fn ensure_body(
    phys: &mut Physics,
    id: GameObjectId,
    kind: BodyKind,
    p: Vector3,
    r: Quaternion,
) -> RigidBodyHandle {
    let rtype = match kind {
        BodyKind::Dynamic => RigidBodyType::Dynamic,
        BodyKind::Kinematic | BodyKind::Character => RigidBodyType::KinematicPositionBased,
        BodyKind::Static => RigidBodyType::Fixed,
    };
    if let Some(e) = phys.bodies.get_mut(&id) {
        if e.kind != kind {
            e.kind = kind;
            if let Some(b) = phys.world.bodies.get_mut(e.handle) {
                b.set_body_type(rtype, true);
            }
        }
        return e.handle;
    }
    let body = RigidBodyBuilder::new(rtype)
        .pose(pose(p, r))
        .user_data(id.to_bits() as u128)
        .build();
    let handle = phys.world.insert_body(body);
    phys.bodies.insert(
        id,
        BodyEntry {
            handle,
            kind,
            synced: (p, r),
            prev: (p, r),
            curr: (p, r),
            interpolate: false,
            mass: 1.0,
            com: Vector3::ZERO,
        },
    );
    handle
}

fn sync_bodies(world: &mut World, phys: &mut Physics, seen: &mut HashSet<GameObjectId>, dt: f32) {
    for (id, cid) in world.query_ids::<Rigidbody>() {
        let (p, r) = (world.position(id), world.rotation(id));
        let Some(rb) = world.component_mut::<Rigidbody>(id, cid) else {
            continue;
        };
        let cmds = std::mem::take(&mut rb.commands);
        let rb = rb.clone();
        let kind = if rb.is_kinematic {
            BodyKind::Kinematic
        } else {
            BodyKind::Dynamic
        };
        let handle = ensure_body(phys, id, kind, p, r);
        seen.insert(id);
        let entry = phys.bodies.get_mut(&id).expect("ensured");
        entry.interpolate =
            rb.interpolation == RigidbodyInterpolation::Interpolate && kind == BodyKind::Dynamic;
        entry.mass = rb.mass.max(1e-4);
        entry.com = rb.center_of_mass;
        let teleport = moved((p, r), entry.synced);
        if teleport {
            entry.synced = (p, r);
            entry.prev = (p, r);
            entry.curr = (p, r);
        }
        let Some(body) = phys.world.bodies.get_mut(handle) else {
            continue;
        };
        body.set_linear_damping(rb.linear_damping);
        body.set_angular_damping(rb.angular_damping);
        body.set_gravity_scale(if rb.use_gravity { 1.0 } else { 0.0 }, false);
        body.set_enabled_translations(
            !rb.freeze_position_x,
            !rb.freeze_position_y,
            !rb.freeze_position_z,
            false,
        );
        body.set_enabled_rotations(
            !rb.freeze_rotation_x,
            !rb.freeze_rotation_y,
            !rb.freeze_rotation_z,
            false,
        );
        body.enable_ccd(rb.collision_detection == CollisionDetectionMode::Continuous);
        match kind {
            BodyKind::Kinematic => body.set_next_kinematic_position(pose(p, r)),
            _ if teleport => body.set_position(pose(p, r), true),
            _ => {}
        }
        // Unity forces last one step; Rapier's persist until reset.
        body.reset_forces(false);
        body.reset_torques(false);
        let m = rb.mass.max(1e-4);
        for c in cmds {
            match c {
                RbCommand::Force(f, mode) => match mode {
                    ForceMode::Force => body.add_force(f, true),
                    ForceMode::Acceleration => body.add_force(f * m, true),
                    ForceMode::Impulse => body.apply_impulse(f, true),
                    ForceMode::VelocityChange => body.apply_impulse(f * m, true),
                },
                RbCommand::ForceAt(f, at, mode) => match mode {
                    ForceMode::Force => body.add_force_at_point(f, at, true),
                    ForceMode::Acceleration => body.add_force_at_point(f * m, at, true),
                    ForceMode::Impulse => body.apply_impulse_at_point(f, at, true),
                    ForceMode::VelocityChange => body.apply_impulse_at_point(f * m, at, true),
                },
                RbCommand::Torque(t, mode) => match mode {
                    ForceMode::Force => body.add_torque(t, true),
                    ForceMode::Acceleration => body.add_torque(t * m, true),
                    ForceMode::Impulse => body.apply_torque_impulse(t, true),
                    ForceMode::VelocityChange => body.apply_torque_impulse(t * m, true),
                },
                RbCommand::SetVelocity(v) => body.set_linvel(v, true),
                RbCommand::SetAngularVelocity(v) => body.set_angvel(v, true),
                RbCommand::MovePosition(np) => {
                    if kind == BodyKind::Kinematic {
                        let rot = *body.rotation();
                        body.set_next_kinematic_position(pose(np, rot));
                    } else {
                        let rot = *body.rotation();
                        body.set_position(pose(np, rot), true);
                    }
                }
                RbCommand::MoveRotation(q) => {
                    let t = body.translation();
                    if kind == BodyKind::Kinematic {
                        body.set_next_kinematic_position(pose(t, q));
                    } else {
                        body.set_position(pose(t, q), true);
                    }
                }
                RbCommand::Sleep => body.sleep(),
                RbCommand::WakeUp => body.wake_up(true),
            }
        }
        let _ = dt;
    }

    for (id, _) in world.query_ids::<CharacterController>() {
        if seen.contains(&id) {
            continue; // a Rigidbody wins
        }
        let (p, r) = (world.position(id), world.rotation(id));
        let handle = ensure_body(phys, id, BodyKind::Character, p, r);
        seen.insert(id);
        let e = phys.bodies.get_mut(&id).expect("ensured");
        e.synced = (p, r);
        if let Some(b) = phys.world.bodies.get_mut(handle) {
            b.set_next_kinematic_position(pose(p, r));
        }
    }
}

struct Desc {
    shape: SharedShape,
    offset: Vector3,
    trigger: bool,
    friction: f32,
    bounciness: f32,
    combine: (PhysicsMaterialCombine, PhysicsMaterialCombine),
    sig: u64,
}

fn combine_rule(c: PhysicsMaterialCombine) -> CoefficientCombineRule {
    match c {
        PhysicsMaterialCombine::Average => CoefficientCombineRule::Average,
        PhysicsMaterialCombine::Minimum => CoefficientCombineRule::Min,
        PhysicsMaterialCombine::Multiply => CoefficientCombineRule::Multiply,
        PhysicsMaterialCombine::Maximum => CoefficientCombineRule::Max,
    }
}

/// A collider's Physics Material, if it has one and it is loaded.
fn collider_material(
    world: &World,
    obj: GameObjectId,
    cid: ComponentId,
) -> Option<(Handle<PhysicsMaterial>, PhysicsMaterial)> {
    let h = if let Some(c) = world.component::<BoxCollider>(obj, cid) {
        c.material.clone()
    } else if let Some(c) = world.component::<SphereCollider>(obj, cid) {
        c.material.clone()
    } else if let Some(c) = world.component::<CapsuleCollider>(obj, cid) {
        c.material.clone()
    } else if let Some(c) = world.component::<MeshCollider>(obj, cid) {
        c.material.clone()
    } else if let Some(c) = world.component::<TerrainCollider>(obj, cid) {
        c.material.clone()
    } else {
        None
    }?;
    let m = world.resource::<AssetServer>().data.get(&h)?.clone();
    Some((h, m))
}

fn hash_f(h: &mut DefaultHasher, v: &[f32]) {
    for x in v {
        x.to_bits().hash(h);
    }
}

fn mesh_shape(
    world: &World,
    obj: GameObjectId,
    mc: &MeshCollider,
    scale: Vector3,
    h: &mut DefaultHasher,
) -> Option<SharedShape> {
    let handle = mc.mesh.clone().or_else(|| {
        world
            .get_component::<MeshFilter>(obj)
            .and_then(|f| f.mesh.clone())
    })?;
    let assets = world.resource::<AssetServer>();
    let mesh = assets.meshes.get(&handle)?;
    handle.id().hash(h);
    assets.meshes.version(handle.id()).hash(h);
    mc.convex.hash(h);
    let verts: Vec<Vector> = mesh.vertices.iter().map(|v| *v * scale).collect();
    if mc.convex {
        SharedShape::convex_hull(&verts)
    } else {
        let idx: Vec<[u32; 3]> = mesh.triangles().collect();
        SharedShape::trimesh(verts, idx).ok()
    }
}

/// Shape, offset and settings for one collider component, in the collider object's
/// scaled local space.
fn describe(world: &World, obj: GameObjectId, cid: ComponentId) -> Option<Desc> {
    let s = world.lossy_scale(obj).abs();
    let mut h = DefaultHasher::new();
    hash_f(&mut h, &s.to_array());
    let d = if let Some(c) = world.component::<BoxCollider>(obj, cid) {
        let half = (c.size * s * 0.5).max(Vector3::splat(1e-3));
        hash_f(
            &mut h,
            &[
                half.x,
                half.y,
                half.z,
                c.center.x,
                c.center.y,
                c.center.z,
                c.friction,
                c.bounciness,
            ],
        );
        Desc {
            shape: SharedShape::cuboid(half.x, half.y, half.z),
            offset: c.center * s,
            trigger: c.is_trigger,
            friction: c.friction,
            bounciness: c.bounciness,
            combine: Default::default(),
            sig: 0,
        }
    } else if let Some(c) = world.component::<SphereCollider>(obj, cid) {
        let r = (c.radius * s.max_element()).max(1e-3);
        hash_f(
            &mut h,
            &[
                r,
                c.center.x,
                c.center.y,
                c.center.z,
                c.friction,
                c.bounciness,
                1.0,
            ],
        );
        Desc {
            shape: SharedShape::ball(r),
            offset: c.center * s,
            trigger: c.is_trigger,
            friction: c.friction,
            bounciness: c.bounciness,
            combine: Default::default(),
            sig: 0,
        }
    } else if let Some(c) = world.component::<CapsuleCollider>(obj, cid) {
        let (axis_scale, radial) = match c.direction {
            CapsuleDirection::XAxis => (s.x, s.y.max(s.z)),
            CapsuleDirection::YAxis => (s.y, s.x.max(s.z)),
            CapsuleDirection::ZAxis => (s.z, s.x.max(s.y)),
        };
        let r = (c.radius * radial).max(1e-3);
        let half = (c.height * axis_scale * 0.5 - r).max(0.0);
        let shape = match c.direction {
            CapsuleDirection::XAxis => SharedShape::capsule_x(half, r),
            CapsuleDirection::YAxis => SharedShape::capsule_y(half, r),
            CapsuleDirection::ZAxis => SharedShape::capsule_z(half, r),
        };
        hash_f(
            &mut h,
            &[
                r,
                half,
                c.direction as u8 as f32,
                c.center.x,
                c.center.y,
                c.center.z,
                c.friction,
                c.bounciness,
                2.0,
            ],
        );
        Desc {
            shape,
            offset: c.center * s,
            trigger: c.is_trigger,
            friction: c.friction,
            bounciness: c.bounciness,
            combine: Default::default(),
            sig: 0,
        }
    } else if let Some(c) = world.component::<MeshCollider>(obj, cid) {
        hash_f(&mut h, &[c.friction, c.bounciness, 3.0]);
        let shape = mesh_shape(world, obj, c, s, &mut h)?;
        Desc {
            shape,
            offset: Vector3::ZERO,
            trigger: c.is_trigger,
            friction: c.friction,
            bounciness: c.bounciness,
            combine: Default::default(),
            sig: 0,
        }
    } else if let Some(c) = world.component::<TerrainCollider>(obj, cid) {
        let n = c.resolution.max(2) as usize;
        if c.heights.len() < n * n {
            return None;
        }
        hash_f(&mut h, &[c.size.x, c.size.y, c.size.z, c.friction, 4.0]);
        (c.version, n, std::sync::Arc::as_ptr(&c.heights) as usize).hash(&mut h);
        // Rapier wants a column-major (rows = z, cols = x) grid centered on the origin.
        let mut data = vec![0.0; n * n];
        for z in 0..n {
            for x in 0..n {
                data[z + x * n] = c.heights[z * n + x];
            }
        }
        let size = c.size * s;
        let mut hf = rapier3d::parry::shape::HeightField::new(
            rapier3d::parry::utils::Array2::new(n, n, data),
            Vector3::new(size.x, size.y, size.z),
        );
        // Terrain holes: removed cells (rows = z, columns = x).
        let cells = n - 1;
        if c.holes.len() == cells * cells {
            for z in 0..cells {
                for x in 0..cells {
                    if c.holes[z * cells + x] == 0 {
                        hf.set_cell_status(
                            z,
                            x,
                            rapier3d::parry::shape::HeightFieldCellStatus::CELL_REMOVED,
                        );
                    }
                }
            }
        }
        let shape = SharedShape::new(hf);
        Desc {
            shape,
            offset: Vector3::new(size.x * 0.5, 0.0, size.z * 0.5),
            trigger: false,
            friction: c.friction,
            bounciness: 0.0,
            combine: Default::default(),
            sig: 0,
        }
    } else {
        let c = world.component::<CharacterController>(obj, cid)?;
        let r = (c.radius * s.x.max(s.z)).max(1e-3);
        let half = (c.height * s.y * 0.5 - r).max(0.0);
        hash_f(&mut h, &[r, half, c.center.x, c.center.y, c.center.z, 5.0]);
        Desc {
            shape: SharedShape::capsule_y(half, r),
            offset: c.center * s,
            trigger: false,
            friction: 0.0,
            bounciness: 0.0,
            combine: Default::default(),
            sig: 0,
        }
    };
    // A Physics Material replaces the collider's own friction and bounciness.
    let mut d = d;
    if let Some((mh, m)) = collider_material(world, obj, cid) {
        mh.id().hash(&mut h);
        world
            .resource::<AssetServer>()
            .data
            .version(mh.path())
            .hash(&mut h);
        d.friction = m.friction();
        d.bounciness = m.bounciness;
        d.combine = (m.friction_combine, m.bounce_combine);
    }
    Some(Desc {
        sig: h.finish(),
        ..d
    })
}

fn sync_colliders(
    world: &mut World,
    phys: &mut Physics,
    seen_bodies: &mut HashSet<GameObjectId>,
) -> HashSet<GameObjectId> {
    let mut list: Vec<(GameObjectId, ComponentId)> = Vec::new();
    list.extend(world.query_ids::<BoxCollider>());
    list.extend(world.query_ids::<SphereCollider>());
    list.extend(world.query_ids::<CapsuleCollider>());
    list.extend(world.query_ids::<MeshCollider>());
    list.extend(world.query_ids::<TerrainCollider>());
    list.extend(world.query_ids::<CharacterController>());
    let mut seen = HashSet::new();
    let mut mass_dirty = HashSet::new();
    for (obj, cid) in list {
        let rb_obj = world.ancestor_with::<Rigidbody>(obj);
        let body_obj = match rb_obj {
            Some(b) => b,
            None if world.has_component::<CharacterController>(obj) => obj,
            None => {
                let (p, r) = (world.position(obj), world.rotation(obj));
                ensure_body(phys, obj, BodyKind::Static, p, r);
                seen_bodies.insert(obj);
                let e = phys.bodies.get_mut(&obj).expect("ensured");
                if moved((p, r), e.synced) {
                    e.synced = (p, r);
                    let h = e.handle;
                    if let Some(b) = phys.world.bodies.get_mut(h) {
                        b.set_position(pose(p, r), true);
                    }
                }
                obj
            }
        };
        // Character controllers only get their capsule (skip other colliders on them).
        if world.has_component::<CharacterController>(obj)
            && world.component::<CharacterController>(obj, cid).is_none()
            && rb_obj.is_none()
        {
            continue;
        }
        let Some(body) = phys.bodies.get(&body_obj).map(|b| b.handle) else {
            continue;
        };
        let Some(d) = describe(world, obj, cid) else {
            continue;
        };
        let layer = world.get(obj).map_or(0, |o| o.layer);
        let (bp, br) = (world.position(body_obj), world.rotation(body_obj));
        let world_center = world.position(obj) + world.rotation(obj) * d.offset;
        let rel_p = br.inverse() * (world_center - bp);
        let rel_r = (br.inverse() * world.rotation(obj)).normalize();
        let mut h = DefaultHasher::new();
        d.sig.hash(&mut h);
        hash_f(
            &mut h,
            &[
                rel_p.x, rel_p.y, rel_p.z, rel_r.x, rel_r.y, rel_r.z, rel_r.w,
            ],
        );
        (
            layer,
            d.trigger,
            body,
            phys.layer_collision_matrix[layer as usize & 31],
        )
            .hash(&mut h);
        let sig = h.finish();
        seen.insert(cid);
        if phys.colliders.get(&cid).is_some_and(|c| c.signature == sig) {
            continue;
        }
        if let Some(old) = phys.colliders.remove(&cid) {
            phys.world.remove_collider(old.handle);
            phys.owners.remove(&old.handle);
        }
        let collider = ColliderBuilder::new(d.shape)
            .position(pose(rel_p, rel_r))
            .sensor(d.trigger)
            .friction(d.friction)
            .restitution(d.bounciness)
            .friction_combine_rule(combine_rule(d.combine.0))
            .restitution_combine_rule(combine_rule(d.combine.1))
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .collision_groups(groups(phys, layer))
            .user_data(obj.to_bits() as u128 | ((layer as u128) << 64))
            .build();
        let handle = phys.world.insert_collider(collider, Some(body));
        phys.owners.insert(handle, (obj, rb_obj));
        phys.colliders.insert(
            cid,
            ColliderEntry {
                handle,
                body: body_obj,
                signature: sig,
            },
        );
        mass_dirty.insert(body_obj);
    }
    // Remove colliders whose component or object went away.
    let stale: Vec<ComponentId> = phys
        .colliders
        .keys()
        .filter(|k| !seen.contains(k))
        .copied()
        .collect();
    for k in stale {
        let e = phys.colliders.remove(&k).expect("listed");
        mass_dirty.insert(e.body);
        phys.world.remove_collider(e.handle);
        phys.owners.remove(&e.handle);
    }
    mass_dirty
}

/// Gives a dynamic body exactly the Rigidbody's mass (distributed over its colliders'
/// volumes for a plausible inertia) and the requested center of mass.
fn update_mass(phys: &mut Physics, id: GameObjectId) {
    let Some(e) = phys.bodies.get(&id) else {
        return;
    };
    if e.kind != BodyKind::Dynamic {
        return;
    }
    let (handle, mass, com) = (e.handle, e.mass, e.com);
    let Some(body) = phys.world.bodies.get(handle) else {
        return;
    };
    let cols: Vec<ColliderHandle> = body.colliders().to_vec();
    let mut props = MassProperties::default();
    let mut volume = 0.0;
    for &c in &cols {
        if let Some(col) = phys.world.colliders.get(c)
            && !col.is_sensor()
        {
            volume += col.shape().mass_properties(1.0).mass();
        }
    }
    if volume > 0.0 {
        let density = mass / volume;
        for &c in &cols {
            if let Some(col) = phys.world.colliders.get(c)
                && !col.is_sensor()
                && let Some(rel) = col.position_wrt_parent()
            {
                props += col.shape().mass_properties(density).transform_by(rel);
            }
        }
    } else {
        props = MassProperties::new(Vector3::ZERO, mass, Vector3::splat(mass * 0.4));
    }
    if com != Vector3::ZERO {
        props.local_com = com;
    }
    for &c in &cols {
        if let Some(col) = phys.world.colliders.get_mut(c) {
            col.set_density(0.0);
        }
    }
    if let Some(body) = phys.world.bodies.get_mut(handle) {
        body.set_additional_mass_properties(props, true);
    }
}

fn sync_joints(world: &mut World, phys: &mut Physics) {
    let mut seen = HashSet::new();
    let mut want: Vec<(
        ComponentId,
        GameObjectId,
        Option<GameObjectId>,
        GenericJoint,
        u64,
    )> = Vec::new();
    for (id, cid) in world.query_ids::<crate::components::FixedJoint>() {
        let j = world
            .component::<crate::components::FixedJoint>(id, cid)
            .expect("listed")
            .clone();
        let (p1, r1) = (world.position(id), world.rotation(id));
        let frame2 = match j.connected_body {
            Some(o) if world.is_alive(o) => {
                let (p2, r2) = (world.position(o), world.rotation(o));
                pose(r2.inverse() * (p1 - p2), r2.inverse() * r1)
            }
            _ => pose(p1, r1),
        };
        let joint = FixedJointBuilder::new()
            .local_frame1(Pose::IDENTITY)
            .local_frame2(frame2)
            .build();
        let mut h = DefaultHasher::new();
        (j.connected_body, 1u8).hash(&mut h);
        want.push((cid, id, j.connected_body, joint.into(), h.finish()));
    }
    for (id, cid) in world.query_ids::<crate::components::HingeJoint>() {
        let j = world
            .component::<crate::components::HingeJoint>(id, cid)
            .expect("listed")
            .clone();
        let s = world.lossy_scale(id);
        let anchor_world = world.transform_point(id, j.anchor);
        let axis = j.axis.normalize_or(Vector3::X);
        let anchor2 = match j.connected_body {
            Some(o) if world.is_alive(o) => {
                world.rotation(o).inverse() * (anchor_world - world.position(o))
            }
            _ => anchor_world,
        };
        let axis2 = match j.connected_body {
            Some(o) if world.is_alive(o) => {
                world.rotation(o).inverse() * (world.rotation(id) * axis)
            }
            _ => world.rotation(id) * axis,
        };
        let mut b = RevoluteJointBuilder::new(axis)
            .local_anchor1(j.anchor * s)
            .local_anchor2(anchor2);
        if j.use_limits {
            b = b.limits([j.min_angle.to_radians(), j.max_angle.to_radians()]);
        }
        if j.use_motor {
            b = b
                .motor_velocity(j.motor_target_velocity.to_radians(), 1.0)
                .motor_max_force(j.motor_force);
        }
        let mut joint: GenericJoint = b.build().into();
        joint.set_local_axis2(axis2);
        let mut h = DefaultHasher::new();
        (j.connected_body, 2u8, j.use_limits, j.use_motor).hash(&mut h);
        hash_f(
            &mut h,
            &[
                j.min_angle,
                j.max_angle,
                j.motor_target_velocity,
                j.motor_force,
                j.axis.x,
                j.axis.y,
                j.axis.z,
                j.anchor.x,
                j.anchor.y,
                j.anchor.z,
            ],
        );
        want.push((cid, id, j.connected_body, joint, h.finish()));
    }
    for (id, cid) in world.query_ids::<crate::components::CharacterJoint>() {
        let j = world
            .component::<crate::components::CharacterJoint>(id, cid)
            .expect("listed")
            .clone();
        // Joint frame: X = twist axis, Y = swing axis (orthonormalized).
        let x = j.axis.normalize_or(Vector3::X);
        let y0 = j.swing_axis - x * j.swing_axis.dot(x);
        let y = y0
            .try_normalize()
            .unwrap_or_else(|| x.any_orthonormal_vector());
        let z = x.cross(y);
        let basis = Quaternion::from_mat3(&oxide_math::glam::Mat3::from_cols(x, y, z));
        let s = world.lossy_scale(id);
        let anchor_world = world.transform_point(id, j.anchor);
        let rot_world = world.rotation(id) * basis;
        let frame2 = match j.connected_body {
            Some(o) if world.is_alive(o) => {
                let (p2, r2) = (world.position(o), world.rotation(o));
                pose(r2.inverse() * (anchor_world - p2), r2.inverse() * rot_world)
            }
            _ => pose(anchor_world, rot_world),
        };
        let joint = GenericJointBuilder::new(JointAxesMask::LOCKED_SPHERICAL_AXES)
            .local_frame1(pose(j.anchor * s, basis))
            .local_frame2(frame2)
            .limits(
                JointAxis::AngX,
                [
                    j.low_twist_limit.min(j.high_twist_limit).to_radians(),
                    j.high_twist_limit.max(j.low_twist_limit).to_radians(),
                ],
            )
            .limits(JointAxis::AngY, {
                let a = j.swing1_limit.abs().to_radians();
                [-a, a]
            })
            .limits(JointAxis::AngZ, {
                let a = j.swing2_limit.abs().to_radians();
                [-a, a]
            })
            .contacts_enabled(j.enable_collision)
            .build();
        let mut h = DefaultHasher::new();
        (j.connected_body, 3u8, j.enable_collision).hash(&mut h);
        hash_f(
            &mut h,
            &[
                j.low_twist_limit,
                j.high_twist_limit,
                j.swing1_limit,
                j.swing2_limit,
                j.axis.x,
                j.axis.y,
                j.axis.z,
                j.swing_axis.x,
                j.swing_axis.y,
                j.swing_axis.z,
                j.anchor.x,
                j.anchor.y,
                j.anchor.z,
            ],
        );
        want.push((cid, id, j.connected_body, joint, h.finish()));
    }
    for (cid, id, other, joint, sig) in want {
        let Some(b1) = phys.bodies.get(&id).map(|b| b.handle) else {
            continue;
        };
        let b2 = match other {
            Some(o) => match phys.bodies.get(&o) {
                Some(b) => b.handle,
                None => continue,
            },
            None => {
                // Joints to "the world" attach to a shared fixed body at the origin.
                let ground = GameObjectId::from_bits(u64::MAX);
                ensure_body(
                    phys,
                    ground,
                    BodyKind::Static,
                    Vector3::ZERO,
                    Quaternion::IDENTITY,
                )
            }
        };
        seen.insert(cid);
        if phys.joints.get(&cid).is_some_and(|(_, s)| *s == sig) {
            continue;
        }
        if let Some((h, _)) = phys.joints.remove(&cid) {
            phys.world.remove_impulse_joint(h);
        }
        let h = phys.world.insert_impulse_joint(b1, b2, joint);
        phys.joints.insert(cid, (h, sig));
    }
    let stale: Vec<ComponentId> = phys
        .joints
        .keys()
        .filter(|k| !seen.contains(k))
        .copied()
        .collect();
    for k in stale {
        if let Some((h, _)) = phys.joints.remove(&k) {
            phys.world.remove_impulse_joint(h);
        }
    }
}

fn remove_stale_bodies(phys: &mut Physics, seen: &HashSet<GameObjectId>) {
    let ground = GameObjectId::from_bits(u64::MAX);
    let stale: Vec<GameObjectId> = phys
        .bodies
        .keys()
        .filter(|k| !seen.contains(k) && **k != ground)
        .copied()
        .collect();
    for id in stale {
        let e = phys.bodies.remove(&id).expect("listed");
        phys.colliders.retain(|_, c| c.body != id);
        if let Some(body) = phys.world.bodies.get(e.handle) {
            for c in body.colliders().to_vec() {
                phys.owners.remove(&c);
            }
        }
        phys.world.remove_body_with_colliders(e.handle, true);
        phys.vehicles.remove(&id);
    }
}

/// Body ids, parents before children: a nested body (ragdoll bone) must be written after
/// the body it hangs from, or its world pose would be computed from a stale parent.
fn ids_top_down(world: &World, phys: &Physics) -> Vec<GameObjectId> {
    let depth = |mut id: GameObjectId| {
        let mut d = 0;
        while let Some(p) = world.parent(id) {
            d += 1;
            id = p;
        }
        d
    };
    let mut ids: Vec<(usize, GameObjectId)> =
        phys.bodies.keys().map(|&id| (depth(id), id)).collect();
    ids.sort_by_key(|(d, id)| (*d, id.to_bits()));
    ids.into_iter().map(|(_, id)| id).collect()
}

fn write_back(world: &mut World, phys: &mut Physics) {
    let ids = ids_top_down(world, phys);
    for id in ids {
        let e = phys.bodies.get_mut(&id).expect("listed");
        let Some(body) = phys.world.bodies.get(e.handle) else {
            continue;
        };
        let (lin, ang, sleeping) = (body.linvel(), body.angvel(), body.is_sleeping());
        if e.kind == BodyKind::Dynamic {
            let p = body.translation();
            let r = *body.rotation();
            e.prev = e.curr;
            e.curr = (p, r);
            if !e.interpolate {
                e.synced = (p, r);
                world.set_rotation(id, r);
                world.set_position(id, p);
            }
        }
        if let Some(rb) = world.get_component_mut::<Rigidbody>(id) {
            rb.velocity = lin;
            rb.angular_velocity = ang;
            rb.sleeping = sleeping;
        }
    }
}

enum Msg {
    CollisionEnter(Collision),
    CollisionStay(Collision),
    CollisionExit(Collision),
    TriggerEnter(GameObjectId),
    TriggerStay(GameObjectId),
    TriggerExit(GameObjectId),
}

fn collision_for(phys: &Physics, mine: ColliderHandle, other: ColliderHandle) -> Collision {
    let (o2, b2) = phys
        .owner(other)
        .unwrap_or((GameObjectId::from_bits(0), None));
    let mut contacts = Vec::new();
    let mut impulse = Vector3::ZERO;
    if let Some(pair) = phys.world.narrow_phase.contact_pair(mine, other) {
        let flip = pair.collider1 != mine;
        let c1 = phys.world.colliders.get(pair.collider1);
        for m in pair.manifolds() {
            let n = if flip { m.data.normal } else { -m.data.normal };
            for p in &m.points {
                let point = c1.map_or(p.local_p1, |c| c.position().transform_point(p.local_p1));
                contacts.push(ContactPoint {
                    point,
                    normal: n,
                    separation: p.dist,
                });
                impulse += n * p.data.impulse;
            }
        }
    }
    let vel = |h: ColliderHandle| {
        phys.world
            .colliders
            .get(h)
            .and_then(|c| c.parent())
            .and_then(|b| phys.world.bodies.get(b))
            .map_or(Vector3::ZERO, |b| b.linvel())
    };
    Collision {
        game_object: b2.unwrap_or(o2),
        contacts,
        relative_velocity: vel(other) - vel(mine),
        impulse,
    }
}

fn collect_events(phys: &mut Physics) -> Vec<(GameObjectId, Msg)> {
    let events: Vec<CollisionEvent> = phys
        .sink
        .events
        .lock()
        .map(|mut e| std::mem::take(&mut *e))
        .unwrap_or_default();
    let mut started = HashSet::new();
    let mut out = Vec::new();
    let key = |a: ColliderHandle, b: ColliderHandle| {
        if a.into_raw_parts() <= b.into_raw_parts() {
            (a, b)
        } else {
            (b, a)
        }
    };
    let push = |out: &mut Vec<(GameObjectId, Msg)>,
                phys: &Physics,
                h1: ColliderHandle,
                h2: ColliderHandle,
                kind: u8,
                sensor: bool| {
        for (mine, other) in [(h1, h2), (h2, h1)] {
            let Some((obj, body)) = phys.owner(mine) else {
                continue;
            };
            let Some((oobj, _)) = phys.owner(other) else {
                continue;
            };
            let mut targets = vec![obj];
            if let Some(b) = body
                && b != obj
            {
                targets.push(b);
            }
            for t in targets {
                let msg = if sensor {
                    match kind {
                        0 => Msg::TriggerEnter(oobj),
                        1 => Msg::TriggerStay(oobj),
                        _ => Msg::TriggerExit(oobj),
                    }
                } else {
                    let c = collision_for(phys, mine, other);
                    match kind {
                        0 => Msg::CollisionEnter(c),
                        1 => Msg::CollisionStay(c),
                        _ => Msg::CollisionExit(c),
                    }
                };
                out.push((t, msg));
            }
        }
    };
    for ev in events {
        let (a, b, start, flags) = match ev {
            CollisionEvent::Started(a, b, f) => (a, b, true, f),
            CollisionEvent::Stopped(a, b, f) => (a, b, false, f),
        };
        let sensor = flags.contains(CollisionEventFlags::SENSOR);
        if start {
            phys.touching.insert(key(a, b));
            started.insert(key(a, b));
            push(&mut out, phys, a, b, 0, sensor);
        } else {
            phys.touching.remove(&key(a, b));
            push(&mut out, phys, a, b, 2, sensor);
        }
    }
    let stay: Vec<(ColliderHandle, ColliderHandle)> = phys
        .touching
        .iter()
        .filter(|k| !started.contains(k))
        .copied()
        .collect();
    for (a, b) in stay {
        let alive = phys.world.colliders.get(a).is_some() && phys.world.colliders.get(b).is_some();
        if !alive {
            phys.touching.remove(&(a, b));
            continue;
        }
        let sensor = phys.world.colliders.get(a).is_some_and(|c| c.is_sensor())
            || phys.world.colliders.get(b).is_some_and(|c| c.is_sensor());
        push(&mut out, phys, a, b, 1, sensor);
    }
    out
}

fn dispatch(world: &mut World, msgs: Vec<(GameObjectId, Msg)>) {
    for (target, msg) in msgs {
        world.broadcast(target, |c, ctx| match &msg {
            Msg::CollisionEnter(col) => c.on_collision_enter(ctx, col),
            Msg::CollisionStay(col) => c.on_collision_stay(ctx, col),
            Msg::CollisionExit(col) => c.on_collision_exit(ctx, col),
            Msg::TriggerEnter(o) => c.on_trigger_enter(ctx, *o),
            Msg::TriggerStay(o) => c.on_trigger_stay(ctx, *o),
            Msg::TriggerExit(o) => c.on_trigger_exit(ctx, *o),
        });
    }
    world.flush_destroyed();
}

/// The physics system: one Rapier step per `FixedUpdate`.
#[derive(Default)]
pub struct PhysicsSystem;

impl oxide_scene::System for PhysicsSystem {
    fn name(&self) -> &'static str {
        "Physics"
    }

    fn fixed_update(&mut self, world: &mut World) {
        let Some(mut phys) = world.resources.remove::<Physics>() else {
            return;
        };
        let dt = world.resource::<Time>().fixed_delta_time();
        phys.world.integration_parameters.dt = dt;
        let mut seen = HashSet::new();
        sync_bodies(world, &mut phys, &mut seen, dt);
        let mass_dirty = sync_colliders(world, &mut phys, &mut seen);
        remove_stale_bodies(&mut phys, &seen);
        // Mass must follow Rigidbody.mass edits as well as collider changes.
        let dynamic: Vec<GameObjectId> = phys
            .bodies
            .iter()
            .filter(|(_, b)| b.kind == BodyKind::Dynamic)
            .map(|(k, _)| *k)
            .collect();
        for id in dynamic {
            let want = phys.bodies[&id].mass;
            let have = phys
                .world
                .bodies
                .get(phys.bodies[&id].handle)
                .map_or(0.0, |b| b.mass());
            if mass_dirty.contains(&id) || (want - have).abs() > want * 1e-3 {
                update_mass(&mut phys, id);
            }
        }
        sync_joints(world, &mut phys);
        crate::vehicle::pre_step(world, &mut phys, dt);
        let Physics {
            world: pw, sink, ..
        } = &mut phys;
        pw.step_with_events(&(), &*sink);
        crate::vehicle::post_step(world, &mut phys, dt);
        write_back(world, &mut phys);
        let msgs = collect_events(&mut phys);
        world.resources.insert(phys);
        dispatch(world, msgs);
    }

    fn update(&mut self, world: &mut World) {
        // Rigidbody interpolation: show bodies between the last two physics poses.
        let (t, ft, fdt) = {
            let time = world.resource::<Time>();
            (time.time(), time.fixed_time(), time.fixed_delta_time())
        };
        let alpha = ((t - ft) / fdt.max(1e-6)).clamp(0.0, 1.0);
        let Some(mut phys) = world.resources.remove::<Physics>() else {
            return;
        };
        for id in ids_top_down(world, &phys) {
            let e = phys.bodies.get_mut(&id).expect("listed");
            if !e.interpolate || e.kind != BodyKind::Dynamic {
                continue;
            }
            let p = e.prev.0.lerp(e.curr.0, alpha);
            let r = e.prev.1.slerp(e.curr.1, alpha);
            world.set_rotation(id, r);
            world.set_position(id, p);
            e.synced = (p, r);
        }
        crate::vehicle::pose_wheels(world, &phys);
        world.resources.insert(phys);
    }

    fn always(&mut self, world: &mut World) {
        let Some(phys) = world.resources.remove::<Physics>() else {
            return;
        };
        if phys.debug_draw
            && let Some(mut dd) = world.resources.remove::<DebugDraw>()
        {
            for (h, c) in phys.world.colliders.iter() {
                let kind = c
                    .parent()
                    .and_then(|b| phys.world.bodies.get(b))
                    .map(|b| b.body_type());
                let color = if c.is_sensor() {
                    Color::YELLOW
                } else if kind == Some(RigidBodyType::Dynamic) {
                    Color::rgb(0.3, 0.6, 1.0)
                } else {
                    Color::GREEN
                };
                let p = c.position();
                if let Some(b) = c.shape().as_cuboid() {
                    dd.wire_box(p.translation, b.half_extents, p.rotation, color);
                } else if let Some(b) = c.shape().as_ball() {
                    dd.wire_sphere(p.translation, b.radius, color);
                } else {
                    let aabb = c.compute_aabb();
                    let center = (aabb.mins + aabb.maxs) * 0.5;
                    dd.wire_box(
                        center,
                        (aabb.maxs - aabb.mins) * 0.5,
                        Quaternion::IDENTITY,
                        color,
                    );
                }
                let _ = h;
            }
            for v in phys.vehicles.values() {
                for w in v.controller.wheels() {
                    let info = w.raycast_info();
                    dd.line(
                        info.hard_point_ws,
                        info.contact_point_ws,
                        Color::rgb(1.0, 0.3, 0.3),
                    );
                }
            }
            world.resources.insert(dd);
        }
        world.resources.insert(phys);
    }
}

#[allow(dead_code)]
fn _unused(_: HashMap<u8, u8>) {}
