//! MCP tools for the editor: state, selection, Play mode, Undo, menu items, the Scene view
//! camera and window screenshots. Register with `App::mcp_tools(oxide_editor::mcp::register)`.

use crate::{Action, Editor, PlayState, Tool, menus};
use oxide_engine::Engine;
use oxide_engine::bus::Bus;
use oxide_engine::mcp_tools::{obj_arg, schema, tool, vec3_arg};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_render::CapturedImage;
use serde_json::Value as Json;
use std::sync::Arc;

/// Runs `f` with the editor and the engine, or errors if no editor is installed.
fn with_editor(
    e: &mut Engine,
    f: impl FnOnce(&mut Editor, &mut Engine) -> ToolResult,
) -> ToolResult {
    let Some(mut shell) = e.shell.take() else {
        return ToolResult::error("the editor is not running (start `oxide` without --player)");
    };
    let r = match shell.as_any_mut().downcast_mut::<Editor>() {
        Some(ed) => f(ed, e),
        None => ToolResult::error("the installed shell is not the editor"),
    };
    e.shell = Some(shell);
    r
}

fn state_json(ed: &Editor, e: &Engine) -> Json {
    let st = &ed.st;
    let w = &e.world;
    let (undo, redo) = st.undo.labels();
    let cam = &st.scene_cam;
    json!({
        "mode": match (st.play, e.paused) {
            (PlayState::Edit, _) => "edit",
            (PlayState::Playing, false) => "playing",
            (PlayState::Playing, true) => "paused",
        },
        "scene": { "name": w.scene_name, "path": w.scene_path, "dirty": st.dirty, "objects": w.len() },
        "selection": st.selection.iter().map(|&id| json!({ "id": id.to_bits(), "path": w.path_of(id) })).collect::<Vec<_>>(),
        "tool": st.tool.name(),
        "handles": if st.local_handles { "local" } else { "global" },
        "game_view_focused": st.game_focused,
        "windows": ed.open_tabs().iter().map(|t| t.title()).collect::<Vec<_>>(),
        "undo": undo, "redo": redo,
        "scene_camera": {
            "position": cam.position().to_array(), "pivot": cam.pivot.to_array(),
            "yaw": cam.yaw, "pitch": cam.pitch, "distance": cam.distance,
            "orthographic": cam.orthographic, "fov": cam.fov,
        },
        "status": st.status,
    })
}

/// Box-filter downscale so the long side is at most `max`.
fn downscale(img: CapturedImage, max: u32) -> CapturedImage {
    let long = img.width.max(img.height);
    if long <= max {
        return img;
    }
    let f = long.div_ceil(max);
    let (w, h) = (img.width / f, img.height / f);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            for dy in 0..f {
                for dx in 0..f {
                    let p = img.pixel(x * f + dx, y * f + dy);
                    for c in 0..4 {
                        acc[c] += p[c] as u32;
                    }
                }
            }
            let i = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                rgba[i + c] = (acc[c] / (f * f)) as u8;
            }
        }
    }
    CapturedImage {
        width: w,
        height: h,
        rgba,
    }
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(s, bus, ToolDef::new("editor_get_state", "Get editor state",
        "Editor mode (edit/playing/paused), open scene and unsaved flag, selection, active tool, open windows, \
         undo/redo history and the Scene view camera.",
        schema(json!({}), &[])).read_only(),
        |_, e| with_editor(e, |ed, e| ToolResult::json(state_json(ed, e))));

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_select",
            "Select objects",
            "Sets the editor selection (what the Inspector shows and the handles move). `objects` is a list of ids or paths; \
         empty clears it. `frame: true` also frames them in the Scene view.",
            schema(
                json!({ "objects": { "type": "array" }, "frame": { "type": "boolean" } }),
                &["objects"],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let list = args
                    .get("objects")
                    .and_then(Json::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut ids = Vec::new();
                for v in list {
                    let mut a = serde_json::Map::new();
                    a.insert("o".into(), v);
                    match obj_arg(&a, &e.world, "o") {
                        Ok(id) => ids.push(id),
                        Err(r) => return r,
                    }
                }
                for &id in &ids {
                    ed.st.hierarchy.reveal(&e.world, id);
                }
                ed.st.selection = ids;
                if args.get("frame").and_then(Json::as_bool).unwrap_or(false) {
                    ed.st.scene_ui.frame_request = true;
                }
                ToolResult::json(state_json(ed, e)["selection"].clone())
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_play",
            "Play mode",
            "Unity's Play/Pause/Step buttons. `action`: play (enter Play mode; the scene is snapshotted), stop (leave it; \
         the scene is restored), pause, resume, step (one frame).",
            schema(
                json!({ "action": { "type": "string", "enum": ["play", "stop", "pause", "resume", "step"] } }),
                &["action"],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let playing = ed.st.play == PlayState::Playing;
                let action = match args.get("action").and_then(Json::as_str).unwrap_or("") {
                    "play" if !playing => Some(Action::TogglePlay),
                    "stop" if playing => Some(Action::TogglePlay),
                    "pause" if playing && !e.paused => Some(Action::TogglePause),
                    "resume" if playing && e.paused => Some(Action::TogglePause),
                    "step" => Some(Action::Step),
                    "play" | "stop" | "pause" | "resume" => None,
                    other => return ToolResult::error(format!("unknown action `{other}`")),
                };
                if let Some(a) = action
                    && let Err(err) = ed.apply(e, a)
                {
                    return ToolResult::error(err);
                }
                ToolResult::json(state_json(ed, e)["mode"].clone())
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_undo",
            "Undo / Redo",
            "Undoes (or with `redo: true`, redoes) the last `steps` editor edits (default 1).",
            schema(
                json!({ "redo": { "type": "boolean" }, "steps": { "type": "integer", "minimum": 1, "maximum": 64 } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let redo = args.get("redo").and_then(Json::as_bool).unwrap_or(false);
                let steps = args.get("steps").and_then(Json::as_u64).unwrap_or(1);
                let mut done = Vec::new();
                for _ in 0..steps {
                    match ed.apply(e, if redo { Action::Redo } else { Action::Undo }) {
                        Ok(m) => done.push(m),
                        Err(err) if done.is_empty() => return ToolResult::error(err),
                        Err(_) => break,
                    }
                }
                ToolResult::json(
                    json!({ "done": done, "history": state_json(ed, e)["undo"].clone() }),
                )
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_execute_menu",
            "Run a menu item",
            "Runs a main-menu item by path, like Unity's EditorApplication.ExecuteMenuItem: e.g. `GameObject/3D Object/Cube`, \
         `Edit/Duplicate`, `Component/Physics/Rigidbody`, `File/Save`, `Window/General/Console`. \
         Call with no path to list every item.",
            schema(json!({ "path": { "type": "string" } }), &[]),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let items = menus::menu_items(&e.world);
                let Some(path) = args.get("path").and_then(Json::as_str) else {
                    let paths: Vec<&str> = items
                        .iter()
                        .filter(|m| m.action.is_some())
                        .map(|m| m.path.as_str())
                        .collect();
                    return ToolResult::json(json!(paths));
                };
                let Some(action) = items
                    .iter()
                    .find(|m| m.path.eq_ignore_ascii_case(path) && m.action.is_some())
                    .and_then(|m| m.action.clone())
                else {
                    return ToolResult::error(format!(
                        "no menu item `{path}`; call without `path` to list them"
                    ));
                };
                match ed.apply(e, action) {
                    Ok(msg) => ToolResult::json(
                        json!({ "ok": true, "message": msg, "selection": state_json(ed, e)["selection"].clone() }),
                    ),
                    Err(err) => ToolResult::error(err),
                }
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_set_tool",
            "Set transform tool",
            "Selects the Scene view tool (hand, move, rotate, scale) and handle orientation (global/local).",
            schema(
                json!({ "tool": { "type": "string", "enum": ["hand", "move", "rotate", "scale"] },
                       "handles": { "type": "string", "enum": ["global", "local"] } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                if let Some(t) = args.get("tool").and_then(Json::as_str) {
                    match Tool::parse(t) {
                        Some(t) => ed.st.tool = t,
                        None => return ToolResult::error(format!("unknown tool `{t}`")),
                    }
                }
                if let Some(h) = args.get("handles").and_then(Json::as_str) {
                    ed.st.local_handles = h == "local";
                }
                ToolResult::json(state_json(ed, e))
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_scene_camera",
            "Move Scene view camera",
            "Positions the Scene view camera: `position` + `look_at`, or `pivot`/`yaw`/`pitch`/`distance`, or `frame` (an \
         object to frame, like pressing F). Also `orthographic` and `fov`.",
            schema(
                json!({ "position": { "type": "array" }, "look_at": { "type": "array" }, "pivot": { "type": "array" },
                       "yaw": { "type": "number" }, "pitch": { "type": "number" }, "distance": { "type": "number" },
                       "frame": {}, "orthographic": { "type": "boolean" }, "fov": { "type": "number" } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let cam = &mut ed.st.scene_cam;
                let num = |k: &str| args.get(k).and_then(Json::as_f64).map(|v| v as f32);
                if let (Some(p), Some(t)) = (
                    oxide_engine::tri!(vec3_arg(args, "position")),
                    oxide_engine::tri!(vec3_arg(args, "look_at")),
                ) {
                    cam.look_at(p, t);
                }
                if let Some(p) = oxide_engine::tri!(vec3_arg(args, "pivot")) {
                    cam.pivot = p;
                }
                if let Some(v) = num("yaw") {
                    cam.yaw = v;
                }
                if let Some(v) = num("pitch") {
                    cam.pitch = v.clamp(-89.9, 89.9);
                }
                if let Some(v) = num("distance") {
                    cam.distance = v.max(0.05);
                }
                if let Some(v) = num("fov") {
                    cam.fov = v.clamp(4.0, 120.0);
                }
                if let Some(o) = args.get("orthographic").and_then(Json::as_bool) {
                    cam.orthographic = o;
                }
                if args.contains_key("frame") {
                    let id = oxide_engine::tri!(obj_arg(args, &e.world, "frame"));
                    if let Some((c, r)) = crate::scene_view::bounds_of(&e.world, &[id]) {
                        ed.st.scene_cam.frame(c, r);
                    }
                }
                ToolResult::json(state_json(ed, e)["scene_camera"].clone())
            })
        },
    );

    tool(s, bus, ToolDef::new("editor_capture", "Screenshot the editor",
        "PNG of the whole editor window (menus, Hierarchy, Scene/Game views, Inspector, ...), as the user sees it. \
         Long side capped by `max_size` (default 1600).",
        schema(json!({ "max_size": { "type": "integer", "minimum": 256, "maximum": 4096 } }), &[])).read_only(),
        |args, e| {
            let max = args.get("max_size").and_then(Json::as_u64).unwrap_or(1600) as u32;
            match e.capture_window().map(|img| downscale(img, max)).and_then(|img| {
                let (w, h) = (img.width, img.height);
                img.to_png().map(|p| (p, w, h))
            }) {
                Ok((png, w, h)) => ToolResult::png(png).with_text(format!("{w}x{h}")),
                Err(err) => ToolResult::error(err),
            }
        });
}
