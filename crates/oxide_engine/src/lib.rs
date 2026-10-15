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
pub mod mcp_physics;
pub mod mcp_terrain;
pub mod mcp_tools;
pub mod objects;

pub use app::{App, AppControl, EngineConfig};
pub use engine::{Engine, Plugin, init_world};

pub use oxide_animation as animation;
pub use oxide_assets as assets;
pub use oxide_audio as audio;
pub use oxide_core as core;
pub use oxide_input as input;
pub use oxide_math as math;
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
    pub use oxide_animation::{Animation, Animator};
    pub use oxide_assets::{
        AnimationClip, AnimatorController, Condition, ConditionMode, Motion, ParameterType,
    };
    pub use oxide_assets::{
        AssetServer, Handle, Material, Mesh, PrimitiveType, ShaderKind, SurfaceType, Texture2D,
    };
    pub use oxide_assets::{TerrainData, TerrainLayer, TreeInstance, TreePrototype};
    pub use oxide_audio::{Audio, AudioExt, AudioListener, AudioSource};
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
        BoxCollider, CapsuleCollider, CharacterController, CharacterExt, CollisionDetectionMode,
        DEFAULT_RAYCAST_LAYERS, FixedJoint, ForceMode, HingeJoint, MeshCollider, Physics,
        PhysicsExt, RaycastHit, Rigidbody, RigidbodyInterpolation, SphereCollider, TerrainCollider,
        WheelCollider,
    };
    pub use oxide_scene::{
        Camera, CameraClearFlags, Collision, Component, Ctx, GameObjectId, Light, LightShadows,
        LightType, MeshFilter, MeshRenderer, Projection, RenderSettings, ShadowCastingMode,
        SkinnedMeshRenderer, Space, World,
    };
    pub use oxide_standard::*;
}

pub mod json_values;

/// Loads a scene file into the World (replacing the scene unless `additive`).
/// Returns warnings (e.g. missing script types).
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
    // Make sure referenced assets are loaded.
    let mut warnings = Vec::new();
    for o in &file.objects {
        for c in &o.components {
            for v in c.fields.values() {
                collect_assets(world, v, &mut warnings);
            }
        }
    }
    if !additive {
        world.clear(true);
        world.scene_name = file.name.clone();
        world.scene_path = Some(path.to_owned());
    }
    let (_, w) = world.spawn(&file, None);
    warnings.extend(w);
    for w in &warnings {
        oxide_core::debug_log_warning!("{w}");
    }
    Ok(warnings)
}

fn collect_assets(
    world: &mut oxide_scene::World,
    v: &oxide_core::reflect::Value,
    warnings: &mut Vec<String>,
) {
    use oxide_core::reflect::Value;
    match v {
        Value::Asset(p) => {
            if let Err(e) = world
                .resource_mut::<oxide_assets::AssetServer>()
                .ensure_loaded(p)
            {
                warnings.push(e);
            }
        }
        Value::List(items) => items
            .iter()
            .for_each(|i| collect_assets(world, i, warnings)),
        _ => {}
    }
}
