//! Oxide scenes: Unity's GameObject / Transform / Component model.
//!
//! - [`World`]: all GameObjects, the Transform hierarchy, component storage, the player loop
//!   messages (`awake`, `start`, `update`, ...) and engine resources.
//! - [`Component`]: implement on a `#[derive(Reflect, Clone, Default)]` struct to write a
//!   script; [`Ctx`] gives it `transform`, `GetComponent`, `Instantiate`, `Time`, `Input`.
//! - [`serialize`]: scene/prefab files and `Instantiate`.
//! - [`builtin`]: Camera, Light, MeshFilter, MeshRenderer, SkinnedMeshRenderer, RenderSettings.

pub mod builtin;
pub mod component;
pub mod ctx;
pub mod id;
pub mod resources;
pub mod serialize;
pub mod system;
pub mod timers;
pub mod world;

pub use builtin::*;
pub use component::{Collision, Component, ComponentRegistry, ContactPoint};
pub use ctx::{Ctx, TransformRef};
pub use id::{ComponentId, GameObjectId};
pub use resources::Resources;
pub use serialize::{ComponentData, ObjectData, SceneFile};
pub use system::{DebugDraw, DebugLine, System};
pub use timers::Timers;
pub use world::{GameObject, LayerNames, Phase, Space, Transform, World};

/// Registers the built-in components with their Add Component menu paths.
pub fn register_builtin(reg: &mut ComponentRegistry) {
    reg.register::<Camera>("Rendering/Camera");
    reg.register::<Light>("Rendering/Light");
    reg.register::<MeshFilter>("Mesh/Mesh Filter");
    reg.register::<MeshRenderer>("Mesh/Mesh Renderer");
    reg.register::<SkinnedMeshRenderer>("Mesh/Skinned Mesh Renderer");
}

#[cfg(test)]
mod tests;
