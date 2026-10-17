//! MCP tool for Video Players.

use crate::bus::Bus;
use crate::json_values::resolve_object;
use crate::mcp_tools::{schema, tool};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_video::VideoPlayer;
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "video",
            "Video Players",
            "Lists every VideoPlayer (clip or URL, render mode, prepared, playing, paused, time, frame, \
             frame count, length, size, error). With `object`: `action` play | pause | stop | prepare, \
             `time` (seconds) or `frame` to seek, `looping`, `speed`.",
            schema(
                json!({
                    "object": {}, "action": { "type": "string" }, "time": { "type": "number" },
                    "frame": { "type": "integer" }, "looping": { "type": "boolean" },
                    "speed": { "type": "number" }
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
                let Some(p) = w.get_component_mut::<VideoPlayer>(id) else {
                    return ToolResult::error("that object has no Video Player");
                };
                match args.get("action").and_then(Json::as_str) {
                    Some("play") => p.play(),
                    Some("pause") => p.pause(),
                    Some("stop") => p.stop(),
                    Some("prepare") => p.prepare(),
                    Some(other) => return ToolResult::error(format!("unknown action `{other}`")),
                    None => {}
                }
                if let Some(t) = args.get("time").and_then(Json::as_f64) {
                    p.set_time(t);
                }
                if let Some(f) = args.get("frame").and_then(Json::as_i64) {
                    p.set_frame(f);
                }
                if let Some(l) = args.get("looping").and_then(Json::as_bool) {
                    p.is_looping = l;
                }
                if let Some(s) = args.get("speed").and_then(Json::as_f64) {
                    p.playback_speed = s as f32;
                }
            }
            let players: Vec<Json> = w
                .query::<VideoPlayer>()
                .map(|(id, p)| {
                    let s = &p.status;
                    json!({
                        "object": id.to_bits(),
                        "name": w.get(id).map(|o| o.name.clone()),
                        "clip": p.clip.as_ref().map(|h| h.path().to_owned()),
                        "url": p.url,
                        "render_mode": format!("{:?}", p.render_mode),
                        "texture": oxide_video::texture_path(id),
                        "prepared": s.is_prepared, "playing": s.is_playing, "paused": s.is_paused,
                        "time": s.time, "frame": s.frame, "frame_count": s.frame_count,
                        "length": s.length, "width": s.width, "height": s.height,
                        "error": s.error,
                    })
                })
                .collect();
            ToolResult::json(json!({ "players": players }))
        },
    );
}
