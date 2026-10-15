//! Unity `CharacterController.Move`: collide-and-slide movement with step climbing, slope
//! limits and ground snapping, run immediately from scripts.

use crate::components::{CharacterController, CollisionFlags};
use crate::physics::{Physics, pose};
use oxide_core::Time;
use oxide_math::{Quaternion, Vector3};
use oxide_scene::{Ctx, GameObjectId, World};
use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::prelude::*;

/// Moves `id` (which must have a CharacterController) by `motion` in world space and
/// returns what it touched. Gravity is up to the caller, as in Unity.
pub fn move_character(world: &mut World, id: GameObjectId, motion: Vector3) -> CollisionFlags {
    let Some(cc) = world.get_component::<CharacterController>(id).cloned() else {
        return CollisionFlags::default();
    };
    let dt = world.resource::<Time>().delta_time().max(1e-4);
    let s = world.lossy_scale(id).abs();
    let r = (cc.radius * s.x.max(s.z)).max(1e-3);
    let half = (cc.height * s.y * 0.5 - r).max(0.0);
    let pos = world.position(id);
    let center = pos + cc.center * s;
    let Some(phys) = world.try_resource::<Physics>() else {
        world.set_position(id, pos + motion);
        return CollisionFlags::default();
    };
    let controller = KinematicCharacterController {
        up: Vector3::Y,
        offset: CharacterLength::Absolute(cc.skin_width.max(0.005)),
        slide: true,
        autostep: (cc.step_offset > 0.0).then_some(CharacterAutostep {
            max_height: CharacterLength::Absolute(cc.step_offset),
            min_width: CharacterLength::Absolute(r * 0.5),
            include_dynamic_bodies: false,
        }),
        max_slope_climb_angle: cc.slope_limit.to_radians(),
        min_slope_slide_angle: cc.slope_limit.to_radians(),
        snap_to_ground: (motion.y <= 0.0)
            .then_some(CharacterLength::Absolute(cc.step_offset.max(0.05))),
        ..Default::default()
    };
    let shape = Capsule::new_y(half, r);
    let mut filter = QueryFilter::default().exclude_sensors();
    if let Some(b) = phys.bodies.get(&id) {
        filter = filter.exclude_rigid_body(b.handle);
    }
    let q = phys.world.query_pipeline_with_filter(filter);
    let mut flags = CollisionFlags::default();
    let result = controller.move_shape(
        dt,
        &q,
        &shape,
        &pose(center, Quaternion::IDENTITY),
        motion,
        |c| {
            let n = c.hit.normal1;
            if n.y > 0.7 {
                flags.below = true;
            } else if n.y < -0.7 {
                flags.above = true;
            } else {
                flags.sides = true;
            }
        },
    );
    flags.below |= result.grounded;
    let delta = result.translation;
    world.set_position(id, pos + delta);
    if let Some(c) = world.get_component_mut::<CharacterController>(id) {
        c.is_grounded = flags.below;
        c.velocity = delta / dt;
        c.collision_flags = flags;
    }
    flags
}

/// `ctx.move_character(motion)` from a script on the character.
pub trait CharacterExt {
    fn move_character(&mut self, motion: Vector3) -> CollisionFlags;
}

impl CharacterExt for Ctx<'_> {
    fn move_character(&mut self, motion: Vector3) -> CollisionFlags {
        let id = self.this;
        move_character(self.world, id, motion)
    }
}
