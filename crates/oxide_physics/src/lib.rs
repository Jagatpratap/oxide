//! Oxide physics on Rapier, with Unity's component model and API:
//! Rigidbody (forces, ForceMode, interpolation, constraints), Box/Sphere/Capsule/Mesh/Terrain
//! colliders (triggers, friction, bounciness, layer collision matrix), compound colliders on
//! child objects, `on_collision_*` / `on_trigger_*` messages, `Physics.Raycast`/`SphereCast`/
//! `OverlapSphere`, CharacterController.Move, WheelCollider vehicles, Fixed/Hinge/Character joints, ragdolls.
//!
//! Reference: <https://docs.unity3d.com/Manual/PhysicsSection.html>

pub mod character;
pub mod components;
pub mod physics;
pub mod ragdoll;
pub mod sync;
pub mod vehicle;

pub use character::{CharacterExt, move_character};
pub use components::*;
pub use physics::{DEFAULT_RAYCAST_LAYERS, Physics, PhysicsExt, RaycastHit};
pub use ragdoll::{RagdollBones, build_ragdoll, set_ragdoll_kinematic};
pub use sync::PhysicsSystem;

use oxide_scene::{System, World};

/// Installs the `Physics` resource and physics components; returns the system to run.
pub fn install(world: &mut World) -> Box<dyn System> {
    world.resources.insert(Physics::default());
    if let Some(a) = world.resources.get_mut::<oxide_assets::AssetServer>() {
        a.register_data_asset::<PhysicsMaterial>(
            oxide_assets::AssetTypeInfo::new("PhysicsMaterial", &["physicMaterial"])
                .menu("Physics Material", "New Physics Material", 40)
                .color(120, 200, 150),
        );
    }
    let r = &mut world.registry;
    r.register::<Rigidbody>("Physics/Rigidbody");
    r.register::<BoxCollider>("Physics/Box Collider");
    r.register::<SphereCollider>("Physics/Sphere Collider");
    r.register::<CapsuleCollider>("Physics/Capsule Collider");
    r.register::<MeshCollider>("Physics/Mesh Collider");
    r.register::<TerrainCollider>("Physics/Terrain Collider");
    r.register::<CharacterController>("Physics/Character Controller");
    r.register::<WheelCollider>("Physics/Wheel Collider");
    r.register::<FixedJoint>("Physics/Fixed Joint");
    r.register::<HingeJoint>("Physics/Hinge Joint");
    r.register::<CharacterJoint>("Physics/Character Joint");
    Box::new(PhysicsSystem)
}

#[cfg(test)]
mod tests;
