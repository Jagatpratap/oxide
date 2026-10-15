//! WheelCollider vehicles: every dynamic Rigidbody with WheelCollider children becomes a
//! Rapier ray-cast vehicle. Wheels hang along their local -Y by `suspension_distance`;
//! `motor_torque`, `brake_torque` and `steer_angle` are read every step, and the wheel
//! state (grounded, rpm, slip, world pose) is written back — Unity's WheelCollider model.

use crate::components::{Rigidbody, WheelCollider};
use crate::physics::{BodyKind, Physics};
use oxide_math::{Quaternion, Vector3};
use oxide_scene::{ComponentId, GameObjectId, World};
use rapier3d::control::{DynamicRayCastVehicleController, WheelTuning};
use rapier3d::prelude::*;
use std::hash::{DefaultHasher, Hash, Hasher};

pub(crate) struct Vehicle {
    pub controller: DynamicRayCastVehicleController,
    wheels: Vec<(GameObjectId, ComponentId)>,
    signature: u64,
}

fn wheels_of(world: &World, body: GameObjectId) -> Vec<(GameObjectId, ComponentId)> {
    world
        .descendants(body)
        .into_iter()
        .filter(|&d| world.ancestor_with::<Rigidbody>(d) == Some(body))
        .filter(|&d| world.get(d).is_some_and(|o| o.active_in_hierarchy()))
        .filter_map(|d| {
            world
                .component_id::<WheelCollider>(d)
                .filter(|&c| world.is_component_enabled(d, c))
                .map(|c| (d, c))
        })
        .collect()
}

pub(crate) fn pre_step(world: &mut World, phys: &mut Physics, dt: f32) {
    let bodies: Vec<(GameObjectId, RigidBodyHandle)> = phys
        .bodies
        .iter()
        .filter(|(_, b)| b.kind == BodyKind::Dynamic)
        .map(|(k, b)| (*k, b.handle))
        .collect();
    let mut live = Vec::new();
    for (id, handle) in bodies {
        let wheels = wheels_of(world, id);
        if wheels.is_empty() {
            continue;
        }
        live.push(id);
        let (bp, br) = (world.position(id), world.rotation(id));
        let mass = phys
            .world
            .bodies
            .get(handle)
            .map_or(1.0, |b| b.mass())
            .max(1.0);
        // Geometry in body space; rebuild the controller only when it changes.
        let geo: Vec<(Vector3, Vector3, Vector3, WheelCollider)> = wheels
            .iter()
            .map(|&(w, c)| {
                let wc = world
                    .component::<WheelCollider>(w, c)
                    .expect("listed")
                    .clone();
                let rot = br.inverse() * world.rotation(w);
                let pos = br.inverse() * (world.transform_point(w, wc.center) - bp);
                (pos, rot * Vector3::NEG_Y, rot * Vector3::NEG_X, wc)
            })
            .collect();
        let mut h = DefaultHasher::new();
        for (p, d, a, wc) in &geo {
            // Quantized so float noise from the moving transform doesn't force rebuilds.
            for f in [
                p.x,
                p.y,
                p.z,
                d.x,
                d.y,
                d.z,
                a.x,
                a.y,
                a.z,
                wc.radius,
                wc.suspension_distance,
                wc.suspension_spring,
                wc.suspension_damper,
                wc.forward_friction,
                wc.sideways_friction,
            ] {
                ((f * 1000.0).round() as i64).hash(&mut h);
            }
        }
        (mass as u32).hash(&mut h);
        let sig = h.finish();
        let rebuild = phys
            .vehicles
            .get(&id)
            .is_none_or(|v| v.signature != sig || v.wheels != wheels);
        if rebuild {
            let mut c = DynamicRayCastVehicleController::new(handle);
            c.index_up_axis = 1;
            c.index_forward_axis = 2;
            for (p, d, a, wc) in &geo {
                let tuning = WheelTuning {
                    suspension_stiffness: wc.suspension_spring,
                    suspension_compression: wc.suspension_damper,
                    suspension_damping: wc.suspension_damper * 1.2,
                    max_suspension_travel: wc.suspension_distance,
                    side_friction_stiffness: wc.sideways_friction,
                    friction_slip: wc.forward_friction,
                    max_suspension_force: mass * 9.81 * 4.0,
                };
                c.add_wheel(*p, *d, *a, wc.suspension_distance, wc.radius, &tuning);
            }
            phys.vehicles.insert(
                id,
                Vehicle {
                    controller: c,
                    wheels: wheels.clone(),
                    signature: sig,
                },
            );
        }
        let v = phys.vehicles.get_mut(&id).expect("inserted");
        for (i, (_, _, _, wc)) in geo.iter().enumerate() {
            let w = &mut v.controller.wheels_mut()[i];
            let r = wc.radius.max(0.01);
            w.engine_force = wc.motor_torque / r;
            w.brake = wc.brake_torque / r * dt;
            w.steering = wc.steer_angle.to_radians();
        }
        let filter = QueryFilter::default()
            .exclude_rigid_body(handle)
            .exclude_sensors();
        let pw = &mut phys.world;
        let q = pw.broad_phase.as_query_pipeline_mut(
            pw.narrow_phase.query_dispatcher(),
            &mut pw.bodies,
            &mut pw.colliders,
            filter,
        );
        v.controller.update_vehicle(dt, q);
    }
    phys.vehicles.retain(|k, _| live.contains(k));
}

pub(crate) fn post_step(world: &mut World, phys: &mut Physics, dt: f32) {
    let ids: Vec<GameObjectId> = phys.vehicles.keys().copied().collect();
    for id in ids {
        let v = &phys.vehicles[&id];
        let Some(body) = phys.world.bodies.get(v.controller.chassis) else {
            continue;
        };
        let br = *body.rotation();
        let mut states = Vec::new();
        for (i, w) in v.controller.wheels().iter().enumerate() {
            let info = w.raycast_info();
            // Body-space center from the post-step suspension length.
            let local_p = w.chassis_connection_point_cs + w.direction_cs * info.suspension_length;
            let local_r = Quaternion::from_axis_angle(Vector3::Y, w.steering)
                * Quaternion::from_axis_angle(Vector3::X, w.rotation);
            let vel = body.velocity_at_point(info.contact_point_ws);
            let fwd = (br * (Quaternion::from_axis_angle(Vector3::Y, w.steering) * Vector3::Z))
                .normalize_or(Vector3::Z);
            let axle = (br * (Quaternion::from_axis_angle(Vector3::Y, w.steering) * Vector3::X))
                .normalize_or(Vector3::X);
            let ground = info.ground_object.and_then(|g| phys.owner(g)).map(|o| o.0);
            states.push((
                i,
                info.is_in_contact,
                info.suspension_length,
                local_p,
                local_r,
                w.rotation,
                0.0,
                info.contact_point_ws,
                info.contact_normal_ws,
                ground,
                vel.dot(fwd),
                vel.dot(axle),
                w.radius,
            ));
        }
        let wheels = v.wheels.clone();
        for (i, grounded, susp, lp, lr, spin, dspin, cp, cn, ground, vf, vs, radius) in states {
            let (obj, cid) = wheels[i];
            let Some(wc) = world.component_mut::<WheelCollider>(obj, cid) else {
                continue;
            };
            let st = &mut wc.state;
            st.grounded = grounded;
            st.suspension_length = susp;
            st.local_position = lp;
            st.local_rotation = lr;
            let dspin = if dspin == 0.0 { spin - st.spin } else { dspin };
            st.spin = spin;
            st.rpm = dspin / dt.max(1e-6) / std::f32::consts::TAU * 60.0;
            st.contact_point = cp;
            st.contact_normal = cn;
            st.ground = ground;
            st.sideways_slip = if grounded { vs.abs() } else { 0.0 };
            st.forward_slip = if grounded {
                (dspin / dt.max(1e-6) * radius - vf).abs()
            } else {
                0.0
            };
        }
    }
}

/// Places wheel poses (and wheel models) relative to the vehicle's current, possibly
/// interpolated, transform. Runs every frame after interpolation.
pub(crate) fn pose_wheels(world: &mut World, phys: &Physics) {
    for (&id, v) in &phys.vehicles {
        let (bp, br) = (world.position(id), world.rotation(id));
        for &(obj, cid) in &v.wheels {
            let Some(wc) = world.component_mut::<WheelCollider>(obj, cid) else {
                continue;
            };
            let p = bp + br * wc.state.local_position;
            let r = br * wc.state.local_rotation;
            wc.state.position = p;
            wc.state.rotation = r;
            if let Some(m) = wc.wheel_model {
                world.set_position(m, p);
                world.set_rotation(m, r);
            }
        }
    }
}

#[allow(dead_code)]
fn _unused(_: &Rigidbody) {}
