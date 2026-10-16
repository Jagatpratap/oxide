//! Main menu, toolbar, status bar and keyboard shortcuts.
//!
//! Menu items are data ([`menu_items`]) so the menu bar, the Hierarchy's create menu and the
//! MCP `editor_execute_menu` tool all run the same [`Action`]s.
//! Reference: <https://docs.unity3d.com/Manual/Toolbar.html>

use crate::{Action, Create, EditorState, PlayState, Tab, Tool};
use egui::{Key, KeyboardShortcut, Modifiers};
use oxide_assets::PrimitiveType;
use oxide_engine::Engine;
use oxide_input::{CursorLockMode, Input};
use oxide_scene::World;

pub struct MenuItem {
    /// `File/Save`; a last segment of `-` is a separator.
    pub path: String,
    pub shortcut: &'static str,
    pub action: Option<Action>,
}

fn item(path: &str, shortcut: &'static str, action: Action) -> MenuItem {
    MenuItem {
        path: path.to_owned(),
        shortcut,
        action: Some(action),
    }
}

fn sep(menu: &str) -> MenuItem {
    MenuItem {
        path: format!("{menu}/-"),
        shortcut: "",
        action: None,
    }
}

#[cfg(target_os = "macos")]
mod keys {
    pub const NEW: &str = "⌘N";
    pub const OPEN: &str = "⌘O";
    pub const SAVE: &str = "⌘S";
    pub const SAVE_AS: &str = "⇧⌘S";
    pub const UNDO: &str = "⌘Z";
    pub const REDO: &str = "⇧⌘Z";
    pub const ALL: &str = "⌘A";
    pub const DUP: &str = "⌘D";
    pub const DEL: &str = "⌫";
    pub const PLAY: &str = "⌘P";
    pub const PAUSE: &str = "⇧⌘P";
    pub const STEP: &str = "⌥⌘P";
    pub const EMPTY: &str = "⇧⌘N";
    pub const CHILD: &str = "⌥⇧N";
    pub const BUILD_SETTINGS: &str = "⇧⌘B";
    pub const BUILD_RUN: &str = "⌘B";
}

#[cfg(not(target_os = "macos"))]
mod keys {
    pub const NEW: &str = "Ctrl+N";
    pub const OPEN: &str = "Ctrl+O";
    pub const SAVE: &str = "Ctrl+S";
    pub const SAVE_AS: &str = "Ctrl+Shift+S";
    pub const UNDO: &str = "Ctrl+Z";
    pub const REDO: &str = "Ctrl+Y";
    pub const ALL: &str = "Ctrl+A";
    pub const DUP: &str = "Ctrl+D";
    pub const DEL: &str = "Del";
    pub const PLAY: &str = "Ctrl+P";
    pub const PAUSE: &str = "Ctrl+Shift+P";
    pub const STEP: &str = "Ctrl+Alt+P";
    pub const EMPTY: &str = "Ctrl+Shift+N";
    pub const CHILD: &str = "Alt+Shift+N";
    pub const BUILD_SETTINGS: &str = "Ctrl+Shift+B";
    pub const BUILD_RUN: &str = "Ctrl+B";
}

fn create_items(prefix: &str) -> Vec<MenuItem> {
    let mut v = vec![
        item(
            &format!("{prefix}Create Empty"),
            keys::EMPTY,
            Action::Create(Create::Empty),
        ),
        item(
            &format!("{prefix}Create Empty Child"),
            keys::CHILD,
            Action::Create(Create::EmptyChild),
        ),
    ];
    for p in [
        PrimitiveType::Cube,
        PrimitiveType::Sphere,
        PrimitiveType::Capsule,
        PrimitiveType::Cylinder,
        PrimitiveType::Plane,
        PrimitiveType::Quad,
    ] {
        v.push(item(
            &format!("{prefix}3D Object/{}", p.name()),
            "",
            Action::Create(Create::Primitive(p)),
        ));
    }
    v.push(item(
        &format!("{prefix}3D Object/Ragdoll..."),
        "",
        Action::CreateRagdoll,
    ));
    v.push(item(
        &format!("{prefix}Light/Directional Light"),
        "",
        Action::Create(Create::DirectionalLight),
    ));
    v.push(item(
        &format!("{prefix}Light/Point Light"),
        "",
        Action::Create(Create::PointLight),
    ));
    v.push(item(
        &format!("{prefix}Light/Spot Light"),
        "",
        Action::Create(Create::SpotLight),
    ));
    v.push(item(
        &format!("{prefix}Light/Reflection Probe"),
        "",
        Action::Create(Create::ReflectionProbe),
    ));
    v.push(item(
        &format!("{prefix}Light/Light Probe Group"),
        "",
        Action::Create(Create::LightProbeGroup),
    ));
    for (name, kind) in [
        ("Global Volume", Create::GlobalVolume),
        ("Box Volume", Create::BoxVolume),
        ("Sphere Volume", Create::SphereVolume),
    ] {
        v.push(item(
            &format!("{prefix}Volume/{name}"),
            "",
            Action::Create(kind),
        ));
    }
    v.push(item(
        &format!("{prefix}Camera"),
        "",
        Action::Create(Create::Camera),
    ));
    v
}

/// Every main-menu item, in menu order. `Component/...` comes from the component registry.
pub fn menu_items(world: &World) -> Vec<MenuItem> {
    let mut v = vec![
        item("File/New Scene", keys::NEW, Action::NewScene),
        item("File/Open Scene...", keys::OPEN, Action::OpenSceneDialog),
        sep("File"),
        item("File/Save", keys::SAVE, Action::SaveScene),
        item("File/Save As...", keys::SAVE_AS, Action::SaveSceneAsDialog),
        sep("File"),
        item(
            "File/Build Settings...",
            keys::BUILD_SETTINGS,
            Action::BuildSettingsDialog,
        ),
        item(
            "File/Build And Run",
            keys::BUILD_RUN,
            Action::BuildPlayer(true),
        ),
        item("Edit/Undo", keys::UNDO, Action::Undo),
        item("Edit/Redo", keys::REDO, Action::Redo),
        sep("Edit"),
        item("Edit/Select All", keys::ALL, Action::SelectAll),
        item("Edit/Duplicate", keys::DUP, Action::Duplicate),
        item("Edit/Delete", keys::DEL, Action::Delete),
        item("Edit/Rename", "F2", Action::Rename),
        item("Edit/Frame Selected", "F", Action::FrameSelected),
        sep("Edit"),
        item("Edit/Play", keys::PLAY, Action::TogglePlay),
        item("Edit/Pause", keys::PAUSE, Action::TogglePause),
        item("Edit/Step", keys::STEP, Action::Step),
        sep("Edit"),
        item("Edit/Tools/Hand", "Q", Action::SetTool(Tool::Hand)),
        item("Edit/Tools/Move", "W", Action::SetTool(Tool::Move)),
        item("Edit/Tools/Rotate", "E", Action::SetTool(Tool::Rotate)),
        item("Edit/Tools/Scale", "R", Action::SetTool(Tool::Scale)),
    ];
    v.extend(create_items("GameObject/"));
    v.push(sep("GameObject"));
    v.push(item(
        "GameObject/Prefab/Apply All Overrides",
        "",
        Action::ApplyPrefab,
    ));
    v.push(item(
        "GameObject/Prefab/Revert All Overrides",
        "",
        Action::RevertPrefab,
    ));
    v.push(item(
        "GameObject/Prefab/Unpack Completely",
        "",
        Action::UnpackPrefab,
    ));
    let mut comps: Vec<(String, String)> = world
        .registry
        .iter()
        .filter(|r| r.info.name != "Transform")
        .map(|r| (r.menu.clone(), r.info.name.to_owned()))
        .collect();
    comps.sort();
    for (menu, name) in comps {
        v.push(MenuItem {
            path: format!("Component/{menu}"),
            shortcut: "",
            action: Some(Action::AddComponent(name)),
        });
    }
    for t in Tab::ALL {
        v.push(item(
            &format!("Window/General/{}", t.title()),
            "",
            Action::OpenTab(t),
        ));
    }
    v.push(item(
        "Assets/Recompile Scripts",
        "",
        Action::RecompileScripts,
    ));
    for size in [100.0, 200.0, 500.0] {
        v.push(item(
            &format!("Tools/World Streaming/Split Scene into {size} m Cells"),
            "",
            Action::SplitIntoCells(size),
        ));
    }
    v.push(item(
        "Window/Rendering/Lighting/Generate Lighting",
        "",
        Action::GenerateLighting,
    ));
    v.push(item(
        "Window/Rendering/Lighting/Clear Baked Data",
        "",
        Action::ClearBakedLighting,
    ));
    v.push(sep("Window"));
    v.push(item("Window/Layouts/Default", "", Action::ResetLayout));
    v
}

/// Shows `items` (paths relative to this level), nesting submenus by path segment.
fn show_level(
    ui: &mut egui::Ui,
    out: &mut Vec<Action>,
    items: &[(Vec<String>, &'static str, Option<Action>)],
) {
    let mut i = 0;
    while i < items.len() {
        let (segs, shortcut, action) = &items[i];
        if segs.len() == 1 {
            if segs[0] == "-" {
                ui.separator();
            } else if let Some(a) = action {
                let b = egui::Button::new(&segs[0]).shortcut_text(*shortcut);
                if ui.add(b).clicked() {
                    out.push(a.clone());
                    ui.close();
                }
            }
            i += 1;
            continue;
        }
        let head = segs[0].clone();
        let mut sub = Vec::new();
        while i < items.len() && items[i].0.len() > 1 && items[i].0[0] == head {
            sub.push((items[i].0[1..].to_vec(), items[i].1, items[i].2.clone()));
            i += 1;
        }
        ui.menu_button(&head, |ui| show_level(ui, out, &sub));
    }
}

fn split(items: Vec<MenuItem>, prefix: &str) -> Vec<(Vec<String>, &'static str, Option<Action>)> {
    items
        .into_iter()
        .filter_map(|m| {
            let rest = m.path.strip_prefix(prefix)?;
            Some((
                rest.split('/').map(str::to_owned).collect(),
                m.shortcut,
                m.action,
            ))
        })
        .collect()
}

/// GameObject creation menu (Hierarchy "+" and context menus).
pub fn create_menu(st: &mut EditorState, ui: &mut egui::Ui) {
    show_level(ui, &mut st.actions, &split(create_items(""), ""));
}

pub fn menu_bar(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let items = menu_items(&engine.world);
    egui::MenuBar::new().ui(ui, |ui| {
        for top in [
            "File",
            "Edit",
            "Assets",
            "GameObject",
            "Component",
            "Tools",
            "Window",
        ] {
            let prefix = format!("{top}/");
            let level: Vec<_> = split(
                items
                    .iter()
                    .filter(|m| m.path.starts_with(&prefix))
                    .map(|m| MenuItem {
                        path: m.path.clone(),
                        shortcut: m.shortcut,
                        action: m.action.clone(),
                    })
                    .collect(),
                &prefix,
            );
            ui.menu_button(top, |ui| show_level(ui, &mut st.actions, &level));
        }
    });
}

pub fn toolbar(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let full = ui.max_rect();
    ui.horizontal(|ui| {
        for (tool, label, key) in [
            (Tool::Hand, "Hand", "Q"),
            (Tool::Move, "Move", "W"),
            (Tool::Rotate, "Rotate", "E"),
            (Tool::Scale, "Scale", "R"),
        ] {
            if ui
                .selectable_label(st.tool == tool, label)
                .on_hover_text(format!("{label} tool ({key})"))
                .clicked()
            {
                st.tool = tool;
            }
        }
        ui.separator();
        let space = if st.local_handles { "Local" } else { "Global" };
        if ui
            .button(space)
            .on_hover_text("Handle orientation")
            .clicked()
        {
            st.local_handles = !st.local_handles;
        }

        // Play / Pause / Step, centered.
        let playing = st.play == PlayState::Playing;
        let center = egui::Rect::from_center_size(full.center(), egui::vec2(120.0, full.height()));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(center)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                let accent = egui::Color32::from_rgb(70, 140, 220);
                let play = egui::Button::new(if playing { "■" } else { "▶" })
                    .min_size(egui::vec2(32.0, 20.0))
                    .fill(if playing {
                        accent
                    } else {
                        egui::Color32::from_rgb(70, 70, 70)
                    });
                if ui.add(play).on_hover_text("Play (⌘P)").clicked() {
                    st.actions.push(Action::TogglePlay);
                }
                let pause = egui::Button::new("⏸")
                    .min_size(egui::vec2(32.0, 20.0))
                    .selected(engine.paused && playing);
                if ui.add(pause).on_hover_text("Pause").clicked() {
                    st.actions.push(Action::TogglePause);
                }
                if ui
                    .add(egui::Button::new("⏭").min_size(egui::vec2(32.0, 20.0)))
                    .on_hover_text("Step")
                    .clicked()
                {
                    st.actions.push(Action::Step);
                }
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(format!("{:.0} FPS", engine.fps));
        });
    });
}

pub fn status_bar(
    st: &mut EditorState,
    engine: &mut Engine,
    scripts: Option<&crate::scripting::ScriptHost>,
    ui: &mut egui::Ui,
) {
    ui.horizontal(|ui| {
        if let Some(e) = st.console.last() {
            let col = match e.log_type {
                oxide_core::LogType::Log => egui::Color32::from_gray(190),
                oxide_core::LogType::Warning => egui::Color32::from_rgb(255, 200, 60),
                _ => egui::Color32::from_rgb(255, 90, 80),
            };
            let line = e.message.lines().next().unwrap_or("").to_owned();
            ui.label(egui::RichText::new(line).color(col).size(11.5));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mode = match (st.play, engine.paused) {
                (PlayState::Edit, _) => "Edit Mode",
                (PlayState::Playing, false) => "Play Mode",
                (PlayState::Playing, true) => "Paused",
            };
            ui.weak(mode);
            ui.separator();
            let bake = oxide_engine::lighting::status(&engine.world);
            if bake.running {
                ui.add(
                    egui::ProgressBar::new(bake.progress)
                        .desired_width(120.0)
                        .text(format!("Baking lighting {:.0}%", bake.progress * 100.0)),
                );
                ui.separator();
            }
            if st.builder.is_building() {
                ui.spinner();
                ui.label(
                    egui::RichText::new(format!("Building player {:.0}s", st.builder.elapsed()))
                        .size(11.5),
                );
                ui.separator();
            }
            if let Some(h) = scripts {
                use crate::scripting::ScriptState;
                match h.state {
                    ScriptState::Compiling => {
                        ui.spinner();
                        ui.label(egui::RichText::new("Compiling scripts").size(11.5));
                    }
                    ScriptState::Failed => {
                        ui.label(
                            egui::RichText::new(format!("Script errors ({})", h.errors.len()))
                                .color(egui::Color32::from_rgb(255, 90, 80))
                                .size(11.5),
                        );
                    }
                    ScriptState::Ready => {
                        ui.weak(format!("{} scripts", h.script_types.len()));
                    }
                    ScriptState::None => {}
                }
                ui.separator();
            }
            ui.weak(format!("{} objects", engine.world.len()));
        });
    });
}

/// Global shortcuts. Editor shortcuts are off while typing in a field or while the Game
/// view has the keyboard in Play mode (Esc gives it back).
pub fn shortcuts(st: &mut EditorState, engine: &mut Engine, ctx: &egui::Context) {
    let cmd = Modifiers::COMMAND;
    let fire =
        |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
    if fire(cmd | Modifiers::ALT, Key::P) {
        st.actions.push(Action::Step);
    } else if fire(cmd | Modifiers::SHIFT, Key::P) {
        st.actions.push(Action::TogglePause);
    } else if fire(cmd, Key::P) {
        st.actions.push(Action::TogglePlay);
    }
    let game_keys = st.play == PlayState::Playing && st.game_focused;
    if game_keys {
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            st.game_focused = false;
            let input = engine.world.resource_mut::<Input>();
            input.cursor_lock = CursorLockMode::None;
            input.cursor_visible = true;
            input.release_all();
        }
        return;
    }
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let pairs: [(Modifiers, Key, Action); 15] = [
        (cmd | Modifiers::SHIFT, Key::B, Action::BuildSettingsDialog),
        (cmd, Key::B, Action::BuildPlayer(true)),
        (cmd | Modifiers::SHIFT, Key::Z, Action::Redo),
        (cmd, Key::Z, Action::Undo),
        (cmd, Key::Y, Action::Redo),
        (cmd | Modifiers::SHIFT, Key::S, Action::SaveSceneAsDialog),
        (cmd, Key::S, Action::SaveScene),
        (
            cmd | Modifiers::SHIFT,
            Key::N,
            Action::Create(Create::Empty),
        ),
        (
            Modifiers::ALT | Modifiers::SHIFT,
            Key::N,
            Action::Create(Create::EmptyChild),
        ),
        (cmd, Key::N, Action::NewScene),
        (cmd, Key::O, Action::OpenSceneDialog),
        (cmd, Key::D, Action::Duplicate),
        (cmd, Key::A, Action::SelectAll),
        (cmd, Key::Backspace, Action::Delete),
        (Modifiers::NONE, Key::Delete, Action::Delete),
    ];
    for (m, k, a) in pairs {
        if fire(m, k) {
            st.actions.push(a);
            return;
        }
    }
    let plain = |k: Key| ctx.input(|i| i.modifiers.is_none() && i.key_pressed(k));
    if plain(Key::Backspace) {
        st.actions.push(Action::Delete);
    }
    if plain(Key::F) {
        st.actions.push(Action::FrameSelected);
    }
    if plain(Key::F2) {
        st.actions.push(Action::Rename);
    }
    if !st.scene_ui.flying {
        for (k, t) in [
            (Key::Q, Tool::Hand),
            (Key::W, Tool::Move),
            (Key::E, Tool::Rotate),
            (Key::R, Tool::Scale),
        ] {
            if plain(k) {
                st.tool = t;
            }
        }
    }
}
