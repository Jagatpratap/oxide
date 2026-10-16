//! MCP tool for Timeline playback (PlayableDirector).

use crate::bus::Bus;
use crate::json_values::resolve_object;
use crate::mcp_tools::{schema, tool};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_scene::World;
use oxide_timeline::{PlayState, PlayableDirector};
use serde_json::Value as Json;
use std::sync::Arc;

fn director_json(w: &World, id: oxide_scene::GameObjectId, d: &PlayableDirector) -> Json {
    json!({
        "object": id.to_bits(),
        "name": w.get(id).map(|o| o.name.clone()),
        "asset": d.playable_asset.as_ref().map(|h| h.path().to_owned()),
        "state": match d.state() { PlayState::Playing => "Playing", PlayState::Paused => "Paused", PlayState::Stopped => "Stopped" },
        "time": d.time(),
        "duration": d.duration(),
        "wrap_mode": format!("{:?}", d.wrap_mode),
        "bindings": d.bindings.iter().map(|b| json!({
            "track": b.track,
            "object": b.object.and_then(|o| w.get(o).map(|x| x.name.clone())),
        })).collect::<Vec<_>>(),
    })
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "timeline",
            "Play Timelines",
            "Lists every PlayableDirector (asset, state, time, duration, bindings), or controls one: \
             `object` is the Director, `action` is play | pause | resume | stop | evaluate, `time` sets \
             PlayableDirector.time (seconds; with `evaluate` it applies at once). Playback runs in Play \
             mode; in Edit mode use editor_timeline to preview.",
            schema(
                json!({ "object": {}, "action": { "type": "string", "enum": ["play", "pause", "resume", "stop", "evaluate"] }, "time": { "type": "number" } }),
                &[],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            if let Some(v) = args.get("object") {
                let Some(id) = resolve_object(w, v) else {
                    return ToolResult::error(format!("no GameObject matches {v}"));
                };
                let Some(d) = w.get_component_mut::<PlayableDirector>(id) else {
                    return ToolResult::error("that object has no PlayableDirector");
                };
                if let Some(t) = args.get("time").and_then(Json::as_f64) {
                    d.set_time(t);
                }
                match args.get("action").and_then(Json::as_str) {
                    Some("play") => d.play(),
                    Some("pause") => d.pause(),
                    Some("resume") => d.resume(),
                    Some("stop") => d.stop(),
                    Some("evaluate") => d.evaluate(),
                    Some(a) => return ToolResult::error(format!("unknown action `{a}`")),
                    None => {}
                }
                let d = w
                    .get_component::<PlayableDirector>(id)
                    .expect("checked")
                    .clone();
                return ToolResult::json(director_json(w, id, &d));
            }
            let all: Vec<Json> = w
                .query::<PlayableDirector>()
                .map(|(id, d)| director_json(w, id, d))
                .collect();
            ToolResult::json(json!({ "directors": all }))
        },
    );
}
