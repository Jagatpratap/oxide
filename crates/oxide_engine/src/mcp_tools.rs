//! The engine's MCP tools. Each tool parses its arguments on the HTTP thread, then runs on
//! the engine thread through the [`Bus`].
//!
//! Naming follows Unity concepts so agents can transfer Unity knowledge: the Console,
//! `Time`, Play-mode Pause/Step, objects with Inspector fields.

use crate::app::Engine;
use crate::bus::Bus;
use oxide_core::debug::{self, LogType};
use oxide_core::reflect::{Reflect, Value, ValueKind};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

const INSTRUCTIONS: &str = "\
Oxide is a Rust game engine modeled on Unity (left-handed, +Y up, +Z forward, angles in degrees, \
colors as sRGB 0-1 like Unity's color picker). These tools drive the running engine. \
Read state with get_engine_info, get_time, get_console_logs, list_objects and inspect_object; \
change it with set_object_field, set_time and set_paused/step_frames; \
look at the result with capture_screenshot. Check get_console_logs (min_type=Warning) after changes.";

const OBJECTS: &[&str] = &["Main Camera", "Triangle"];

fn object<'a>(engine: &'a mut Engine, name: &str) -> Option<&'a mut dyn Reflect> {
    match name.to_ascii_lowercase().as_str() {
        "main camera" | "camera" => Some(&mut engine.camera),
        "triangle" => Some(&mut engine.triangle),
        _ => None,
    }
}

/// Builds the server with every engine tool registered.
pub fn build_server(bus: Arc<Bus>) -> McpServer {
    let mut s = McpServer::new("oxide", oxide_core::ENGINE_VERSION, INSTRUCTIONS);
    let empty = json!({ "type": "object", "properties": {}, "additionalProperties": false });

    let b = bus.clone();
    s.add_tool(
        ToolDef::new(
            "get_engine_info",
            "Get engine info",
            "Engine version, GPU and graphics backend, window size, frame rate, play/pause state.",
            empty.clone(),
        )
        .read_only(),
        move |_| {
            b.run(|e| {
                let gpu = e.gpu.adapter_info();
                let (w, h) = e.surface.size();
                let (logs, warnings, errors) = debug::counts();
                ToolResult::json(json!({
                    "engine": "Oxide",
                    "version": oxide_core::ENGINE_VERSION,
                    "phase": "P0 (foundation)",
                    "gpu": { "name": gpu.name, "backend": format!("{:?}", gpu.backend),
                             "device_type": format!("{:?}", gpu.device_type), "driver": gpu.driver },
                    "window": { "width": w, "height": h, "scale_factor": e.window.scale_factor(),
                                "surface_format": format!("{:?}", e.surface.format()) },
                    "fps": e.fps,
                    "paused": e.paused,
                    "frame_count": e.time.frame_count(),
                    "console": { "logs": logs, "warnings": warnings, "errors": errors },
                    "mcp_url": e.mcp_url,
                }))
            })
        },
    );

    s.add_tool(
        ToolDef::new(
            "get_console_logs",
            "Read the Console",
            "Console messages (Debug.Log/LogWarning/LogError, plus GPU and window-system warnings), \
             oldest first. Pass the returned `last_seq` as `after_seq` next time to get only new ones.",
            json!({
                "type": "object",
                "properties": {
                    "after_seq": { "type": "integer", "minimum": 0, "description": "Only entries newer than this sequence number." },
                    "min_type": { "type": "string", "enum": ["Log", "Warning", "Error", "Exception"], "description": "Minimum severity (default Log)." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "description": "Most recent N matches (default 100)." }
                },
                "additionalProperties": false
            }),
        )
        .read_only(),
        |args| {
            let after = args.get("after_seq").and_then(Json::as_u64).unwrap_or(0);
            let min_type = match args.get("min_type").and_then(Json::as_str) {
                Some(s) => match LogType::parse(s) {
                    Some(t) => t,
                    None => return ToolResult::error(format!("unknown min_type `{s}`")),
                },
                None => LogType::Log,
            };
            let limit = args.get("limit").and_then(Json::as_u64).unwrap_or(100).clamp(1, 1000) as usize;
            let entries = debug::entries_since(after, min_type, limit);
            let last_seq = entries.last().map_or(after, |e| e.seq);
            let (logs, warnings, errors) = debug::counts();
            ToolResult::json(json!({
                "entries": entries,
                "last_seq": last_seq,
                "counts": { "logs": logs, "warnings": warnings, "errors": errors },
            }))
        },
    );

    s.add_tool(
        ToolDef::new(
            "clear_console",
            "Clear the Console",
            "Removes all Console messages.",
            empty.clone(),
        ),
        |_| {
            debug::clear();
            ToolResult::text("Console cleared.")
        },
    );

    s.add_tool(
        ToolDef::new(
            "debug_log",
            "Write to the Console",
            "Writes a message to the Console, like Debug.Log / LogWarning / LogError.",
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" },
                    "type": { "type": "string", "enum": ["Log", "Warning", "Error"], "description": "Default Log." }
                },
                "required": ["message"],
                "additionalProperties": false
            }),
        ),
        |args| {
            let Some(message) = args.get("message").and_then(Json::as_str) else {
                return ToolResult::error("`message` is required");
            };
            let log_type = args.get("type").and_then(Json::as_str).and_then(LogType::parse).unwrap_or(LogType::Log);
            debug::log(log_type, "mcp", message);
            ToolResult::text(format!("Logged as {log_type:?}."))
        },
    );

    let b = bus.clone();
    s.add_tool(
        ToolDef::new(
            "get_time",
            "Get Time",
            "Unity Time values: time, deltaTime, unscaledTime, fixedTime, fixedDeltaTime, timeScale, \
             maximumDeltaTime, frameCount, plus how many FixedUpdate/Update/LateUpdate calls have run.",
            empty.clone(),
        )
        .read_only(),
        move |_| b.run(|e| ToolResult::json(time_json(e))),
    );

    let b = bus.clone();
    s.add_tool(
        ToolDef::new(
            "set_time",
            "Set Time settings",
            "Changes Time.timeScale (0 freezes game time), Time.fixedDeltaTime and Time.maximumDeltaTime.",
            json!({
                "type": "object",
                "properties": {
                    "time_scale": { "type": "number", "minimum": 0, "maximum": 100 },
                    "fixed_delta_time": { "type": "number", "minimum": 0.0001, "maximum": 1 },
                    "maximum_delta_time": { "type": "number", "minimum": 0.0001, "maximum": 10 }
                },
                "additionalProperties": false
            }),
        ),
        move |args| {
            let get = |k: &str| args.get(k).and_then(Json::as_f64).map(|v| v as f32);
            let (scale, fixed, max) = (get("time_scale"), get("fixed_delta_time"), get("maximum_delta_time"));
            b.run(move |e| {
                if let Some(v) = scale {
                    e.time.set_time_scale(v);
                }
                if let Some(v) = fixed {
                    e.time.set_fixed_delta_time(v);
                }
                if let Some(v) = max {
                    e.time.set_maximum_delta_time(v);
                }
                ToolResult::json(time_json(e))
            })
        },
    );

    let b = bus.clone();
    s.add_tool(
        ToolDef::new(
            "set_paused",
            "Pause or resume",
            "Pauses or resumes the game loop (the editor's Pause button). Rendering continues while paused.",
            json!({
                "type": "object",
                "properties": { "paused": { "type": "boolean" } },
                "required": ["paused"],
                "additionalProperties": false
            }),
        ),
        move |args| {
            let Some(paused) = args.get("paused").and_then(Json::as_bool) else {
                return ToolResult::error("`paused` (boolean) is required");
            };
            b.run(move |e| {
                e.paused = paused;
                e.pending_steps = 0;
                ToolResult::text(if paused { "Paused." } else { "Resumed." })
            })
        },
    );

    let b = bus.clone();
    s.add_tool(
        ToolDef::new(
            "step_frames",
            "Step frames",
            "While paused, runs `count` frames of 1/60 s each (the editor's Step button). \
             Returns immediately; the frames run on the next display refreshes.",
            json!({
                "type": "object",
                "properties": { "count": { "type": "integer", "minimum": 1, "maximum": 600, "description": "Default 1." } },
                "additionalProperties": false
            }),
        ),
        move |args| {
            let count = args.get("count").and_then(Json::as_u64).unwrap_or(1).clamp(1, 600) as u32;
            b.run(move |e| {
                if !e.paused {
                    return ToolResult::error("not paused; call set_paused {paused: true} first");
                }
                e.pending_steps += count;
                ToolResult::text(format!("Queued {count} frame(s)."))
            })
        },
    );

    let b = bus.clone();
    s.add_tool(
        ToolDef::new(
            "capture_screenshot",
            "Capture screenshot",
            "Renders the current frame and returns it as a PNG. Defaults to the window size; \
             the long side is capped at `max_size` (default 1280) to keep responses small.",
            json!({
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "minimum": 16, "maximum": 4096 },
                    "height": { "type": "integer", "minimum": 16, "maximum": 4096 },
                    "max_size": { "type": "integer", "minimum": 64, "maximum": 4096 }
                },
                "additionalProperties": false
            }),
        )
        .read_only(),
        move |args| {
            let get = |k: &str| {
                args.get(k)
                    .and_then(Json::as_u64)
                    .map(|v| v.clamp(16, 4096) as u32)
            };
            let (width, height) = (get("width"), get("height"));
            let max_size = get("max_size").unwrap_or(1280);
            b.run(move |e| {
                let (ww, wh) = e.surface.size();
                let (mut w, mut h) = (width.unwrap_or(ww), height.unwrap_or(wh));
                let long = w.max(h);
                if long > max_size {
                    w = (w as u64 * max_size as u64 / long as u64).max(1) as u32;
                    h = (h as u64 * max_size as u64 / long as u64).max(1) as u32;
                }
                match e.capture(w, h).and_then(|img| img.to_png()) {
                    Ok(png) => ToolResult::png(png)
                        .with_text(format!("{w}x{h} frame {}", e.time.frame_count())),
                    Err(err) => ToolResult::error(err),
                }
            })
        },
    );

    s.add_tool(
        ToolDef::new(
            "list_objects",
            "List objects",
            "Objects in the scene. In P0 these are fixed (`Main Camera`, `Triangle`); \
             real GameObjects and the Hierarchy arrive in P1.",
            empty.clone(),
        )
        .read_only(),
        |_| ToolResult::json(json!({ "objects": OBJECTS })),
    );

    let b = bus.clone();
    s.add_tool(
        ToolDef::new(
            "inspect_object",
            "Inspect object",
            "All fields of an object with their types and current values, like the Inspector.",
            json!({
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Object name, e.g. \"Triangle\"." } },
                "required": ["name"],
                "additionalProperties": false
            }),
        )
        .read_only(),
        move |args| {
            let Some(name) = args.get("name").and_then(Json::as_str).map(str::to_owned) else {
                return ToolResult::error("`name` is required");
            };
            b.run(move |e| match object(e, &name) {
                Some(obj) => ToolResult::json(inspect(obj)),
                None => ToolResult::error(format!("no object named `{name}`; try {OBJECTS:?}")),
            })
        },
    );

    let b = bus.clone();
    s.add_tool(
        ToolDef::new(
            "set_object_field",
            "Set object field",
            "Sets one field of an object, like editing it in the Inspector. Vectors accept \
             [x, y, z] or {\"x\":..}; colors accept [r, g, b(, a)] in 0-1, {\"r\":..} or \"#RRGGBB(AA)\".",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "field": { "type": "string" },
                    "value": { "description": "New value; shape depends on the field type." }
                },
                "required": ["name", "field", "value"],
                "additionalProperties": false
            }),
        ),
        move |args| {
            let (Some(name), Some(field), Some(value)) = (
                args.get("name").and_then(Json::as_str).map(str::to_owned),
                args.get("field").and_then(Json::as_str).map(str::to_owned),
                args.get("value").cloned(),
            ) else {
                return ToolResult::error("`name`, `field` and `value` are required");
            };
            b.run(move |e| {
                let Some(obj) = object(e, &name) else {
                    return ToolResult::error(format!("no object named `{name}`; try {OBJECTS:?}"));
                };
                let Some(kind) = obj.type_info().fields.iter().find(|f| f.name == field).map(|f| f.kind) else {
                    let fields: Vec<_> = obj.type_info().fields.iter().map(|f| f.name).collect();
                    return ToolResult::error(format!("`{name}` has no field `{field}`; fields: {fields:?}"));
                };
                let parsed = match json_to_value(kind, &value) {
                    Ok(v) => v,
                    Err(err) => return ToolResult::error(format!("{field}: {err}")),
                };
                match obj.set_field(&field, parsed) {
                    Ok(()) => ToolResult::json(inspect(obj)),
                    Err(err) => ToolResult::error(err.to_string()),
                }
            })
        },
    );

    let b = bus.clone();
    s.add_tool(
        ToolDef::new(
            "list_types",
            "List reflected types",
            "Every type registered for reflection, with its fields (what the Inspector can edit).",
            empty.clone(),
        )
        .read_only(),
        move |_| {
            b.run(|e| ToolResult::json(json!({ "types": e.types.iter().collect::<Vec<_>>() })))
        },
    );

    let b = bus;
    s.add_tool(
        ToolDef::new(
            "quit",
            "Quit engine",
            "Closes the engine window and exits the process.",
            empty,
        )
        .destructive(),
        move |_| {
            b.run(|e| {
                e.quit_requested = true;
                ToolResult::text("Quitting.")
            })
        },
    );

    s
}

fn time_json(e: &Engine) -> Json {
    let t = &e.time;
    json!({
        "time": t.time(),
        "delta_time": t.delta_time(),
        "unscaled_time": t.unscaled_time(),
        "unscaled_delta_time": t.unscaled_delta_time(),
        "fixed_time": t.fixed_time(),
        "fixed_delta_time": t.fixed_delta_time(),
        "maximum_delta_time": t.maximum_delta_time(),
        "time_scale": t.time_scale(),
        "smooth_delta_time": t.smooth_delta_time(),
        "frame_count": t.frame_count(),
        "realtime_since_startup": oxide_core::time::realtime_since_startup(),
        "fixed_update_calls": t.fixed_step_count(),
        "update_calls": e.update_count,
        "late_update_calls": e.late_update_count,
        "paused": e.paused,
    })
}

fn inspect(obj: &dyn Reflect) -> Json {
    let info = obj.type_info();
    let fields: Vec<Json> = info
        .fields
        .iter()
        .map(|f| {
            json!({
                "name": f.name,
                "type": format!("{:?}", f.kind),
                "value": obj.get_field(f.name).map(|v| value_to_json(&v)),
            })
        })
        .collect();
    json!({ "type": info.name, "fields": fields })
}

fn value_to_json(v: &Value) -> Json {
    match v {
        Value::Bool(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Float(f) => json!(f),
        Value::String(s) => json!(s),
        Value::Vector2([x, y]) => json!({ "x": x, "y": y }),
        Value::Vector3([x, y, z]) => json!({ "x": x, "y": y, "z": z }),
        Value::Vector4([x, y, z, w]) | Value::Quaternion([x, y, z, w]) => {
            json!({ "x": x, "y": y, "z": z, "w": w })
        }
        Value::Color([r, g, b, a]) => json!({ "r": r, "g": g, "b": b, "a": a }),
    }
}

/// Reads `n` floats from `[..]` or from an object with the given keys.
/// Keys after the first `required` ones are optional and default to `default`.
fn floats(v: &Json, keys: &[&str], required: usize, default: f32) -> Result<Vec<f32>, String> {
    let nums: Vec<Option<f64>> = match v {
        Json::Array(a) => {
            if a.len() < required || a.len() > keys.len() {
                return Err(format!(
                    "expected {required}..={} numbers, got {}",
                    keys.len(),
                    a.len()
                ));
            }
            a.iter()
                .map(Json::as_f64)
                .map(Some)
                .collect::<Option<Vec<_>>>()
                .ok_or("array must contain only numbers")?
        }
        Json::Object(o) => keys
            .iter()
            .map(|k| o.get(*k).and_then(Json::as_f64))
            .collect(),
        _ => return Err(format!("expected an array or an object with keys {keys:?}")),
    };
    keys.iter()
        .enumerate()
        .map(|(i, k)| match nums.get(i).copied().flatten() {
            Some(n) => Ok(n as f32),
            None if i >= required => Ok(default),
            None => Err(format!("missing `{k}`")),
        })
        .collect()
}

fn parse_hex_color(s: &str) -> Result<[f32; 4], String> {
    let hex = s.trim_start_matches('#');
    let byte = |i: usize| {
        u8::from_str_radix(hex.get(i..i + 2).ok_or("bad hex color")?, 16)
            .map(|b| b as f32 / 255.0)
            .map_err(|_| "bad hex color".to_string())
    };
    match hex.len() {
        6 => Ok([byte(0)?, byte(2)?, byte(4)?, 1.0]),
        8 => Ok([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => Err("hex colors are #RRGGBB or #RRGGBBAA".into()),
    }
}

fn json_to_value(kind: ValueKind, v: &Json) -> Result<Value, String> {
    let arr = |f: Vec<f32>| -> [f32; 4] {
        [
            f[0],
            f[1],
            f.get(2).copied().unwrap_or(0.0),
            f.get(3).copied().unwrap_or(0.0),
        ]
    };
    Ok(match kind {
        ValueKind::Bool => Value::Bool(v.as_bool().ok_or("expected true/false")?),
        ValueKind::Int => Value::Int(v.as_i64().ok_or("expected an integer")?),
        ValueKind::Float => Value::Float(v.as_f64().ok_or("expected a number")?),
        ValueKind::String => Value::String(v.as_str().ok_or("expected a string")?.to_owned()),
        ValueKind::Vector2 => {
            let f = floats(v, &["x", "y"], 2, 0.0)?;
            Value::Vector2([f[0], f[1]])
        }
        ValueKind::Vector3 => {
            let f = floats(v, &["x", "y", "z"], 3, 0.0)?;
            Value::Vector3([f[0], f[1], f[2]])
        }
        ValueKind::Vector4 => Value::Vector4(arr(floats(v, &["x", "y", "z", "w"], 4, 0.0)?)),
        ValueKind::Quaternion => Value::Quaternion(arr(floats(v, &["x", "y", "z", "w"], 4, 0.0)?)),
        ValueKind::Color => Value::Color(match v {
            Json::String(s) => parse_hex_color(s)?,
            _ => arr(floats(v, &["r", "g", "b", "a"], 3, 1.0)?),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_values() {
        assert_eq!(
            json_to_value(ValueKind::Vector3, &json!([1, 2, 3])).unwrap(),
            Value::Vector3([1.0, 2.0, 3.0])
        );
        assert_eq!(
            json_to_value(ValueKind::Vector3, &json!({"x": 1, "y": 2, "z": 3})).unwrap(),
            Value::Vector3([1.0, 2.0, 3.0])
        );
        assert!(json_to_value(ValueKind::Vector3, &json!({"x": 1})).is_err());
        assert_eq!(
            json_to_value(ValueKind::Color, &json!([1, 0, 0])).unwrap(),
            Value::Color([1.0, 0.0, 0.0, 1.0])
        );
        assert_eq!(
            json_to_value(ValueKind::Color, &json!({"r": 0, "g": 1, "b": 0})).unwrap(),
            Value::Color([0.0, 1.0, 0.0, 1.0])
        );
        assert_eq!(
            json_to_value(ValueKind::Color, &json!("#ff000080")).unwrap(),
            Value::Color([1.0, 0.0, 0.0, 128.0 / 255.0])
        );
        assert!(json_to_value(ValueKind::Float, &json!("fast")).is_err());
    }
}
