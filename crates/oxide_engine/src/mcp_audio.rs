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

    tool(
        s,
        bus,
        ToolDef::new(
            "audio_mixer",
            "Audio Mixer",
            "An Audio Mixer at runtime (Unity AudioMixer): `set` {\"MusicVol\": -10} sets exposed parameters \
             (dB for volumes), `clear` removes script values, `snapshot` + `seconds` transitions to a snapshot. \
             Returns the mixer's groups, snapshots, exposed parameters and the values it plays at now.",
            schema(
                json!({ "mixer": { "type": "string" }, "set": { "type": "object" }, "clear": { "type": "array" },
                        "snapshot": { "type": "string" }, "seconds": { "type": "number" } }),
                &["mixer"],
            ),
        ),
        |args, e| {
            use oxide_audio::mixer::AudioMixer;
            let path = args
                .get("mixer")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_owned();
            let h: Handle<AudioMixer> = Handle::from_path(path.clone());
            let Some(m) = e
                .world
                .resource_mut::<AssetServer>()
                .data_asset(&h)
                .cloned()
            else {
                return ToolResult::error(format!("cannot load mixer {path}"));
            };
            let mut audio = e.world.resources.remove::<Audio>().expect("Audio resource");
            if let Some(set) = args.get("set").and_then(Json::as_object) {
                for (k, v) in set {
                    if let Some(x) = v.as_f64() {
                        audio.set_float(&path, k, x as f32);
                    }
                }
            }
            for k in args
                .get("clear")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(k) = k.as_str() {
                    audio.clear_float(&path, k);
                }
            }
            if let Some(snap) = args.get("snapshot").and_then(Json::as_str) {
                if !m.snapshots.iter().any(|s| s.name == snap) {
                    e.world.resources.insert(audio);
                    return ToolResult::error(format!("no snapshot `{snap}`"));
                }
                let secs = args.get("seconds").and_then(Json::as_f64).unwrap_or(0.0) as f32;
                audio.transition_to(&path, snap, secs, e.world.resource::<AssetServer>());
            }
            let st = audio.mixer_states.entry(path.clone()).or_default().clone();
            e.world.resources.insert(audio);
            let values = st.values(&m);
            let groups: Vec<Json> = m
                .groups
                .iter()
                .map(|g| json!({ "name": g.name, "parent": g.parent, "volume_db": m.effective_volume(&g.name, &values),
                                 "pitch": m.pitch(&g.name, &values), "mute": g.mute, "solo": g.solo,
                                 "effects": g.effects.iter().map(|x| format!("{:?}", x.kind)).collect::<Vec<_>>() }))
                .collect();
            ToolResult::json(json!({
                "groups": groups,
                "snapshot": st.snapshot,
                "transitioning": st.transition.is_some(),
                "snapshots": m.snapshots.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
                "exposed": m.exposed_parameters.iter().map(|x| json!({ "name": x.name, "parameter": x.parameter, "value": values.get(&x.parameter) })).collect::<Vec<_>>(),
            }))
        },
    );
}
