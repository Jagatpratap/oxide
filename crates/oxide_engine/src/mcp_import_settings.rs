//! MCP tool for Model and Audio import settings (textures: `texture_import_settings`).

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool};
use oxide_assets::AssetServer;
use oxide_assets::importers::{AudioImporter, ModelImporter};
use oxide_core::reflect::Reflect;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

fn settings_json(imp: &dyn Reflect) -> Json {
    Json::Object(
        imp.fields()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), crate::json_values::value_to_json(None, &v)))
            .collect(),
    )
}

fn apply(
    imp: &mut dyn Reflect,
    fields: &serde_json::Map<String, Json>,
    w: &oxide_scene::World,
) -> Result<(), String> {
    for (k, v) in fields {
        let info = imp
            .type_info()
            .fields
            .into_iter()
            .find(|f| f.name == k)
            .ok_or_else(|| format!("no import setting `{k}`"))?;
        let value =
            crate::json_values::json_to_value(&info, v, w).map_err(|e| format!("{k}: {e}"))?;
        imp.set_field(k, value).map_err(|e| format!("{k}: {e}"))?;
    }
    Ok(())
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "import_settings",
            "Model and audio import settings",
            "Unity's Model Importer (glb/gltf/fbx/obj: scale_factor, convert_units, generate_colliders, normals \
             Import|Calculate, smoothing_angle, swap_uvs, generate_lightmap_uvs, animation_type \
             None|Legacy|Generic|Humanoid, import_animation, loop_time, import_materials) or Audio Importer \
             (force_to_mono, normalize, sample_rate_setting PreserveSampleRate|OptimizeSampleRate|\
             OverrideSampleRate, sample_rate, preload_audio_data) of `path`, kept in its .meta. `settings` \
             changes fields and re-imports. Textures use texture_import_settings.",
            schema(
                json!({ "path": { "type": "string" }, "settings": { "type": "object" } }),
                &["path"],
            ),
        ),
        |args, e| {
            let path = args
                .get("path")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_owned();
            let fields = args.get("settings").and_then(Json::as_object).cloned();
            let model = oxide_assets::is_model_path(&path);
            let audio = e
                .world
                .resource::<AssetServer>()
                .types
                .for_path(&path)
                .is_some_and(|t| t.type_name == "AudioClip");
            if !model && !audio {
                return ToolResult::error("`path` must be a model or an audio clip");
            }
            let r = if model {
                let mut imp: ModelImporter = e.world.resource::<AssetServer>().importer(&path);
                if let Some(f) = &fields {
                    if let Err(err) = apply(&mut imp, f, &e.world) {
                        return ToolResult::error(err);
                    }
                    if let Err(err) = e
                        .world
                        .resource_mut::<AssetServer>()
                        .set_importer(&path, &imp)
                    {
                        return ToolResult::error(err);
                    }
                }
                let assets = e.world.resource_mut::<AssetServer>();
                let h = assets
                    .models
                    .handle_of(&path)
                    .map_or_else(|| assets.load_model(&path), Ok);
                let summary = h.ok().and_then(|h| assets.models.get(&h).cloned()).map(|m| {
                    let verts: usize = m
                        .nodes
                        .iter()
                        .filter_map(|n| n.mesh.as_ref())
                        .filter_map(|h| assets.meshes.get(h))
                        .map(|me| me.vertices.len())
                        .sum();
                    json!({ "nodes": m.nodes.len(), "vertices": verts, "animations": m.animations.len() })
                });
                json!({ "path": path, "type": "ModelImporter", "settings": settings_json(&imp), "imported": summary })
            } else {
                let mut imp: AudioImporter = e.world.resource::<AssetServer>().importer(&path);
                if let Some(f) = &fields {
                    if let Err(err) = apply(&mut imp, f, &e.world) {
                        return ToolResult::error(err);
                    }
                    if let Err(err) = e
                        .world
                        .resource_mut::<AssetServer>()
                        .set_importer(&path, &imp)
                    {
                        return ToolResult::error(err);
                    }
                }
                json!({ "path": path, "type": "AudioImporter", "settings": settings_json(&imp) })
            };
            ToolResult::json(r)
        },
    );
}
