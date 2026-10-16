//! Inverse kinematics on the animated pose: Unity's humanoid IK goals (two-bone limbs) and
//! LookAt. Runs after the Animator has written its pose and scripts had `OnAnimatorIK`.
//! Reference: <https://docs.unity3d.com/Manual/InverseKinematics.html>

use oxide_math::{Quaternion, Vector3};
use oxide_scene::{GameObjectId, World};

const EPS: f32 = 1e-5;

/// Rotates `upper` and `mid` so the tip of `end` reaches toward `target` (blended by
/// `weight` from the animated position), bending in the plane of `hint` (or the current
/// bend when there is none). Unreachable targets stretch the limb straight toward them.
pub fn solve_two_bone(
    world: &mut World,
    [upper, mid, end]: [GameObjectId; 3],
    target: Vector3,
    weight: f32,
    hint: Option<Vector3>,
) {
    if weight <= 0.0 {
        return;
    }
    let (pu, pm, pe) = (
        world.position(upper),
        world.position(mid),
        world.position(end),
    );
    let (a, b) = (pm.distance(pu), pe.distance(pm));
    if a < EPS || b < EPS {
        return;
    }
    let goal = pe.lerp(target, weight.min(1.0));
    let to_goal = goal - pu;
    let Some(dir) = to_goal.try_normalize() else {
        return;
    };
    let c = to_goal.length().clamp((a - b).abs() + 1e-4, a + b - 1e-4);
    // Bend direction: perpendicular to the reach direction, toward the hint / current knee.
    let bend = hint.map_or(pm - pu, |h| h - pu);
    let perp = bend - dir * bend.dot(dir);
    let perp = perp
        .try_normalize()
        .or_else(|| {
            let p = (pm - pu) - dir * (pm - pu).dot(dir);
            p.try_normalize()
        })
        .unwrap_or_else(|| dir.any_orthonormal_vector());
    let cos_a = ((a * a + c * c - b * b) / (2.0 * a * c)).clamp(-1.0, 1.0);
    let sin_a = (1.0 - cos_a * cos_a).max(0.0).sqrt();
    let mid_goal = pu + (dir * cos_a + perp * sin_a) * a;
    rotate_toward(world, upper, pm - pu, mid_goal - pu);
    let (pm2, pe2) = (world.position(mid), world.position(end));
    rotate_toward(world, mid, pe2 - pm2, pu + dir * c - pm2);
}

/// Applies the world rotation taking direction `from` to `to` to `id`.
fn rotate_toward(world: &mut World, id: GameObjectId, from: Vector3, to: Vector3) {
    let (Some(f), Some(t)) = (from.try_normalize(), to.try_normalize()) else {
        return;
    };
    let q = Quaternion::from_rotation_arc(f, t);
    world.set_rotation(id, (q * world.rotation(id)).normalize());
}

/// Blends `id`'s world rotation toward `rotation`.
pub fn set_end_rotation(world: &mut World, id: GameObjectId, rotation: Quaternion, weight: f32) {
    if weight <= 0.0 {
        return;
    }
    let cur = world.rotation(id);
    world.set_rotation(id, cur.slerp(rotation, weight.min(1.0)));
}

/// Unity `LookAt` weights.
pub struct LookAt {
    pub target: Vector3,
    pub weight: f32,
    pub body: f32,
    pub head: f32,
    /// 0 = unrestricted, 0.5 = at most 90° from forward, 1 = no motion.
    pub clamp: f32,
}

/// Turns the spine (by `body`) then neck and head (by `head`) toward the target.
/// `forward` is the direction the animated head faces (the character's forward).
pub fn look_at(
    world: &mut World,
    forward: Vector3,
    spine: &[GameObjectId],
    head: &[GameObjectId],
    l: &LookAt,
) {
    let w = l.weight.clamp(0.0, 1.0);
    let Some(&tip) = head.last() else { return };
    if w <= 0.0 {
        return;
    }
    let max_angle = (1.0 - l.clamp.clamp(0.0, 1.0)) * std::f32::consts::PI;
    let turn = |world: &World| -> Option<Quaternion> {
        let fwd = forward.try_normalize()?;
        let mut dir = (l.target - world.position(tip)).try_normalize()?;
        let ang = fwd.angle_between(dir);
        if ang > max_angle {
            dir = fwd.slerp_toward(dir, max_angle / ang);
        }
        Some(Quaternion::from_rotation_arc(fwd, dir))
    };
    let Some(q) = turn(world) else { return };
    let body = (w * l.body.clamp(0.0, 1.0) * 0.5).min(1.0);
    for &b in spine {
        let part = Quaternion::IDENTITY.slerp(q, body / spine.len() as f32);
        world.set_rotation(b, (part * world.rotation(b)).normalize());
    }
    // The head finishes what the body left, from where the spine moved it.
    let moved = Quaternion::IDENTITY.slerp(q, body);
    let rest = moved.inverse() * q;
    let h = w * l.head.clamp(0.0, 1.0);
    for &b in head {
        let part = Quaternion::IDENTITY.slerp(rest, h / head.len() as f32);
        world.set_rotation(b, (part * world.rotation(b)).normalize());
    }
}

trait SlerpToward {
    fn slerp_toward(self, to: Self, t: f32) -> Self;
}

impl SlerpToward for Vector3 {
    fn slerp_toward(self, to: Vector3, t: f32) -> Vector3 {
        Quaternion::IDENTITY.slerp(Quaternion::from_rotation_arc(self, to), t) * self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_bone_reaches_target_and_bends_toward_hint() {
        let mut w = World::new();
        let upper = w.create("UpperArm");
        let mid = w.create("LowerArm");
        let end = w.create("Hand");
        w.set_parent(mid, Some(upper), false).unwrap();
        w.set_parent(end, Some(mid), false).unwrap();
        w.transform_mut(mid).unwrap().local_position = Vector3::new(0.0, -1.0, 0.0);
        w.transform_mut(end).unwrap().local_position = Vector3::new(0.0, -1.0, 0.0);
        let target = Vector3::new(1.0, -0.5, 0.0);
        let hint = Vector3::new(0.0, 0.0, 5.0);
        solve_two_bone(&mut w, [upper, mid, end], target, 1.0, Some(hint));
        assert!(
            w.position(end).distance(target) < 1e-3,
            "{}",
            w.position(end)
        );
        assert!(w.position(mid).z > 0.1, "elbow bends toward the hint");
        // Bone lengths are kept.
        assert!((w.position(mid).length() - 1.0).abs() < 1e-3);
        // Out of reach: points straight at it.
        solve_two_bone(
            &mut w,
            [upper, mid, end],
            Vector3::new(5.0, 0.0, 0.0),
            1.0,
            None,
        );
        assert!(w.position(end).distance(Vector3::new(2.0, 0.0, 0.0)) < 1e-2);
    }
}
