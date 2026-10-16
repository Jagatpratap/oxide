//! WheelCollider vehicles: every dynamic Rigidbody with WheelCollider children becomes a
//! Rapier ray-cast vehicle. Wheels hang along their local -Y by `suspension_distance`;
//! `motor_torque`, `brake_torque` and `steer_angle` are read every step, and the wheel
//! state (grounded, rpm, slip, world pose) is written back — Unity's WheelCollider model.
//!
//! Rapier's ray-cast vehicle provides the suspension only. Tyres are simulated here: each
//! wheel has its own spin, driven by motor and brake torque and the road's reaction; the
//! slip ratio and slip angle go through the wheel's `WheelFrictionCurve`s, scaled by the
//! tyre load, and the combined force is limited to a friction ellipse.

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
    /// Per wheel: spin speed (rad/s), spin angle, forward slip, sideways slip.
    tyres: Vec<Tyre>,
}

#[derive(Clone, Copy, Default)]
struct Tyre {
    omega: f32,
    spin: f32,
    forward_slip: f32,
    sideways_slip: f32,
}

/// Wheel-spin integration steps per physics step (the spin is much stiffer than the body).
const TYRE_SUBSTEPS: usize = 10;
/// Below this speed (m/s) slip is measured against this speed instead, so slip ratios and
/// angles stay finite when parked.
const SLIP_MIN_SPEED: f32 = 1.0;

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
                    // Tyre forces are computed below, not by Rapier.
                    side_friction_stiffness: 0.0,
                    friction_slip: 0.0,
                    max_suspension_force: mass * 9.81 * 4.0,
                };
                c.add_wheel(*p, *d, *a, wc.suspension_distance, wc.radius, &tuning);
            }
            // Keep the wheels' spin when only tuning changed.
            let tyres = match phys.vehicles.get(&id) {
                Some(v) if v.tyres.len() == geo.len() => v.tyres.clone(),
                _ => vec![Tyre::default(); geo.len()],
            };
            phys.vehicles.insert(
                id,
                Vehicle {
                    controller: c,
                    wheels: wheels.clone(),
                    signature: sig,
                    tyres,
                },
            );
        }
        let v = phys.vehicles.get_mut(&id).expect("inserted");
        for (i, (_, _, _, wc)) in geo.iter().enumerate() {
            let w = &mut v.controller.wheels_mut()[i];
            w.engine_force = 0.0;
            w.brake = 0.0;
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
        let wcs: Vec<&WheelCollider> = geo.iter().map(|g| &g.3).collect();
        apply_tyre_forces(v, &mut phys.world.bodies, handle, &wcs, mass, dt);
    }
    phys.vehicles.retain(|k, _| live.contains(k));
}

/// Spins each wheel from its motor, brake and road torque and pushes the body with the
/// tyre forces from the friction curves.
fn apply_tyre_forces(
    v: &mut Vehicle,
    bodies: &mut RigidBodySet,
    handle: RigidBodyHandle,
    wcs: &[&WheelCollider],
    mass: f32,
    dt: f32,
) {
    let Some(body) = bodies.get_mut(handle) else {
        return;
    };
    let rot = *body.rotation();
    let up_body = rot * Vector3::Y;
    let com = body.center_of_mass();
    // The share of the car's mass each wheel can stop in one step (keeps low-speed grip
    // from overshooting and jittering).
    let m_share = mass / wcs.len().max(1) as f32;
    let h = dt / TYRE_SUBSTEPS as f32;
    for (i, wc) in wcs.iter().enumerate() {
        let w = &v.controller.wheels()[i];
        let info = w.raycast_info();
        let r = wc.radius.max(0.01);
        let inertia = (0.5 * wc.mass.max(1.0) * r * r).max(0.02);
        let tyre = &mut v.tyres[i];
        let spin_free = |omega: &mut f32, extra: f32, step: f32| {
            *omega +=
                (wc.motor_torque - extra * r - wc.wheel_damping_rate * *omega) * step / inertia;
            let b = wc.brake_torque.max(0.0) * step / inertia;
            *omega = if omega.abs() <= b {
                0.0
            } else {
                *omega - b * omega.signum()
            };
        };
        let load = w.wheel_suspension_force;
        if !info.is_in_contact || load <= 0.0 {
            spin_free(&mut tyre.omega, 0.0, dt);
            tyre.spin += tyre.omega * dt;
            tyre.forward_slip = 0.0;
            tyre.sideways_slip = 0.0;
            continue;
        }
        let n = info.contact_normal_ws;
        let steer = Quaternion::from_axis_angle(Vector3::Y, w.steering);
        // Same frame as Rapier: axle along the wheel's -X, forward = normal × axle.
        let axle = rot * (steer * Vector3::NEG_X);
        let axle = (axle - n * axle.dot(n)).normalize_or(Vector3::NEG_X);
        let fwd = n.cross(axle).normalize_or(Vector3::Z);
        let vel = body.velocity_at_point(info.contact_point_ws);
        let v_long = vel.dot(fwd);
        let v_lat = vel.dot(axle);
        let denom = v_long.abs().max(SLIP_MIN_SPEED);

        // Longitudinal: integrate the wheel's spin against the road in small steps.
        let mut fx_sum = 0.0;
        let mut slip = 0.0;
        for _ in 0..TYRE_SUBSTEPS {
            let slip_speed = tyre.omega * r - v_long;
            slip = slip_speed / denom;
            // At most the force that would match body and tread speed this step, so
            // grip at low speed settles instead of overshooting.
            let stop = m_share * slip_speed.abs() / dt;
            let fx = (slip.signum() * wc.forward_friction.evaluate(slip) * load).clamp(-stop, stop);
            spin_free(&mut tyre.omega, fx, h);
            fx_sum += fx;
        }
        let mut fx = fx_sum / TYRE_SUBSTEPS as f32;

        // Lateral: slip angle through the sideways curve, never more than stops the slide.
        let alpha = (v_lat / denom).atan();
        let mut fy = -alpha.signum() * wc.sideways_friction.evaluate(alpha) * load;
        let stop_y = m_share * v_lat.abs() / dt;
        fy = fy.clamp(-stop_y, stop_y);

        // Friction ellipse: grip used in one direction isn't available in the other.
        let fx_max = (wc.forward_friction.peak() * load).max(1e-3);
        let fy_max = (wc.sideways_friction.peak() * load).max(1e-3);
        let k = ((fx / fx_max).powi(2) + (fy / fy_max).powi(2)).sqrt();
        if k > 1.0 {
            fx /= k;
            fy /= k;
        }

        // Push at the application height, at most up to the centre of mass.
        let to_com = (com - info.contact_point_ws).dot(up_body).max(0.0);
        let point =
            info.contact_point_ws + up_body * wc.force_app_point_distance.clamp(0.0, to_com);
        body.apply_impulse_at_point((fwd * fx + axle * fy) * dt, point, true);
        tyre.spin += tyre.omega * dt;
        tyre.forward_slip = slip;
        tyre.sideways_slip = alpha;
    }
}

pub(crate) fn post_step(world: &mut World, phys: &mut Physics, _dt: f32) {
    let ids: Vec<GameObjectId> = phys.vehicles.keys().copied().collect();
    for id in ids {
        let v = &phys.vehicles[&id];
        let mut states = Vec::new();
        for (i, w) in v.controller.wheels().iter().enumerate() {
            let info = w.raycast_info();
            // Body-space center from the post-step suspension length.
            let local_p = w.chassis_connection_point_cs + w.direction_cs * info.suspension_length;
            let tyre = v.tyres.get(i).copied().unwrap_or_default();
            let local_r = Quaternion::from_axis_angle(Vector3::Y, w.steering)
                * Quaternion::from_axis_angle(Vector3::X, tyre.spin);
            let ground = info.ground_object.and_then(|g| phys.owner(g)).map(|o| o.0);
            let load = if info.is_in_contact {
                w.wheel_suspension_force
            } else {
                0.0
            };
            states.push((i, info, local_p, local_r, tyre, ground, load));
        }
        let wheels = v.wheels.clone();
        for (i, info, lp, lr, tyre, ground, load) in states {
            let (obj, cid) = wheels[i];
            let Some(wc) = world.component_mut::<WheelCollider>(obj, cid) else {
                continue;
            };
            let st = &mut wc.state;
            st.grounded = info.is_in_contact;
            st.suspension_length = info.suspension_length;
            st.local_position = lp;
            st.local_rotation = lr;
            st.spin = tyre.spin;
            st.angular_velocity = tyre.omega;
            st.rpm = tyre.omega / std::f32::consts::TAU * 60.0;
            st.contact_point = info.contact_point_ws;
            st.contact_normal = info.contact_normal_ws;
            st.ground = ground;
            st.load = load;
            st.forward_slip = tyre.forward_slip;
            st.sideways_slip = tyre.sideways_slip;
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
