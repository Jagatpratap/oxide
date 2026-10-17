//! MCP tool for Visual Effects (VFX Graph) playback and properties.

use crate::bus::Bus;
use crate::json_values::resolve_object;
use crate::mcp_tools::{schema, tool};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_scene::World;
use oxide_vfx::VisualEffect;
use serde_json::Value as Json;
use std::sync::Arc;

fn effect_json(w: &World, id: oxide_scene::GameObjectId, v: &VisualEffect) -> Json {
    let c = v.state.compiled.as_ref();
    json!({
        "object": id.to_bits(),
        "name": w.get(id).map(|o| o.name.clone()),
        "asset": v.asset.as_ref().map(|h| h.path().to_owned()),
        "playing": v.is_playing(),
        "pause": v.pause,
        "play_rate": v.play_rate,
        "time": v.state.time,
        "spawned": v.state.spawned,
        "systems": c.map(|c| c.systems.iter().map(|s| json!({
            "capacity": s.program.capacity,
            "blend": format!("{:?}", s.program.blend),
            "local_space": s.program.local_space,
            "spawn_blocks": s.spawn.iter().map(|b| b.kind.clone()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>()),
        "properties": c.map(|c| c.properties.iter().map(|p| json!({
            "name": p.name,
            "reference": p.reference,
            "type": p.kind.kind_name(),
            "value": v.get_vector(&p.reference).map(|x| x.to_array()),
            "default": p.default.to_array(),
        })).collect::<Vec<_>>()),
        "errors": c.map(|c| c.errors.iter().map(|(n, e)| json!({"node": n, "error": e})).collect::<Vec<_>>()),
    })
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "visual_effect",
            "Visual Effects",
            "Lists every Visual Effect (VFX Graph): asset, playing, time, particles spawned, systems, exposed \
             properties and compile errors. With `object`: `action` play | stop | pause | resume | reinit | \
             send_event (with `event`); `set` {reference: number | [x,y,z,w] | bool} overrides exposed \
             properties (Unity SetFloat/SetVector/SetBool); `reset` [references] removes overrides; \
             `play_rate`; `asset` assigns a .vfx. Effects also play in Edit mode in the editor.",
            schema(
                json!({
                    "object": {}, "action": { "type": "string", "enum": ["play", "stop", "pause", "resume", "reinit", "send_event"] },
                    "event": { "type": "string" }, "set": { "type": "object" }, "reset": { "type": "array" },
                    "play_rate": { "type": "number" }, "asset": { "type": "string" }
                }),
                &[],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            if let Some(o) = args.get("object") {
                let Some(id) = resolve_object(w, o) else {
                    return ToolResult::error(format!("no GameObject matches {o}"));
                };
                let Some(v) = w.get_component_mut::<VisualEffect>(id) else {
                    return ToolResult::error("that object has no Visual Effect");
                };
                if let Some(a) = args.get("asset").and_then(Json::as_str) {
                    v.asset = Some(oxide_assets::Handle::from_path(a.to_owned()));
                    v.reinit();
                }
                if let Some(r) = args.get("play_rate").and_then(Json::as_f64) {
                    v.play_rate = r as f32;
                }
                if let Some(set) = args.get("set").and_then(Json::as_object) {
                    for (k, val) in set {
                        let x = match val {
                            Json::Number(n) => oxide_math::Vector4::new(
                                n.as_f64().unwrap_or(0.0) as f32,
                                0.0,
                                0.0,
                                0.0,
                            ),
                            Json::Bool(b) => {
                                oxide_math::Vector4::new(*b as u32 as f32, 0.0, 0.0, 0.0)
                            }
                            Json::Array(a) => {
                                let mut o = [0.0f32; 4];
                                for (i, x) in a.iter().take(4).enumerate() {
                                    o[i] = x.as_f64().unwrap_or(0.0) as f32;
                                }
                                oxide_math::Vector4::from_array(o)
                            }
                            _ => {
                                return ToolResult::error(format!(
                                    "set.{k}: a number, bool or [x, y, z, w]"
                                ));
                            }
                        };
                        v.set_vector(k, x);
                    }
                }
                if let Some(list) = args.get("reset").and_then(Json::as_array) {
                    for r in list.iter().filter_map(Json::as_str) {
                        v.reset_override(r);
                    }
                }
                match args.get("action").and_then(Json::as_str) {
                    Some("play") => {
                        v.pause = false;
                        v.play();
                    }
                    Some("stop") => v.stop(),
                    Some("pause") => v.pause = true,
                    Some("resume") => v.pause = false,
                    Some("reinit") => v.reinit(),
                    Some("send_event") => {
                        let ev = args
                            .get("event")
                            .and_then(Json::as_str)
                            .unwrap_or("OnPlay")
                            .to_owned();
                        v.send_event(&ev);
                    }
                    _ => {}
                }
                let v = w.get_component::<VisualEffect>(id).expect("still there");
                return ToolResult::json(effect_json(w, id, v));
            }
            let list: Vec<Json> = w
                .query::<VisualEffect>()
                .map(|(id, v)| effect_json(w, id, v))
                .collect();
            ToolResult::json(json!(list))
        },
    );
}
