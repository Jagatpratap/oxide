//! Ragdolls on humanoid characters: builds Unity's Ragdoll Wizard setup from the bones an
//! Animator mapped, and switches between animated and simulated (the usual "die and go
//! limp" script).

use oxide_animation::{Animator, HumanBodyBones as B, auto_map};
use oxide_physics::{RagdollBones, build_ragdoll, set_ragdoll_kinematic};
use oxide_scene::{GameObjectId, World};

/// The Ragdoll Wizard's bones, found by humanoid bone mapping below `root`.
pub fn humanoid_ragdoll_bones(world: &World, root: GameObjectId) -> Result<RagdollBones, String> {
    let map = auto_map(world, root);
    let get = |b: B| {
        map.get(&b)
            .copied()
            .ok_or_else(|| format!("no {b:?} bone found below this object"))
    };
    Ok(RagdollBones {
        pelvis: get(B::Hips)?,
        left_hips: get(B::LeftUpperLeg)?,
        left_knee: get(B::LeftLowerLeg)?,
        left_foot: get(B::LeftFoot)?,
        right_hips: get(B::RightUpperLeg)?,
        right_knee: get(B::RightLowerLeg)?,
        right_foot: get(B::RightFoot)?,
        left_arm: get(B::LeftUpperArm)?,
        left_elbow: get(B::LeftLowerArm)?,
        left_hand: get(B::LeftHand).ok(),
        right_arm: get(B::RightUpperArm)?,
        right_elbow: get(B::RightLowerArm)?,
        right_hand: get(B::RightHand).ok(),
        middle_spine: get(B::Chest).or_else(|_| get(B::Spine))?,
        head: get(B::Head)?,
    })
}

/// Adds a ragdoll to a humanoid. Characters with an Animator get kinematic bodies that
/// follow the animation until [`set_ragdoll_active`] lets them fall.
pub fn create_ragdoll(
    world: &mut World,
    root: GameObjectId,
    total_mass: f32,
) -> Result<Vec<GameObjectId>, String> {
    let bones = humanoid_ragdoll_bones(world, root)?;
    let animated = world.has_component::<Animator>(root);
    build_ragdoll(world, &bones, total_mass, animated)
}

/// Ragdoll on: the Animator stops and the bodies simulate. Off: bodies follow the
/// animation again. Returns the number of bodies switched.
pub fn set_ragdoll_active(world: &mut World, root: GameObjectId, active: bool) -> usize {
    if let Some(cid) = world.component_id::<Animator>(root) {
        world.set_component_enabled(root, cid, !active);
    }
    set_ragdoll_kinematic(world, root, !active)
}
