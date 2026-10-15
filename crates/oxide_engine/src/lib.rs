//! Oxide runtime: the window, Unity's player loop, scene → renderer extraction, the
//! GameObject menu (primitives, lights, cameras, models) and the built-in MCP server tools.

pub mod app;
pub mod bus;
pub mod engine;
pub mod extract;
pub mod input_map;
pub mod mcp_tools;
pub mod objects;

pub use app::{App, AppControl, EngineConfig};
pub use engine::{Engine, Plugin, init_world};

pub use oxide_assets as assets;
pub use oxide_core as core;
pub use oxide_input as input;
pub use oxide_math as math;
pub use oxide_render as render;
pub use oxide_scene as scene;

/// Subsystems installed into every App (physics, audio, animation as they are added).
pub fn default_plugins() -> Vec<Box<dyn Plugin>> {
    Vec::new()
}

/// Everything a game script usually needs.
pub mod prelude {
    pub use crate::objects::{
        color_material, create_camera, create_directional_light, create_point_light,
        create_primitive, instantiate_model, set_material,
    };
    pub use crate::{App, AppControl};
    pub use oxide_assets::{
        AssetServer, Handle, Material, Mesh, PrimitiveType, ShaderKind, SurfaceType, Texture2D,
    };
    pub use oxide_core::{Reflect, Time, debug_log, debug_log_error, debug_log_warning};
    pub use oxide_input::{
        CursorLockMode, GamepadAxis, GamepadButton, Input, KeyCode, MouseButton,
    };
    pub use oxide_math::prelude::*;
    pub use oxide_scene::{
        Camera, CameraClearFlags, Collision, Component, Ctx, GameObjectId, Light, LightShadows,
        LightType, MeshFilter, MeshRenderer, Projection, RenderSettings, ShadowCastingMode,
        SkinnedMeshRenderer, Space, World,
    };
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
