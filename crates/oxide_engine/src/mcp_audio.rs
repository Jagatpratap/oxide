//! MCP tools for audio.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool, vec3_arg};
use crate::tri;
use oxide_assets::{AssetServer, AudioClip, Handle};
use oxide_audio::Audio;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "audio_info",
            "Audio info",
            "Audio device availability, playing voices, master volume and loaded clips. Optional `master_volume` (0-1) and `paused` change them.",
            schema(
                json!({ "master_volume": { "type": "number", "minimum": 0, "maximum": 1 }, "paused": { "type": "boolean" } }),
                &[],
            ),
        ),
        |args, e| {
            let clips: Vec<String> = e
                .world
                .resource::<AssetServer>()
                .audio
                .iter()
                .map(|(p, _)| p.to_owned())
                .collect();
            let a = e.world.resource_mut::<Audio>();
            if let Some(v) = args.get("master_volume").and_then(Json::as_f64) {
                a.master_volume = v as f32
            }
            if let Some(p) = args.get("paused").and_then(Json::as_bool) {
                a.paused = p
            }
            ToolResult::json(
                json!({ "available": a.is_available(), "voices": a.active_voices(),
                "master_volume": a.master_volume, "paused": a.paused, "clips": clips }),
            )
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "play_sound",
            "Play sound",
            "Plays an audio clip once: at a world `position` (3D, like AudioSource.PlayClipAtPoint) or 2D if omitted. \
         Clips: asset paths (Assets/...wav/ogg/mp3/flac) or built-ins builtin:engine, builtin:gunshot, builtin:beep, builtin:wind, builtin:skid.",
            schema(
                json!({ "clip": { "type": "string" }, "position": { "type": "array" }, "volume": { "type": "number" } }),
                &["clip"],
            ),
        ),
        |args, e| {
            let clip = args
                .get("clip")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            if let Err(err) = e.world.resource_mut::<AssetServer>().ensure_loaded(&clip) {
                return ToolResult::error(err);
            }
            let h = Handle::<AudioClip>::from_path(clip.clone());
            if !e.world.resource::<AssetServer>().audio.contains(&h) {
                return ToolResult::error(format!("no audio clip `{clip}`"));
            }
            let vol = args.get("volume").and_then(Json::as_f64).unwrap_or(1.0) as f32;
            let pos = tri!(vec3_arg(args, "position"));
            let mut audio = e
                .world
                .resources
                .remove::<Audio>()
                .expect("audio installed");
            match pos {
                Some(p) => audio.play_clip_at_point(e.world.resource::<AssetServer>(), &h, p, vol),
                None => audio.play_2d(e.world.resource::<AssetServer>(), &h, vol),
            }
            let ok = audio.is_available();
            e.world.resources.insert(audio);
            ToolResult::text(if ok {
                format!("Playing {clip}.")
            } else {
                "Audio device unavailable; nothing played.".into()
            })
        },
    );
}
