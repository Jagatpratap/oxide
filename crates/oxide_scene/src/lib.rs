//! Oxide scenes: Unity's GameObject / Transform / Component model.
//!
//! - [`World`]: all GameObjects, the Transform hierarchy, component storage, the player loop
//!   messages (`awake`, `start`, `update`, ...) and engine resources.
//! - [`Component`]: implement on a `#[derive(Reflect, Clone, Default)]` struct to write a
//!   script; [`Ctx`] gives it `transform`, `GetComponent`, `Instantiate`, `Time`, `Input`.
//! - [`serialize`]: scene/prefab files and `Instantiate`.
//! - [`builtin`]: Camera, Light, MeshFilter, MeshRenderer, SkinnedMeshRenderer, RenderSettings.

pub mod application;
pub mod builtin;
pub mod component;
pub mod ctx;
pub mod id;
pub mod prefab;
pub mod resources;
pub mod scene_manager;
pub mod serialize;
pub mod system;
pub mod timers;
pub mod volume;
pub mod world;

pub use application::Application;
pub use builtin::*;
pub use component::{Collision, Component, ComponentRegistry, ContactPoint, Gizmos};
pub use ctx::{Ctx, TransformRef};
pub use id::{ComponentId, GameObjectId};
pub use prefab::{ObjectProperty, PrefabOverride, is_prefab_path};
pub use resources::Resources;
pub use scene_manager::{AsyncOperation, PumpResult, SceneOpStatus, SceneOperations};
pub use serialize::{ComponentData, ObjectData, SceneFile};
pub use system::{DebugDraw, DebugLine, System};
pub use timers::Timers;
pub use volume::*;
pub use world::{
    GameObject, LayerNames, Phase, ReloadedComponent, SceneHandle, SceneInfo, Space,
    StaticEditorFlags, Transform, World,
};

/// Registers the built-in components with their Add Component menu paths.
pub fn register_builtin(reg: &mut ComponentRegistry) {
    reg.register::<Camera>("Rendering/Camera");
    reg.register::<Light>("Rendering/Light");
    reg.register::<MeshFilter>("Mesh/Mesh Filter");
    reg.register::<MeshRenderer>("Mesh/Mesh Renderer");
    reg.register::<SkinnedMeshRenderer>("Mesh/Skinned Mesh Renderer");
    reg.register::<LODGroup>("Rendering/LOD Group");
    reg.register::<OcclusionPortal>("Rendering/Occlusion Portal");
    reg.register::<ReflectionProbe>("Rendering/Reflection Probe");
    reg.register::<LightProbeGroup>("Rendering/Light Probe Group");
    volume::register(reg);
    reg.register::<FullScreenPassRendererFeature>("Rendering/Renderer Features/Full Screen Pass");
    reg.register::<RenderObjectsRendererFeature>("Rendering/Renderer Features/Render Objects");
}

#[cfg(test)]
mod tests;
