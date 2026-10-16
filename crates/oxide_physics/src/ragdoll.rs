//! Unity's Ragdoll Wizard (GameObject → 3D Object → Ragdoll...): given the main bones of a
//! character it adds a Rigidbody, a collider and a `CharacterJoint` to each, with Unity's
//! mass split and joint limits. Reference:
//! <https://docs.unity3d.com/Manual/wizard-RagdollWizard.html>

use crate::components::*;
use oxide_math::Vector3;
use oxide_scene::{GameObjectId, World};

/// The bones the Ragdoll Wizard asks for. Hands are optional (they set the forearm length).
#[derive(Clone, Debug)]
pub struct RagdollBones {
    pub pelvis: GameObjectId,
    pub left_hips: GameObjectId,
    pub left_knee: GameObjectId,
    pub left_foot: GameObjectId,
    pub right_hips: GameObjectId,
    pub right_knee: GameObjectId,
    pub right_foot: GameObjectId,
    pub left_arm: GameObjectId,
    pub left_elbow: GameObjectId,
    pub left_hand: Option<GameObjectId>,
    pub right_arm: GameObjectId,
    pub right_elbow: GameObjectId,
    pub right_hand: Option<GameObjectId>,
    pub middle_spine: GameObjectId,
    pub head: GameObjectId,
}

impl RagdollBones {
    pub fn all(&self) -> Vec<GameObjectId> {
        vec![
            self.pelvis,
            self.left_hips,
            self.left_knee,
            self.right_hips,
            self.right_knee,
            self.left_arm,
            self.left_elbow,
            self.right_arm,
            self.right_elbow,
            self.middle_spine,
            self.head,
        ]
    }
}

fn clean(world: &mut World, id: GameObjectId) {
    for name in [
        "CharacterJoint",
        "Rigidbody",
        "BoxCollider",
        "SphereCollider",
        "CapsuleCollider",
    ] {
        for cid in world.components_named(id, name) {
            world.remove_component_now(id, cid);
        }
    }
}

fn max_scale(world: &World, id: GameObjectId) -> f32 {
    let s = world.lossy_scale(id).abs();
    s.x.max(s.y).max(s.z).max(1e-6)
}

/// A capsule along the bone from `id` to the world point `end`, `radius` × its length.
fn limb(world: &World, id: GameObjectId, end: Vector3, radius: f32) -> CapsuleCollider {
    let local = world.inverse_transform_point(id, end);
    let a = local.abs();
    let (direction, len) = if a.x >= a.y && a.x >= a.z {
        (CapsuleDirection::XAxis, a.x)
    } else if a.y >= a.z {
        (CapsuleDirection::YAxis, a.y)
    } else {
        (CapsuleDirection::ZAxis, a.z)
    };
    let r = (len * radius).max(1e-4);
    CapsuleCollider {
        center: match direction {
            CapsuleDirection::XAxis => Vector3::X * local.x * 0.5,
            CapsuleDirection::YAxis => Vector3::Y * local.y * 0.5,
            CapsuleDirection::ZAxis => Vector3::Z * local.z * 0.5,
        },
        radius: r,
        height: len.max(2.0 * r),
        direction,
        ..Default::default()
    }
}

/// A box around world points (in `id`'s space), padded so it has some thickness.
fn box_around(world: &World, id: GameObjectId, points: &[Vector3]) -> BoxCollider {
    let local: Vec<Vector3> = points
        .iter()
        .map(|&p| world.inverse_transform_point(id, p))
        .chain([Vector3::ZERO])
        .collect();
    let min = local
        .iter()
        .fold(Vector3::splat(f32::MAX), |m, p| m.min(*p));
    let max = local
        .iter()
        .fold(Vector3::splat(f32::MIN), |m, p| m.max(*p));
    let ext = max - min;
    let big = ext.max_element();
    let size = ext.max(Vector3::splat(big * 0.5)) * 1.1;
    BoxCollider {
        center: (min + max) * 0.5,
        size,
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn joint(
    world: &World,
    id: GameObjectId,
    parent: GameObjectId,
    twist: Vector3,
    swing: Vector3,
    low: f32,
    high: f32,
    swing1: f32,
    swing2: f32,
) -> CharacterJoint {
    let dir = |v: Vector3| {
        world
            .inverse_transform_direction(id, v)
            .normalize_or(Vector3::X)
    };
    CharacterJoint {
        connected_body: Some(parent),
        anchor: Vector3::ZERO,
        axis: dir(twist),
        swing_axis: dir(swing),
        low_twist_limit: low,
        high_twist_limit: high,
        swing1_limit: swing1,
        swing2_limit: swing2,
        enable_collision: false,
    }
}

/// Adds the ragdoll to the bones (replacing any earlier one). The bodies start kinematic
/// when `kinematic` is set, so an Animator can keep driving them until
/// [`set_ragdoll_kinematic`] switches physics on. Returns the bones that got bodies.
pub fn build_ragdoll(
    world: &mut World,
    b: &RagdollBones,
    total_mass: f32,
    kinematic: bool,
) -> Result<Vec<GameObjectId>, String> {
    for id in b.all() {
        if !world.is_alive(id) {
            return Err(format!("ragdoll bone {id:?} does not exist"));
        }
    }
    let character = world.root_of(b.pelvis);
    let (right, up, fwd) = (
        world.right(character),
        world.up(character),
        world.forward(character),
    );
    let pos = |id: GameObjectId| world.position(id);
    let child_end = |world: &World, id: GameObjectId, parent: GameObjectId| -> Vector3 {
        // No hand given: extend the forearm by the upper arm's length.
        let d = pos(id) - pos(parent);
        world
            .children(id)
            .first()
            .map(|&c| world.position(c))
            .unwrap_or(pos(id) + d)
    };
    let left_hand = b
        .left_hand
        .map(pos)
        .unwrap_or_else(|| child_end(world, b.left_elbow, b.left_arm));
    let right_hand = b
        .right_hand
        .map(pos)
        .unwrap_or_else(|| child_end(world, b.right_elbow, b.right_arm));

    let pelvis_box = box_around(
        world,
        b.pelvis,
        &[pos(b.left_hips), pos(b.right_hips), pos(b.middle_spine)],
    );
    let chest_top = pos(b.head);
    let mut chest_box = box_around(
        world,
        b.middle_spine,
        &[pos(b.left_arm), pos(b.right_arm), chest_top],
    );
    // Keep the chest box clear of the head sphere.
    chest_box.size *= 0.9;
    let head_len = pos(b.head).distance(pos(b.middle_spine));
    let head_r = head_len * 0.3;
    let head_center = world.inverse_transform_point(b.head, pos(b.head) + up * head_r);
    let head_sphere = SphereCollider {
        center: head_center,
        radius: head_r / max_scale(world, b.head),
        ..Default::default()
    };

    // Unity's mass split.
    let parts: Vec<(GameObjectId, f32, Box<dyn oxide_scene::Component>)> = vec![
        (b.pelvis, 0.2, Box::new(pelvis_box)),
        (b.middle_spine, 0.2, Box::new(chest_box)),
        (b.head, 0.08, Box::new(head_sphere)),
        (
            b.left_hips,
            0.1,
            Box::new(limb(world, b.left_hips, pos(b.left_knee), 0.2)),
        ),
        (
            b.right_hips,
            0.1,
            Box::new(limb(world, b.right_hips, pos(b.right_knee), 0.2)),
        ),
        (
            b.left_knee,
            0.07,
            Box::new(limb(world, b.left_knee, pos(b.left_foot), 0.18)),
        ),
        (
            b.right_knee,
            0.07,
            Box::new(limb(world, b.right_knee, pos(b.right_foot), 0.18)),
        ),
        (
            b.left_arm,
            0.05,
            Box::new(limb(world, b.left_arm, pos(b.left_elbow), 0.25)),
        ),
        (
            b.right_arm,
            0.05,
            Box::new(limb(world, b.right_arm, pos(b.right_elbow), 0.25)),
        ),
        (
            b.left_elbow,
            0.04,
            Box::new(limb(world, b.left_elbow, left_hand, 0.2)),
        ),
        (
            b.right_elbow,
            0.04,
            Box::new(limb(world, b.right_elbow, right_hand, 0.2)),
        ),
    ];
    // Twist about the character's right axis: positive swings a limb forward.
    let joints = [
        joint(
            world,
            b.left_hips,
            b.pelvis,
            right,
            fwd,
            -20.0,
            70.0,
            30.0,
            20.0,
        ),
        joint(
            world,
            b.right_hips,
            b.pelvis,
            right,
            fwd,
            -20.0,
            70.0,
            30.0,
            20.0,
        ),
        joint(
            world,
            b.left_knee,
            b.left_hips,
            right,
            fwd,
            -120.0,
            0.0,
            5.0,
            5.0,
        ),
        joint(
            world,
            b.right_knee,
            b.right_hips,
            right,
            fwd,
            -120.0,
            0.0,
            5.0,
            5.0,
        ),
        joint(
            world,
            b.left_arm,
            b.middle_spine,
            fwd,
            up,
            -70.0,
            70.0,
            60.0,
            30.0,
        ),
        joint(
            world,
            b.right_arm,
            b.middle_spine,
            fwd,
            up,
            -70.0,
            70.0,
            60.0,
            30.0,
        ),
        // Elbows bend forward: about up for the left arm, down for the right.
        joint(
            world,
            b.left_elbow,
            b.left_arm,
            up,
            fwd,
            -140.0,
            0.0,
            5.0,
            5.0,
        ),
        joint(
            world,
            b.right_elbow,
            b.right_arm,
            -up,
            fwd,
            -140.0,
            0.0,
            5.0,
            5.0,
        ),
        joint(
            world,
            b.middle_spine,
            b.pelvis,
            right,
            fwd,
            -20.0,
            20.0,
            10.0,
            10.0,
        ),
        joint(
            world,
            b.head,
            b.middle_spine,
            right,
            fwd,
            -40.0,
            25.0,
            25.0,
            25.0,
        ),
    ];
    let joined = [
        b.left_hips,
        b.right_hips,
        b.left_knee,
        b.right_knee,
        b.left_arm,
        b.right_arm,
        b.left_elbow,
        b.right_elbow,
        b.middle_spine,
        b.head,
    ];
    let mut out = vec![];
    for (id, share, collider) in parts {
        clean(world, id);
        world.add_component(
            id,
            Rigidbody {
                mass: total_mass * share,
                is_kinematic: kinematic,
                ..Default::default()
            },
        );
        world.add_component_boxed(id, collider);
        out.push(id);
    }
    for (id, j) in joined.into_iter().zip(joints) {
        world.add_component(id, j);
    }
    Ok(out)
}

/// Switches every Rigidbody below `root` between animated (kinematic) and simulated.
pub fn set_ragdoll_kinematic(world: &mut World, root: GameObjectId, kinematic: bool) -> usize {
    let mut n = 0;
    for id in world.descendants(root) {
        if let Some(rb) = world.get_component_mut::<Rigidbody>(id) {
            rb.is_kinematic = kinematic;
            n += 1;
        }
    }
    n
}
