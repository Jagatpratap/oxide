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
pub mod constraints;
pub mod coroutines;
pub mod ctx;
pub mod execution_order;
pub mod id;
pub mod lens_flare;
pub mod player_input;
pub mod prefab;
pub mod presets;
pub mod quality;
pub mod render_pipeline;
pub mod resources;
pub mod scene_manager;
pub mod serialize;
pub mod sprite_library;
pub mod system;
pub mod tilemap;
pub mod timers;
pub mod volume;
pub mod world;

pub use application::{Application, RuntimePlatform, Screen};
pub use builtin::*;
pub use component::{Collision, Component, ComponentRegistry, ContactPoint, Gizmos};
pub use constraints::{
    AimConstraint, ConstraintAxes, ConstraintKind, ConstraintSource, ConstraintSystem,
    LookAtConstraint, ParentConstraint, ParentConstraintOffset, PositionConstraint,
    RotationConstraint, ScaleConstraint, WorldUpType,
};
pub use coroutines::{Co, CoroutineId, Coroutines};
pub use ctx::{Ctx, TransformRef};
pub use execution_order::{ScriptExecutionOrder, ScriptOrder};
pub use id::{ComponentId, GameObjectId};
pub use lens_flare::{
    LensFlareBlendMode, LensFlareComponent, LensFlareData, LensFlareDataElement,
    LensFlareDistribution, LensFlareType,
};
pub use player_input::PlayerInput;
pub use prefab::{ObjectProperty, PrefabOverride, is_prefab_path};
pub use presets::{CurvePreset, CurvePresetLibrary, GradientPreset, GradientPresetLibrary};
pub use quality::{QualityLevel, QualitySettings, ShadowQuality};
pub use render_pipeline::{
    GraphicsSettings, PipelineMsaa, RenderingPath, Shader, UniversalRenderPipelineAsset,
};
pub use resources::Resources;
pub use scene_manager::{AsyncOperation, PumpResult, SceneOpStatus, SceneOperations};
pub use serialize::{ComponentData, ObjectData, SceneFile};
pub use sprite_library::{SpriteLibrary, SpriteLibraryAsset, SpriteResolver};
pub use system::{DebugDraw, DebugLine, System};
pub use tilemap::{
    AnimatedTile, Grid, RuleNeighbor, RuleOutput, RuleTile, Tile, TileBase, TileCell, TilePalette,
    Tilemap, TilemapRenderer, TilingRule,
};
pub use timers::Timers;
pub use volume::*;
pub use world::{
    BUILTIN_TAGS, GameObject, LayerNames, Phase, ReloadedComponent, SceneHandle, SceneInfo, Space,
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
    reg.register::<LensFlareComponent>("Rendering/Lens Flare (SRP)");
    volume::register(reg);
    reg.register::<FullScreenPassRendererFeature>("Rendering/Renderer Features/Full Screen Pass");
    reg.register::<RenderObjectsRendererFeature>("Rendering/Renderer Features/Render Objects");
    reg.register::<player_input::PlayerInput>("Input/Player Input");
    reg.register::<SpriteRenderer>("Rendering/Sprite Renderer");
    reg.register::<sprite_library::SpriteLibrary>("2D Animation/Sprite Library");
    reg.register::<sprite_library::SpriteResolver>("2D Animation/Sprite Resolver");
    reg.register::<tilemap::Grid>("Tilemap/Grid");
    reg.register::<tilemap::Tilemap>("Tilemap/Tilemap");
    reg.register::<tilemap::TilemapRenderer>("Tilemap/Tilemap Renderer");
    constraints::register(reg);
}

#[cfg(test)]
mod tests;
