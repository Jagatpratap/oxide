//! Oxide runtime: the window, Unity's player loop, scene → renderer extraction, the
//! GameObject menu (primitives, lights, cameras, models) and the built-in MCP server tools.

pub mod app;
pub mod bus;
pub mod engine;
pub mod extract;
pub mod gui;
pub mod input_map;
pub mod mcp_animation;
pub mod mcp_audio;
pub mod mcp_network;
pub mod mcp_occlusion;
pub mod mcp_physics;
pub mod mcp_prefabs;
pub mod mcp_streaming;
pub mod mcp_terrain;
pub mod mcp_tools;
pub mod objects;
pub mod streaming;

pub use app::{App, AppControl, EngineConfig, McpToolsFn};
pub use engine::{Engine, GameViewport, Plugin, Shell, init_world};

pub use oxide_animation as animation;
pub use oxide_assets as assets;
pub use oxide_audio as audio;
pub use oxide_core as core;
pub use oxide_input as input;
pub use oxide_math as math;
pub use oxide_network as network;
pub use oxide_particles as particles;
pub use oxide_physics as physics;
pub use oxide_render as render;
pub use oxide_scene as scene;
pub use oxide_standard as standard;
pub use oxide_terrain as terrain;
pub use oxide_ui as ui;

/// Physics (Rapier) with Unity's components and `Physics` API.
pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn build(
        &self,
        world: &mut oxide_scene::World,
        systems: &mut Vec<Box<dyn oxide_scene::System>>,
    ) {
        systems.push(oxide_physics::install(world));
    }
}

/// Audio (kira): AudioSource, AudioListener, built-in clips.
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(
        &self,
        world: &mut oxide_scene::World,
        systems: &mut Vec<Box<dyn oxide_scene::System>>,
    ) {
        systems.push(oxide_audio::install(world));
    }
}

/// Animation: Animator state machines and legacy Animation.
pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(
        &self,
        world: &mut oxide_scene::World,
        systems: &mut Vec<Box<dyn oxide_scene::System>>,
    ) {
        systems.push(oxide_animation::install(world));
    }
}

/// Game UI: Canvas components and immediate-mode GUI (egui).
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(
        &self,
        world: &mut oxide_scene::World,
        systems: &mut Vec<Box<dyn oxide_scene::System>>,
    ) {
        systems.push(oxide_ui::install(world));
    }
}

/// Terrain: heightmap terrain, splat layers, trees, collider.
pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(
        &self,
        world: &mut oxide_scene::World,
        systems: &mut Vec<Box<dyn oxide_scene::System>>,
    ) {
        systems.push(oxide_terrain::install(world));
    }
}

/// Particle systems.
pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(
        &self,
        world: &mut oxide_scene::World,
        systems: &mut Vec<Box<dyn oxide_scene::System>>,
    ) {
        systems.push(oxide_particles::install(world));
    }
}

/// Multiplayer: NetworkManager, NetworkObject, NetworkTransform, RPCs.
pub struct NetworkPlugin;

impl Plugin for NetworkPlugin {
    fn build(
        &self,
        world: &mut oxide_scene::World,
        systems: &mut Vec<Box<dyn oxide_scene::System>>,
    ) {
        systems.push(oxide_network::install(world));
    }
}

/// Standard assets: car controller, cameras, character controllers.
pub struct StandardAssetsPlugin;

impl Plugin for StandardAssetsPlugin {
    fn build(&self, world: &mut oxide_scene::World, _: &mut Vec<Box<dyn oxide_scene::System>>) {
        oxide_standard::register(&mut world.registry);
    }
}

/// Subsystems installed into every App.
pub fn default_plugins() -> Vec<Box<dyn Plugin>> {
    vec![
        Box::new(PhysicsPlugin),
        Box::new(AnimationPlugin),
        Box::new(AudioPlugin),
        Box::new(UiPlugin),
        Box::new(TerrainPlugin),
        Box::new(ParticlesPlugin),
        Box::new(NetworkPlugin),
        Box::new(StandardAssetsPlugin),
    ]
}

/// Everything a game script usually needs.
pub mod prelude {
    pub use crate::objects::{
        color_material, create_camera, create_directional_light, create_point_light,
        create_primitive, instantiate_model, set_material,
    };
    pub use crate::{App, AppControl};
    pub use oxide_animation::{
        Animation, Animator, AnimatorStateInfo, AnimatorUpdateMode, AvatarIKGoal, AvatarIKHint,
        HumanBodyBones,
    };
    pub use oxide_assets::{
        AnimationClip, AnimationEvent, AnimatorController, AnimatorControllerLayer,
        AnimatorLayerBlendingMode, AvatarMask, BlendTree2DType, Condition, ConditionMode, Motion,
        ParameterType,
    };
    pub use oxide_assets::{
        AssetServer, Handle, Material, Mesh, PrimitiveType, ShaderKind, SurfaceType, Texture2D,
    };
    pub use oxide_assets::{TerrainData, TerrainLayer, TreeInstance, TreePrototype};
    pub use oxide_audio::{Audio, AudioExt, AudioListener, AudioSource};
    pub use oxide_network::{
        HOST_CLIENT_ID, NetMode, Network, NetworkExt, NetworkManager, NetworkObject,
        NetworkTransform,
    };
    pub use oxide_particles::{
        BlendMode, ParticleShape, ParticleSystem, ParticlesExt, SimulationSpace,
    };
    pub use oxide_terrain::{Terrain, create_terrain, sample_height, simple_tree_mesh};
    pub use oxide_ui::{
        Anchor, Canvas, Gui, GuiExt, TextAlign, UIButton, UIImage, UISlider, UIText, UIToggle, egui,
    };

    pub use oxide_core::{Reflect, Time, debug_log, debug_log_error, debug_log_warning};
    pub use oxide_input::{
        CursorLockMode, GamepadAxis, GamepadButton, Input, KeyCode, MouseButton,
    };
    pub use oxide_math::prelude::*;
    pub use oxide_physics::{
        BoxCollider, CapsuleCollider, CharacterController, CharacterExt, CharacterJoint,
        CollisionDetectionMode, DEFAULT_RAYCAST_LAYERS, FixedJoint, ForceMode, HingeJoint,
        MeshCollider, Physics, PhysicsExt, RaycastHit, Rigidbody, RigidbodyInterpolation,
        SphereCollider, TerrainCollider, WheelCollider,
    };
    pub use oxide_scene::{
        Camera, CameraClearFlags, Collision, Component, Ctx, GameObjectId, Light, LightShadows,
        LightType, MeshFilter, MeshRenderer, OcclusionPortal, Projection, RenderSettings,
        ShadowCastingMode, SkinnedMeshRenderer, Space, StaticEditorFlags, World,
    };
    pub use oxide_standard::*;
}

pub mod json_values;

/// Loads a scene file into the World now: replacing the open scenes, or as a new additive
/// scene. Returns warnings (e.g. missing script types). For background loading see
/// [`World::load_scene_async`](oxide_scene::World::load_scene_async).
pub fn load_scene(
    world: &mut oxide_scene::World,
    path: &str,
    additive: bool,
) -> Result<Vec<String>, String> {
    let full = world
        .resource::<oxide_assets::AssetServer>()
        .full_path(path);
    let text = std::fs::read_to_string(&full)
        .map_err(|e| format!("cannot read {}: {e}", full.display()))?;
    let file = oxide_scene::SceneFile::from_yaml(&text)?;
    let (_, warnings) = world.activate_scene_file(&file, path, additive);
    for w in &warnings {
        oxide_core::debug_log_warning!("{w}");
    }
    Ok(warnings)
}

/// Saves the main scene as YAML to `path` (relative to the project) and makes it the
/// open scene's path. Returns the number of objects written.
pub fn save_scene(world: &mut oxide_scene::World, path: &str) -> Result<usize, String> {
    save_scene_as(world, oxide_scene::SceneHandle::MAIN, path)
}

/// Saves one loaded scene (main or additive) to `path`. Returns the number of objects
/// written.
pub fn save_scene_as(
    world: &mut oxide_scene::World,
    scene: oxide_scene::SceneHandle,
    path: &str,
) -> Result<usize, String> {
    let name = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Scene")
        .to_owned();
    let mut file = world.serialize_scene(scene, &name);
    use oxide_core::reflect::Reflect;
    for w in world.persist_runtime_assets(&mut file, &oxide_scene::prefab::asset_folder(path)) {
        oxide_core::debug_log_warning!("{w}");
    }
    file.prefabs = world.prefab_snapshots();
    file.render_settings = world
        .resource::<oxide_scene::RenderSettings>()
        .fields()
        .into_iter()
        .filter(|(k, _)| *k != "sun")
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    let full = world
        .resource::<oxide_assets::AssetServer>()
        .full_path(path);
    if let Some(dir) = full.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let yaml = file.to_yaml()?;
    std::fs::write(&full, yaml).map_err(|e| format!("cannot write {}: {e}", full.display()))?;
    world.set_scene_path(scene, &name, path);
    Ok(file.objects.len())
}
