//! MCP tools for rendering features: reflection probes.

use crate::bus::Bus;
use crate::mcp_tools::{obj_arg, schema, tool};
use crate::tri;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_scene::ReflectionProbe;
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "texture_import_settings",
            "Texture import settings",
            "Gets a texture's import settings (its `.meta` file, Unity's Texture Importer) and what the import \
             produced (size, mips, GPU format). With `settings` (any of texture_type Default|NormalMap, srgb, \
             max_size, generate_mip_maps, wrap_mode, filter_mode, aniso_level, compression \
             None|LowQuality|NormalQuality|HighQuality), writes the .meta and re-imports. Compression is BC7 for \
             color and BC5 for normal maps (BC1/BC3 for LowQuality), cached in Library/TextureCache.",
            schema(
                json!({ "path": { "type": "string" }, "settings": { "type": "object" } }),
                &["path"],
            ),
        ),
        |args, e| {
            let path = args
                .get("path")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            let assets = e.world.resource_mut::<oxide_assets::AssetServer>();
            if let Some(Json::Object(fields)) = args.get("settings") {
                let mut imp = assets.texture_importer(&path);
                for (k, v) in fields {
                    let Some(info) = oxide_core::reflect::Reflect::type_info(&imp)
                        .fields
                        .into_iter()
                        .find(|f| f.name == k)
                    else {
                        return ToolResult::error(format!("no import setting `{k}`"));
                    };
                    let value = match crate::json_values::json_to_value(&info, v, &e.world) {
                        Ok(v) => v,
                        Err(err) => return ToolResult::error(format!("{k}: {err}")),
                    };
                    if let Err(err) = oxide_core::reflect::Reflect::set_field(&mut imp, k, value) {
                        return ToolResult::error(format!("{k}: {err}"));
                    }
                }
                let assets = e.world.resource_mut::<oxide_assets::AssetServer>();
                if let Err(err) = assets.set_texture_importer(&path, &imp) {
                    return ToolResult::error(err);
                }
            } else if assets.textures.handle_of(&path).is_none()
                && let Err(err) = assets.load_texture(&path)
            {
                return ToolResult::error(err);
            }
            let assets = e.world.resource::<oxide_assets::AssetServer>();
            let imp = assets.texture_importer(&path);
            let settings: serde_json::Map<String, Json> =
                oxide_core::reflect::Reflect::fields(&imp)
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), crate::json_values::value_to_json(None, &v)))
                    .collect();
            let tex = assets
                .textures
                .handle_of(&path)
                .and_then(|h| assets.textures.get(&h));
            ToolResult::json(json!({
                "path": path,
                "settings": settings,
                "imported": tex.map(|t| json!({
                    "width": t.width,
                    "height": t.height,
                    "mips": t.compressed.as_ref().map_or(t.mips.len(), |c| c.mips.len()),
                    "gpu_format": t.compressed.as_ref().map_or("RGBA8".to_owned(), |c| format!("{:?}", c.format)),
                    "gpu_bytes": t.compressed.as_ref().map_or(
                        t.mips.iter().map(Vec::len).sum::<usize>(),
                        |c| c.mips.iter().map(Vec::len).sum::<usize>(),
                    ),
                })),
            }))
        },
    );
    tool(
        s,
        bus,
        ToolDef::new(
            "create_render_texture",
            "Create Render Texture",
            "Creates a `.renderTexture` asset (Unity RenderTexture) at `path` (e.g. \"Assets/Mirror.renderTexture\"). \
             Set a Camera's `target_texture` to it to render into it instead of the screen, and use the same path \
             in a material's `base_map` / `emission_map` (mirrors, screens, scopes).",
            schema(
                json!({
                    "path": { "type": "string" },
                    "width": { "type": "integer", "minimum": 1, "maximum": 8192 },
                    "height": { "type": "integer", "minimum": 1, "maximum": 8192 }
                }),
                &["path"],
            ),
        ),
        |args, e| {
            let path = args.get("path").and_then(Json::as_str).unwrap_or_default();
            if !path.to_ascii_lowercase().ends_with(".rendertexture") {
                return ToolResult::error("`path` must end in .renderTexture");
            }
            let size = |k: &str| args.get(k).and_then(Json::as_u64).unwrap_or(256) as u32;
            let rt = oxide_assets::RenderTexture {
                name: std::path::Path::new(path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("RenderTexture")
                    .to_owned(),
                width: size("width").clamp(1, 8192),
                height: size("height").clamp(1, 8192),
            };
            let assets = e.world.resource_mut::<oxide_assets::AssetServer>();
            match assets.save_render_texture(path, &rt) {
                Ok(_) => ToolResult::json(
                    json!({ "path": path, "width": rt.width, "height": rt.height }),
                ),
                Err(err) => ToolResult::error(err),
            }
        },
    );
    tool(
        s,
        bus,
        ToolDef::new(
            "reflection_probes",
            "Reflection probes",
            "Lists Reflection Probes (box, mode, refresh mode, importance) and the last frame's reflection stats \
             (`active` = probes with a captured cubemap, up to 15; `captures` = cubemaps captured that frame, sky \
             included). `render` = an object with a probe: capture it again next frame (Unity `RenderProbe()`, the \
             only trigger for ViaScripting probes). Add probes with add_component \"ReflectionProbe\"; renderers \
             inside a probe's box reflect it, the sky elsewhere.",
            schema(
                json!({ "render": { "description": "Object id, name or path" } }),
                &[],
            ),
        ),
        |args, e| {
            if args.contains_key("render") {
                let id = tri!(obj_arg(args, &e.world, "render"));
                match e.world.get_component_mut::<ReflectionProbe>(id) {
                    Some(p) => p.render_probe(),
                    None => return ToolResult::error("that object has no ReflectionProbe"),
                }
            }
            let probes: Vec<Json> = e
                .world
                .query::<ReflectionProbe>()
                .map(|(id, p)| {
                    let (min, max) = p.bounds(e.world.position(id));
                    json!({
                        "id": id.to_bits(),
                        "name": e.world.get(id).map(|o| o.name.clone()),
                        "mode": format!("{:?}", p.mode),
                        "refresh_mode": format!("{:?}", p.refresh_mode),
                        "importance": p.importance,
                        "intensity": p.intensity,
                        "box_projection": p.box_projection,
                        "box_min": min.to_array(),
                        "box_max": max.to_array(),
                    })
                })
                .collect();
            let s = e.renderer.last_stats;
            ToolResult::json(json!({
                "probes": probes,
                "last_frame": { "active": s.reflection_probes, "captures": s.probe_captures },
            }))
        },
    );
}
