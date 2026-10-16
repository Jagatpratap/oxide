//! MCP tools for the editor: state, selection, Play mode, Undo, menu items, the Scene view
//! camera and window screenshots. Register with `App::mcp_tools(oxide_editor::mcp::register)`.

use crate::{Action, Editor, PlayState, Tool, asset_ops, menus};
use oxide_assets::AssetServer;
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
        "handle_position": if st.center_handles { "center" } else { "pivot" },
        "game_view_focused": st.game_focused,
        "windows": ed.open_tabs().iter().map(|t| t.title()).collect::<Vec<_>>(),
        "undo": undo, "redo": redo,
        "scene_camera": {
            "position": cam.position().to_array(), "pivot": cam.pivot.to_array(),
            "yaw": cam.yaw, "pitch": cam.pitch, "distance": cam.distance,
            "orthographic": cam.orthographic, "fov": cam.fov,
            "draw_mode": st.scene_ui.draw_mode.name(),
            "mode_2d": st.scene_ui.mode_2d,
            "lighting": st.scene_ui.lighting,
            "effects": {
                "enabled": st.scene_ui.effects.enabled, "skybox": st.scene_ui.effects.skybox,
                "fog": st.scene_ui.effects.fog, "particles": st.scene_ui.effects.particles,
                "post_processing": st.scene_ui.effects.post_processing,
            },
            "scene_visibility": !st.visibility.show_all,
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
                let items = menus::menu_items(&e.world, &ed.st.keymap);
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
            "Selects the Scene view tool (hand, move, rotate, scale, rect, transform), handle orientation \
             (global/local) and handle position (pivot/center).",
            schema(
                json!({ "tool": { "type": "string", "enum": ["hand", "move", "rotate", "scale", "rect", "transform"] },
                       "handles": { "type": "string", "enum": ["global", "local"] },
                       "position": { "type": "string", "enum": ["pivot", "center"] } }),
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
                if let Some(p) = args.get("position").and_then(Json::as_str) {
                    ed.st.center_handles = p == "center";
                }
                ToolResult::json(state_json(ed, e))
            })
        },
    );

    let windows: Vec<&str> = crate::Tab::ALL
        .iter()
        .chain(&[
            crate::Tab::ProjectSettings,
            crate::Tab::Preferences,
            crate::Tab::Shortcuts,
            crate::Tab::InputActions,
            crate::Tab::Animator,
            crate::Tab::Animation,
            crate::Tab::AudioMixer,
            crate::Tab::SpriteEditor,
            crate::Tab::TilePalette,
        ])
        .map(|t| t.title())
        .collect();
    tool(
        s,
        bus,
        ToolDef::new(
            "editor_open_window",
            "Open editor window",
            "Opens (or focuses) an editor window. For Project Settings and Preferences, `page` picks the page \
             (e.g. \"Physics\", \"Quality\", \"Tags and Layers\", \"Input Manager\"; Preferences: \"General\", \
             \"Console\", \"Scene View\", \"Colors\", \"External Tools\"). For Input Actions, `asset` is the \
             .inputactions file to edit.",
            schema(
                json!({ "window": { "type": "string", "enum": windows }, "page": { "type": "string" }, "asset": { "type": "string" } }),
                &["window"],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let name = args.get("window").and_then(Json::as_str).unwrap_or("");
                let all = crate::Tab::ALL.into_iter().chain([
                    crate::Tab::ProjectSettings,
                    crate::Tab::Preferences,
                    crate::Tab::Shortcuts,
                    crate::Tab::InputActions,
                    crate::Tab::Animator,
                    crate::Tab::Animation,
                    crate::Tab::AudioMixer,
                    crate::Tab::SpriteEditor,
                    crate::Tab::TilePalette,
                ]);
                let Some(tab) = all
                    .into_iter()
                    .find(|t| t.title().eq_ignore_ascii_case(name))
                else {
                    return ToolResult::error(format!("unknown window `{name}`"));
                };
                if tab == crate::Tab::SpriteEditor
                    && let Some(a) = args.get("asset").and_then(Json::as_str)
                {
                    ed.st.sprite_editor.open(a);
                }
                if tab == crate::Tab::InputActions {
                    match args.get("asset").and_then(Json::as_str) {
                        Some(a) => ed.st.input_actions.open(e, a),
                        None if ed.st.input_actions.path().is_none() => {
                            return ToolResult::error(
                                "Input Actions needs `asset`: an .inputactions file",
                            );
                        }
                        None => {}
                    }
                }
                if let Some(page) = args.get("page").and_then(Json::as_str) {
                    let find = |n: &str| n.eq_ignore_ascii_case(page);
                    match tab {
                        crate::Tab::ProjectSettings => {
                            match crate::settings_window::PAGES.iter().find(|(_, n)| find(n)) {
                                Some((p, _)) => ed.st.settings.page = *p,
                                None => {
                                    return ToolResult::error(format!(
                                        "no Project Settings page `{page}`"
                                    ));
                                }
                            }
                        }
                        crate::Tab::Preferences => match crate::prefs::PAGES
                            .iter()
                            .find(|(_, n)| find(n))
                        {
                            Some((p, _)) => ed.st.prefs_page = *p,
                            None => {
                                return ToolResult::error(format!("no Preferences page `{page}`"));
                            }
                        },
                        _ => {
                            return ToolResult::error(
                                "only Project Settings and Preferences have pages",
                            );
                        }
                    }
                }
                ed.focus_tab(tab);
                ToolResult::json(state_json(ed, e))
            })
        },
    );

    let sizes: Vec<&str> = crate::game_view::SIZES.iter().map(|(n, _)| *n).collect();
    tool(
        s,
        bus,
        ToolDef::new(
            "editor_game_view",
            "Game view settings",
            "Game view control bar: `size` (aspect or fixed resolution, e.g. \"Full HD (1920x1080)\"), `scale` \
             (zoom, 1 = fit), `maximize_on_play`, `mute_audio`, `stats`. Returns the current settings.",
            schema(
                json!({
                    "size": { "type": "string", "enum": sizes },
                    "scale": { "type": "number", "minimum": 1, "maximum": 5 },
                    "maximize_on_play": { "type": "boolean" },
                    "mute_audio": { "type": "boolean" },
                    "stats": { "type": "boolean" }
                }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, _| {
                let g = &mut ed.st.game;
                if let Some(n) = args.get("size").and_then(Json::as_str) {
                    match crate::game_view::SIZES
                        .iter()
                        .find(|(s, _)| s.eq_ignore_ascii_case(n))
                    {
                        Some((_, sz)) => g.size = *sz,
                        None => return ToolResult::error(format!("unknown size `{n}`")),
                    }
                }
                if let Some(v) = args.get("scale").and_then(Json::as_f64) {
                    g.scale = (v as f32).clamp(1.0, 5.0);
                }
                for (k, f) in [
                    ("maximize_on_play", &mut g.maximize_on_play),
                    ("mute_audio", &mut g.mute_audio),
                    ("stats", &mut g.stats),
                ] {
                    if let Some(v) = args.get(k).and_then(Json::as_bool) {
                        *f = v;
                    }
                }
                let size = crate::game_view::SIZES
                    .iter()
                    .find(|(_, s)| *s == g.size)
                    .map_or("custom", |(n, _)| *n);
                ToolResult::json(json!({
                    "size": size, "scale": g.scale, "maximize_on_play": g.maximize_on_play,
                    "mute_audio": g.mute_audio, "stats": g.stats,
                }))
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_inspector",
            "Inspector lock and Debug mode",
            "Locks the Inspector to the current selection (`lock`: true) or unlocks it, toggles Debug mode, and \
             returns which objects the Inspector shows. With several objects selected the Inspector edits them all. \
             `scroll_to` (a component type, e.g. \"CarController\") expands that component and scrolls it into view.",
            schema(
                json!({ "lock": { "type": "boolean" }, "debug": { "type": "boolean" }, "scroll_to": { "type": "string" } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let ins = &mut ed.st.inspector;
                if let Some(l) = args.get("lock").and_then(Json::as_bool) {
                    ins.locked = l.then(|| ed.st.selection.clone());
                }
                if let Some(d) = args.get("debug").and_then(Json::as_bool) {
                    ins.debug = d;
                }
                if let Some(t) = args.get("scroll_to").and_then(Json::as_str) {
                    ins.scroll_to = Some(t.to_owned());
                }
                let shown = ins
                    .locked
                    .clone()
                    .unwrap_or_else(|| ed.st.selection.clone());
                let names: Vec<_> = shown
                    .iter()
                    .filter_map(|&id| e.world.get(id).map(|o| o.name.clone()))
                    .collect();
                ToolResult::json(
                    json!({ "locked": ins.locked.is_some(), "debug": ins.debug, "inspecting": names }),
                )
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_animation",
            "Animation window",
            "The Animation window (it edits the clips of the selected object's Animator): returns its target, \
             clip, playhead, record and preview state. `time` moves the playhead (seconds, snapped to 60 fps \
             frames), `record` toggles Record mode (moving objects or changing component values then writes \
             keys), `preview` poses the object with the clip in Edit mode, `clip` picks one of the target's clips.",
            schema(
                json!({ "time": { "type": "number" }, "record": { "type": "boolean" }, "preview": { "type": "boolean" }, "clip": { "type": "string" } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                ed.st.animation.control(
                    &mut e.world,
                    args.get("time").and_then(Json::as_f64).map(|t| t as f32),
                    args.get("record").and_then(Json::as_bool),
                    args.get("preview").and_then(Json::as_bool),
                    args.get("clip").and_then(Json::as_str),
                );
                ToolResult::json(ed.st.animation.state_json())
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_ui_input",
            "Click, drag, scroll or type in the editor UI",
            "Sends synthetic mouse and keyboard input to the editor's UI, for testing windows that need \
             the pointer (curve editors, gizmo drags, menus). Coordinates are UI points, the same as pixels \
             in an `editor_capture` image. `action`: click | double_click | right_click | move | drag (from \
             x,y to `to` [x, y] over `steps` frames) | scroll (`delta` points, positive scrolls content down) \
             | key (`key` name such as \"Delete\", \"F\", \"Enter\") | text. `modifiers` may hold shift, \
             command, alt. Input plays back over the next frames; capture a few frames later to see it.",
            schema(
                json!({
                    "action": { "type": "string", "enum": ["click", "double_click", "right_click", "move", "drag", "scroll", "key", "text"] },
                    "x": { "type": "number" }, "y": { "type": "number" },
                    "to": { "type": "array" },
                    "steps": { "type": "integer", "minimum": 1, "maximum": 240 },
                    "delta": { "type": "number" },
                    "key": { "type": "string" },
                    "text": { "type": "string" },
                    "modifiers": { "type": "array" }
                }),
                &["action"],
            ),
        ),
        |args, e| {
            use egui::{Event, Modifiers, PointerButton, pos2};
            let num = |k: &str| args.get(k).and_then(Json::as_f64).map(|v| v as f32);
            let p = pos2(num("x").unwrap_or(0.0), num("y").unwrap_or(0.0));
            let mut modifiers = Modifiers::NONE;
            for m in args
                .get("modifiers")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                match m.as_str().unwrap_or("") {
                    "shift" => modifiers.shift = true,
                    "alt" => modifiers.alt = true,
                    "command" | "cmd" | "ctrl" => {
                        modifiers.command = true;
                        if cfg!(target_os = "macos") {
                            modifiers.mac_cmd = true;
                        } else {
                            modifiers.ctrl = true;
                        }
                    }
                    other => return ToolResult::error(format!("unknown modifier `{other}`")),
                }
            }
            let button = |pos, button, pressed| Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            };
            let click = |b: PointerButton| {
                vec![
                    vec![Event::PointerMoved(p)],
                    vec![button(p, b, true)],
                    vec![button(p, b, false)],
                ]
            };
            let batches: Vec<Vec<Event>> =
                match args.get("action").and_then(Json::as_str).unwrap_or("") {
                    "click" => click(PointerButton::Primary),
                    "right_click" => click(PointerButton::Secondary),
                    "double_click" => {
                        let mut v = click(PointerButton::Primary);
                        v.extend(click(PointerButton::Primary).into_iter().skip(1));
                        v
                    }
                    "move" => vec![vec![Event::PointerMoved(p)]],
                    "drag" => {
                        let Some(to) = args
                            .get("to")
                            .and_then(Json::as_array)
                            .filter(|a| a.len() == 2)
                            .and_then(|a| Some(pos2(a[0].as_f64()? as f32, a[1].as_f64()? as f32)))
                        else {
                            return ToolResult::error("drag needs `to`: [x, y]");
                        };
                        let steps = args
                            .get("steps")
                            .and_then(Json::as_u64)
                            .unwrap_or(10)
                            .max(1) as usize;
                        let mut v = vec![
                            vec![Event::PointerMoved(p)],
                            vec![button(p, PointerButton::Primary, true)],
                        ];
                        for i in 1..=steps {
                            v.push(vec![Event::PointerMoved(
                                p.lerp(to, i as f32 / steps as f32),
                            )]);
                        }
                        v.push(vec![button(to, PointerButton::Primary, false)]);
                        v
                    }
                    "scroll" => vec![
                        vec![Event::PointerMoved(p)],
                        vec![Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, num("delta").unwrap_or(0.0)),
                            phase: egui::TouchPhase::Move,
                            modifiers,
                        }],
                    ],
                    "key" => {
                        let name = args.get("key").and_then(Json::as_str).unwrap_or("");
                        let Some(key) = egui::Key::from_name(name) else {
                            return ToolResult::error(format!("unknown key `{name}`"));
                        };
                        let k = |pressed| Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers,
                        };
                        vec![vec![k(true)], vec![k(false)]]
                    }
                    "text" => vec![vec![Event::Text(
                        args.get("text")
                            .and_then(Json::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    )]],
                    other => return ToolResult::error(format!("unknown action `{other}`")),
                };
            let frames = batches.len();
            e.world
                .resource_mut::<oxide_ui::Gui>()
                .injected_input
                .extend(batches);
            ToolResult::json(json!({ "queued_frames": frames }))
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_preferences",
            "Editor Preferences",
            "Reads the per-user editor Preferences; `values` changes some (e.g. {\"move_snap\": 0.5, \
             \"script_editor\": \"code -g {file}:{line}\", \"stack_trace_warnings\": true}) and saves them.",
            schema(json!({ "values": { "type": "object" } }), &[]),
        ),
        |args, e| {
            with_editor(e, |ed, _| {
                if let Some(values) = args.get("values").and_then(Json::as_object) {
                    let mut cur = serde_json::to_value(&ed.st.prefs).unwrap_or_default();
                    let Some(obj) = cur.as_object_mut() else {
                        return ToolResult::error("preferences are not an object");
                    };
                    for (k, v) in values {
                        if !obj.contains_key(k) {
                            return ToolResult::error(format!("no preference `{k}`"));
                        }
                        obj.insert(k.clone(), v.clone());
                    }
                    match serde_json::from_value(cur) {
                        Ok(p) => {
                            ed.st.prefs = p;
                            ed.st.prefs_changed = true;
                        }
                        Err(err) => return ToolResult::error(err.to_string()),
                    }
                }
                ToolResult::json(serde_json::to_value(&ed.st.prefs).unwrap_or_default())
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
         object to frame, like pressing F). Also `orthographic`, `fov`, `draw_mode` (Shaded, Wireframe, Shaded \
         Wireframe, Albedo, Normals, Lighting Only, Shadow Cascades, Baked Lightmap), `mode_2d`, `lighting` (scene \
         lights on/off) and `effects` ({enabled, skybox, fog, particles, post_processing}).",
            schema(
                json!({ "position": { "type": "array" }, "look_at": { "type": "array" }, "pivot": { "type": "array" },
                       "yaw": { "type": "number" }, "pitch": { "type": "number" }, "distance": { "type": "number" },
                       "frame": {}, "orthographic": { "type": "boolean" }, "fov": { "type": "number" },
                       "draw_mode": { "type": "string" }, "mode_2d": { "type": "boolean" },
                       "lighting": { "type": "boolean" }, "effects": { "type": "object" } }),
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
                if let Some(name) = args.get("draw_mode").and_then(Json::as_str) {
                    let Some(mode) = oxide_render::DebugView::ALL
                        .into_iter()
                        .find(|m| m.name().eq_ignore_ascii_case(name))
                    else {
                        return ToolResult::error(format!("unknown draw_mode `{name}`"));
                    };
                    ed.st.scene_ui.draw_mode = mode;
                }
                if let Some(on) = args.get("mode_2d").and_then(Json::as_bool) {
                    crate::scene_view::set_2d(&mut ed.st, on);
                }
                if let Some(on) = args.get("lighting").and_then(Json::as_bool) {
                    ed.st.scene_ui.lighting = on;
                }
                if let Some(fx) = args.get("effects").and_then(Json::as_object) {
                    let e = &mut ed.st.scene_ui.effects;
                    for (k, flag) in [
                        ("enabled", &mut e.enabled),
                        ("skybox", &mut e.skybox),
                        ("fog", &mut e.fog),
                        ("particles", &mut e.particles),
                        ("post_processing", &mut e.post_processing),
                    ] {
                        if let Some(v) = fx.get(k).and_then(Json::as_bool) {
                            *flag = v;
                        }
                    }
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

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_scene_visibility",
            "Scene visibility",
            "The Hierarchy's eye and hand toggles: `hide`/`show` objects in the Scene view (children follow), \
             `disable_picking`/`enable_picking`, `show_all` (clear everything), or `enabled: false` to show hidden \
             objects without clearing them. Game view and builds are unaffected. Returns hidden/unpickable objects.",
            schema(
                json!({ "action": { "type": "string", "enum": ["status", "hide", "show", "disable_picking", "enable_picking", "show_all"] },
                       "objects": { "type": "array" }, "enabled": { "type": "boolean" } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let w = &e.world;
                let mut ids = Vec::new();
                for v in args
                    .get("objects")
                    .and_then(Json::as_array)
                    .cloned()
                    .unwrap_or_default()
                {
                    let mut a = serde_json::Map::new();
                    a.insert("o".into(), v);
                    match obj_arg(&a, w, "o") {
                        Ok(id) => ids.push(id),
                        Err(r) => return r,
                    }
                }
                let vis = &mut ed.st.visibility;
                let action = args
                    .get("action")
                    .and_then(Json::as_str)
                    .unwrap_or("status");
                for &id in &ids {
                    match action {
                        "hide" if !vis.hidden_in_chain(w, id) => vis.toggle_hidden(w, id),
                        "show" if vis.hidden_in_chain(w, id) => vis.toggle_hidden(w, id),
                        "disable_picking" if !vis.is_unpickable(w, id) => {
                            vis.toggle_pickable(w, id)
                        }
                        "enable_picking" if vis.is_unpickable(w, id) => vis.toggle_pickable(w, id),
                        _ => {}
                    }
                }
                match action {
                    "show_all" => {
                        vis.hidden.clear();
                        vis.unpickable.clear();
                    }
                    "status" | "hide" | "show" | "disable_picking" | "enable_picking" => {}
                    other => return ToolResult::error(format!("unknown action `{other}`")),
                }
                if let Some(on) = args.get("enabled").and_then(Json::as_bool) {
                    vis.show_all = !on;
                }
                let list = |set: &std::collections::HashSet<oxide_scene::GameObjectId>| {
                    set.iter()
                        .map(|&id| json!({ "id": id.to_bits(), "path": w.path_of(id) }))
                        .collect::<Vec<_>>()
                };
                ToolResult::json(json!({
                    "enabled": !vis.show_all,
                    "hidden": list(&vis.hidden),
                    "unpickable": list(&vis.unpickable),
                }))
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_project",
            "Project window",
            "Assets in the Project window. `list` a folder (default: the open one), `search` with Unity syntax \
             (`car t:Prefab l:Vehicle`; `folder` limits it; also typed into the window's search box), `select` an asset (shows it in the Project window and \
             Inspector), `create_folder`, `rename` (`name`; extension kept), `move` (`to`: new path), `duplicate`, \
             `delete` (macOS: to the Trash), `labels` (read, or set with `labels`), `zoom` (icon size 16-128; 16 is \
             the list view). Moving or renaming rewrites references in scenes, prefabs, materials and the open scene.",
            schema(
                json!({ "action": { "type": "string", "enum": ["status", "list", "search", "select", "create_folder", "rename", "move", "duplicate", "delete", "labels", "zoom"] },
                       "path": { "type": "string" }, "folder": { "type": "string" }, "query": { "type": "string" },
                       "name": { "type": "string" }, "to": { "type": "string" },
                       "labels": { "type": "array", "items": { "type": "string" } }, "zoom": { "type": "number" } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let root = e.world.resource::<AssetServer>().full_path("");
                let s = |k: &str| args.get(k).and_then(Json::as_str).map(str::to_owned);
                let entry_json = |x: &crate::project::Entry| json!({ "name": x.name, "path": x.path, "type": crate::project::entry_kind(x).0, "labels": x.labels });
                let path = s("path");
                let need = || path.clone().ok_or_else(|| "`path` is required".to_owned());
                let st = &mut ed.st;
                let r: Result<Json, String> = (|| {
                    Ok(match s("action").as_deref().unwrap_or("status") {
                        "status" => json!({
                            "folder": st.project.folder, "selected": st.project.selected,
                            "search": st.project.search, "zoom": st.project.zoom,
                            "inspecting": st.project.inspecting,
                        }),
                        "list" => {
                            let folder = s("folder").unwrap_or_else(|| st.project.folder.clone());
                            let v: Vec<Json> = crate::project::list_dir(&root, &folder)
                                .iter()
                                .map(entry_json)
                                .collect();
                            json!({ "folder": folder, "entries": v })
                        }
                        "search" => {
                            // Shown in the Project window too, as if typed in its search box.
                            st.project.search = s("query").unwrap_or_default();
                            st.project.search_all = s("folder").is_none();
                            let q = crate::project::Query::parse(&st.project.search);
                            let folder = s("folder").unwrap_or_else(|| "Assets".into());
                            let v: Vec<Json> = crate::project::list_all(&root, &folder)
                                .iter()
                                .filter(|x| q.matches(x))
                                .map(entry_json)
                                .collect();
                            json!({ "results": v })
                        }
                        "select" => {
                            let p = need()?;
                            if !root.join(&p).exists() {
                                return Err(format!("{p} doesn't exist"));
                            }
                            st.project.show(&p);
                            st.selection.clear();
                            st.actions.push(Action::OpenTab(crate::Tab::Project));
                            json!({ "selected": p })
                        }
                        "create_folder" => {
                            let p = asset_ops::unique_path(&root, &need()?);
                            if !p.starts_with("Assets/") {
                                return Err("folders go under Assets/".into());
                            }
                            std::fs::create_dir_all(root.join(&p)).map_err(|e| e.to_string())?;
                            st.project.invalidate();
                            json!({ "created": p })
                        }
                        "rename" | "move" => {
                            let p = need()?;
                            let to = if s("action").as_deref() == Some("rename") {
                                crate::project::renamed_path(
                                    &root,
                                    &p,
                                    &s("name").ok_or("`name` is required")?,
                                )?
                            } else {
                                s("to").ok_or("`to` is required")?
                            };
                            let msg = asset_ops::move_asset(st, e, &p, &to)?;
                            st.project.invalidate();
                            json!({ "moved": to, "message": msg })
                        }
                        "duplicate" => {
                            let new = asset_ops::duplicate_asset(&root, &need()?)?;
                            st.project.invalidate();
                            json!({ "created": new })
                        }
                        "delete" => {
                            let p = need()?;
                            asset_ops::delete_asset(&root, &p)?;
                            if st.project.selected.as_deref() == Some(p.as_str()) {
                                st.project.selected = None;
                            }
                            st.project.invalidate();
                            json!({ "deleted": p })
                        }
                        "labels" => {
                            let p = need()?;
                            let assets = e.world.resource::<AssetServer>();
                            if let Some(v) = args.get("labels").and_then(Json::as_array) {
                                let v: Vec<String> = v
                                    .iter()
                                    .filter_map(|l| l.as_str().map(str::to_owned))
                                    .collect();
                                assets.set_asset_labels(&p, &v)?;
                                st.project.invalidate();
                            }
                            json!({ "path": p, "labels": assets.asset_labels(&p) })
                        }
                        "zoom" => {
                            let z = args
                                .get("zoom")
                                .and_then(Json::as_f64)
                                .ok_or("`zoom` is required")?;
                            st.project.zoom = (z as f32).clamp(crate::project::MIN_ZOOM, 128.0);
                            json!({ "zoom": st.project.zoom })
                        }
                        other => return Err(format!("unknown action `{other}`")),
                    })
                })();
                match r {
                    Ok(v) => ToolResult::json(v),
                    Err(m) => ToolResult::error(m),
                }
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_shortcuts",
            "Keyboard shortcuts",
            "The Shortcuts Manager's bindings. `list` every command (id, shortcuts, defaults, conflicts; `query` \
             filters by id or key), `set` a command's shortcuts (`id`, `shortcuts`: [\"Cmd+Shift+K\"]; Cmd is \
             Ctrl outside macOS; an empty list removes them; `reassign: true` takes the keys from other commands), \
             or `reset` one command (`id`) or all. Changes are saved in Preferences.",
            schema(
                json!({ "action": { "type": "string", "enum": ["list", "set", "reset"] },
                       "id": { "type": "string" }, "query": { "type": "string" },
                       "shortcuts": { "type": "array", "items": { "type": "string" } },
                       "reassign": { "type": "boolean" } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, _| {
                use crate::shortcuts::{Binding, Keymap};
                let st = &mut ed.st;
                let s = |k: &str| args.get(k).and_then(Json::as_str).map(str::to_owned);
                let r: Result<Json, String> = (|| {
                    let action = s("action").unwrap_or_else(|| "list".into());
                    match action.as_str() {
                        "set" => {
                            let id = s("id").ok_or("`id` is required")?;
                            let i = st
                                .keymap
                                .find(&id)
                                .ok_or_else(|| format!("no command `{id}`"))?;
                            let list = args
                                .get("shortcuts")
                                .and_then(Json::as_array)
                                .ok_or("`shortcuts` is required")?;
                            let mut parsed = Vec::new();
                            for v in list {
                                let t = v.as_str().unwrap_or_default();
                                parsed.push(
                                    Binding::parse(t)
                                        .ok_or_else(|| format!("can't read shortcut `{t}`"))?,
                                );
                            }
                            let steal = args
                                .get("reassign")
                                .and_then(Json::as_bool)
                                .unwrap_or(false);
                            st.keymap.bindings[i].clear();
                            for b in parsed {
                                st.keymap.bind(i, b, false, steal);
                            }
                        }
                        "reset" => match s("id") {
                            Some(id) => {
                                let i = st
                                    .keymap
                                    .find(&id)
                                    .ok_or_else(|| format!("no command `{id}`"))?;
                                st.keymap.bindings[i] = st.keymap.commands[i].defaults.clone();
                            }
                            None => st.keymap = Keymap::default(),
                        },
                        "list" => {}
                        other => return Err(format!("unknown action `{other}`")),
                    }
                    if action != "list" {
                        st.prefs.shortcuts = st.keymap.overrides();
                        st.prefs_changed = true;
                    }
                    let q = s("query").unwrap_or_default().to_lowercase();
                    let km = &st.keymap;
                    let rows: Vec<Json> = (0..km.commands.len())
                        .filter(|&i| {
                            q.is_empty()
                                || km.commands[i].id.to_lowercase().contains(&q)
                                || km.bindings[i].iter().any(|b| b.to_config().to_lowercase().contains(&q))
                        })
                        .map(|i| {
                            let c = &km.commands[i];
                            json!({
                                "id": c.id,
                                "shortcuts": km.bindings[i].iter().map(|b| b.to_config()).collect::<Vec<_>>(),
                                "display": km.bindings[i].iter().map(|b| b.label()).collect::<Vec<_>>(),
                                "defaults": c.defaults.iter().map(|b| b.to_config()).collect::<Vec<_>>(),
                                "conflicts": km.conflicts(i).iter().map(|&j| km.commands[j].id.clone()).collect::<Vec<_>>(),
                            })
                        })
                        .collect();
                    Ok(json!({ "commands": rows }))
                })();
                match r {
                    Ok(v) => ToolResult::json(v),
                    Err(m) => ToolResult::error(m),
                }
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_search",
            "Search everything",
            "Unity's Search window (⌘K): find assets (`t:Type l:Label`), scene objects (`t:Camera` finds objects \
             with that component), menu items and settings pages. Prefixes `p:` `h:` `m:` `set:` pick one provider. \
             `activate`: index of a result to select it (assets and objects) or run it (menus, settings); \
             `open: true` opens an asset instead. `show: true` opens the Search popup with this query.",
            schema(
                json!({ "query": { "type": "string" }, "activate": { "type": "integer", "minimum": 0 },
                       "open": { "type": "boolean" }, "show": { "type": "boolean" } }),
                &["query"],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                use crate::search_window::{Hit, activate, search};
                let q = args
                    .get("query")
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let hits = search(&mut ed.st, e, &q);
                if args.get("show").and_then(Json::as_bool) == Some(true) {
                    ed.st.search.open = true;
                    ed.st.search.query = q.clone();
                }
                let mut done = Json::Null;
                if let Some(i) = args.get("activate").and_then(Json::as_u64) {
                    let Some(hit) = hits.get(i as usize) else {
                        return ToolResult::error(format!(
                            "no result {i} ({} results)",
                            hits.len()
                        ));
                    };
                    let open = args.get("open").and_then(Json::as_bool).unwrap_or(false);
                    activate(&mut ed.st, e, hit, open);
                    let actions = std::mem::take(&mut ed.st.actions);
                    for a in actions {
                        if let Err(err) = ed.apply(e, a) {
                            return ToolResult::error(err);
                        }
                    }
                    done = json!(i);
                }
                let rows: Vec<Json> = hits
                    .iter()
                    .enumerate()
                    .map(|(i, h)| match h {
                        Hit::Asset(x) => json!({ "index": i, "provider": "Project", "path": x.path,
                                                 "type": crate::project::entry_kind(x).0, "labels": x.labels }),
                        Hit::Object { id, path } => json!({ "index": i, "provider": "Hierarchy", "path": path, "id": id.to_bits() }),
                        Hit::Menu { path, shortcut, .. } => json!({ "index": i, "provider": "Menu", "path": path, "shortcut": shortcut }),
                        Hit::Setting { name, .. } => json!({ "index": i, "provider": "Settings", "path": name }),
                    })
                    .collect();
                ToolResult::json(json!({ "results": rows, "activated": done }))
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_scripts",
            "Game scripts",
            "The project's Rust scripts: `status` (compile state, errors, script component types), `recompile`, or \
         `create` a new script component named `name` (Unity Create > Script). Scripts hot-reload when saved.",
            schema(
                json!({ "action": { "type": "string", "enum": ["status", "recompile", "create"] },
                       "name": { "type": "string" } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                match args
                    .get("action")
                    .and_then(Json::as_str)
                    .unwrap_or("status")
                {
                    "recompile" => {
                        let _ = ed.apply(e, Action::RecompileScripts);
                    }
                    "create" => {
                        let Some(name) = args.get("name").and_then(Json::as_str) else {
                            return ToolResult::error("`name` is required");
                        };
                        if let Err(err) = ed.apply(e, Action::CreateScript(name.to_owned())) {
                            return ToolResult::error(err);
                        }
                    }
                    _ => {}
                }
                let Some(h) = ed.scripts.as_ref() else {
                    return ToolResult::json(json!({ "available": false,
                    "note": "this editor was not opened on a project (use `oxide open <dir>`)" }));
                };
                ToolResult::json(json!({
                    "available": h.state != crate::scripting::ScriptState::None,
                    "state": format!("{:?}", h.state),
                    "compiling": h.is_compiling(),
                    "types": h.script_types,
                    "errors": h.errors,
                    "last_build_seconds": h.last_build_seconds,
                    "scripts_dir": h.scripts_dir(),
                }))
            })
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "editor_build",
            "Build the game",
            "Unity Build Settings: `status` (default), `settings` (read; or change with `scenes`, `product_name`, `width`, \
         `height`, `fullscreen`, `output`), `build` or `build_and_run` (starts in the background; poll `status`). \
         Builds a standalone release executable of the project with its Scripts and Assets.",
            schema(
                json!({ "action": { "type": "string", "enum": ["status", "settings", "build", "build_and_run"] },
                       "scenes": { "type": "array", "items": { "type": "string" } }, "product_name": { "type": "string" },
                       "width": { "type": "integer" }, "height": { "type": "integer" }, "fullscreen": { "type": "boolean" },
                       "output": { "type": "string" } }),
                &[],
            ),
        ),
        |args, e| {
            with_editor(e, |ed, e| {
                let project = e
                    .world
                    .resource::<oxide_assets::AssetServer>()
                    .full_path("");
                let mut settings = crate::player_build::BuildSettings::load(&project);
                let mut changed = false;
                if let Some(v) = args.get("scenes").and_then(Json::as_array) {
                    settings.scenes = v
                        .iter()
                        .filter_map(Json::as_str)
                        .map(str::to_owned)
                        .collect();
                    changed = true;
                }
                if let Some(v) = args.get("product_name").and_then(Json::as_str) {
                    settings.product_name = v.into();
                    changed = true;
                }
                if let Some(v) = args.get("output").and_then(Json::as_str) {
                    settings.output = v.into();
                    changed = true;
                }
                if let Some(v) = args.get("width").and_then(Json::as_u64) {
                    settings.width = v as u32;
                    changed = true;
                }
                if let Some(v) = args.get("height").and_then(Json::as_u64) {
                    settings.height = v as u32;
                    changed = true;
                }
                if let Some(v) = args.get("fullscreen").and_then(Json::as_bool) {
                    settings.fullscreen = v;
                    changed = true;
                }
                if changed && let Err(err) = settings.save(&project) {
                    return ToolResult::error(err);
                }
                let action = args
                    .get("action")
                    .and_then(Json::as_str)
                    .unwrap_or("status");
                if matches!(action, "build" | "build_and_run")
                    && let Err(err) = ed.apply(e, Action::BuildPlayer(action == "build_and_run"))
                {
                    return ToolResult::error(err);
                }
                let b = &ed.st.builder;
                let state = match &b.state {
                    crate::player_build::BuildState::Idle => json!({ "state": "idle" }),
                    crate::player_build::BuildState::Building => {
                        json!({ "state": "building", "seconds": b.elapsed() })
                    }
                    crate::player_build::BuildState::Succeeded(p) => {
                        json!({ "state": "succeeded", "executable": p })
                    }
                    crate::player_build::BuildState::Failed(err) => {
                        json!({ "state": "failed", "error": err })
                    }
                };
                ToolResult::json(json!({ "build": state, "settings": settings }))
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
