//! Main menu, toolbar, status bar and keyboard shortcuts.
//!
//! Menu items are data ([`menu_items`]) so the menu bar, the Hierarchy's create menu and the
//! MCP `editor_execute_menu` tool all run the same [`Action`]s.
//! Reference: <https://docs.unity3d.com/Manual/Toolbar.html>

use crate::shortcuts::{Effect, Keymap};
use crate::{Action, Create, EditorState, PlayState, Tab, Tool};
use oxide_assets::PrimitiveType;
use oxide_engine::Engine;
use oxide_scene::World;

pub struct MenuItem {
    /// `File/Save`; a last segment of `-` is a separator.
    pub path: String,
    /// Shown next to the item (from the [`Keymap`]).
    pub shortcut: String,
    pub action: Option<Action>,
    /// Greyed out (an extension's validate function said no).
    pub enabled: bool,
}

fn item(path: &str, action: Action) -> MenuItem {
    MenuItem {
        path: path.to_owned(),
        shortcut: String::new(),
        action: Some(action),
        enabled: true,
    }
}

fn sep(menu: &str) -> MenuItem {
    MenuItem {
        path: format!("{menu}/-"),
        shortcut: String::new(),
        action: None,
        enabled: true,
    }
}

fn create_items(prefix: &str) -> Vec<MenuItem> {
    let mut v = vec![
        item(
            &format!("{prefix}Create Empty"),
            Action::Create(Create::Empty),
        ),
        item(
            &format!("{prefix}Create Empty Child"),
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
            Action::Create(Create::Primitive(p)),
        ));
    }
    v.push(item(
        &format!("{prefix}3D Object/Terrain"),
        Action::Create(Create::Terrain),
    ));
    v.push(item(
        &format!("{prefix}3D Object/Ragdoll..."),
        Action::CreateRagdoll,
    ));
    v.push(item(
        &format!("{prefix}3D Object/Text - TextMeshPro"),
        Action::Create(Create::Text3D),
    ));
    v.push(item(
        &format!("{prefix}UI/Text - TextMeshPro"),
        Action::Create(Create::TextUI),
    ));
    for s in ["Square", "Circle"] {
        v.push(item(
            &format!("{prefix}2D Object/Sprites/{s}"),
            Action::Create(Create::Sprite(s)),
        ));
    }
    v.push(item(
        &format!("{prefix}2D Object/Tilemap/Rectangular"),
        Action::Create(Create::Tilemap),
    ));
    v.push(item(
        &format!("{prefix}Light/Directional Light"),
        Action::Create(Create::DirectionalLight),
    ));
    v.push(item(
        &format!("{prefix}Light/Point Light"),
        Action::Create(Create::PointLight),
    ));
    v.push(item(
        &format!("{prefix}Light/Spot Light"),
        Action::Create(Create::SpotLight),
    ));
    v.push(item(
        &format!("{prefix}Light/Area Light"),
        Action::Create(Create::AreaLight),
    ));
    v.push(item(
        &format!("{prefix}Light/Reflection Probe"),
        Action::Create(Create::ReflectionProbe),
    ));
    v.push(item(
        &format!("{prefix}Light/Light Probe Group"),
        Action::Create(Create::LightProbeGroup),
    ));
    for (name, kind) in [
        ("Global Volume", Create::GlobalVolume),
        ("Box Volume", Create::BoxVolume),
        ("Sphere Volume", Create::SphereVolume),
    ] {
        v.push(item(
            &format!("{prefix}Volume/{name}"),
            Action::Create(kind),
        ));
    }
    for (name, kind) in [
        ("NavMesh Surface", Create::NavMeshSurface),
        ("NavMesh Modifier Volume", Create::NavMeshModifierVolume),
        ("NavMesh Link", Create::NavMeshLink),
    ] {
        v.push(item(&format!("{prefix}AI/{name}"), Action::Create(kind)));
    }
    v.push(item(
        &format!("{prefix}Effects/Particle System"),
        Action::Create(Create::ParticleSystem),
    ));
    for s in oxide_probuilder::ShapeType::ALL {
        v.push(item(
            &format!("{prefix}ProBuilder/{}", s.name()),
            Action::Create(Create::ProBuilderShape(s)),
        ));
    }
    v.push(item(
        &format!("{prefix}Visual Effects/Visual Effect"),
        Action::Create(Create::VisualEffect),
    ));
    v.push(item(
        &format!("{prefix}UI Toolkit/UI Document"),
        Action::Create(Create::UIDocument),
    ));
    v.push(item(
        &format!("{prefix}Camera"),
        Action::Create(Create::Camera),
    ));
    v
}

/// Every main-menu item, in menu order. `Component/...` comes from the component registry.
pub fn menu_items(world: &World, keymap: &Keymap) -> Vec<MenuItem> {
    let mut v = vec![
        item("File/New Project...", Action::OpenHub),
        item("File/Open Project...", Action::OpenHub),
        sep("File"),
        item("File/New Scene", Action::NewScene),
        item("File/Open Scene...", Action::OpenSceneDialog),
        sep("File"),
        item("File/Save", Action::SaveScene),
        item("File/Save As...", Action::SaveSceneAsDialog),
        sep("File"),
        item("File/Build Settings...", Action::BuildSettingsDialog),
        item("File/Build And Run", Action::BuildPlayer(true)),
        item("Edit/Undo", Action::Undo),
        item("Edit/Redo", Action::Redo),
        sep("Edit"),
        item("Edit/Select All", Action::SelectAll),
        item("Edit/Duplicate", Action::Duplicate),
        item("Edit/Delete", Action::Delete),
        item("Edit/Rename", Action::Rename),
        item("Edit/Frame Selected", Action::FrameSelected),
        item("Edit/Selection/Back", Action::SelectionBack),
        item("Edit/Selection/Forward", Action::SelectionForward),
        sep("Edit/Selection"),
        sep("Edit"),
        item("Edit/Play", Action::TogglePlay),
        item("Edit/Pause", Action::TogglePause),
        item("Edit/Step", Action::Step),
        sep("Edit"),
        item("Edit/Tools/Hand", Action::SetTool(Tool::Hand)),
        item("Edit/Tools/Move", Action::SetTool(Tool::Move)),
        item("Edit/Tools/Rotate", Action::SetTool(Tool::Rotate)),
        item("Edit/Tools/Scale", Action::SetTool(Tool::Scale)),
        item("Edit/Tools/Rect", Action::SetTool(Tool::Rect)),
        item("Edit/Tools/Transform", Action::SetTool(Tool::Transform)),
        sep("Edit"),
        item(
            "Edit/Project Settings...",
            Action::OpenProjectSettings(crate::settings_window::Page::default()),
        ),
        item("Edit/Preferences...", Action::OpenPreferences),
        item("Edit/Shortcuts...", Action::OpenTab(Tab::Shortcuts)),
        item("Edit/Search All...", Action::OpenSearch),
    ];
    for n in 1..=9 {
        v.push(item(
            &format!("Edit/Selection/Save Selection {n}"),
            Action::SaveSelection(n),
        ));
    }
    for n in 1..=9 {
        v.push(item(
            &format!("Edit/Selection/Load Selection {n}"),
            Action::LoadSelection(n),
        ));
    }
    v.extend(create_items("GameObject/"));
    v.push(sep("GameObject"));
    v.push(item(
        "GameObject/Prefab/Apply All Overrides",
        Action::ApplyPrefab,
    ));
    v.push(item(
        "GameObject/Prefab/Revert All Overrides",
        Action::RevertPrefab,
    ));
    v.push(item(
        "GameObject/Prefab/Unpack Completely",
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
            shortcut: String::new(),
            action: Some(Action::AddComponent(name)),
            enabled: true,
        });
    }
    for t in Tab::ALL {
        v.push(item(
            &format!("Window/General/{}", t.title()),
            Action::OpenTab(t),
        ));
    }
    v.push(item(
        "Window/Animation/Animation",
        Action::OpenTab(Tab::Animation),
    ));
    v.push(item(
        "Window/Animation/Animator",
        Action::OpenTab(Tab::Animator),
    ));
    v.push(item(
        "Assets/Import Package/Custom Package…",
        Action::ImportPackage(None),
    ));
    v.push(item(
        "Assets/Export Package…",
        Action::ExportPackage(Vec::new()),
    ));
    v.push(item("Assets/Refresh", Action::RefreshAssets));
    v.push(item("Assets/Recompile Scripts", Action::RecompileScripts));
    for size in [100.0, 200.0, 500.0] {
        v.push(item(
            &format!("Tools/World Streaming/Split Scene into {size} m Cells"),
            Action::SplitIntoCells(size),
        ));
    }
    v.push(item(
        "Window/Sequencing/Timeline",
        Action::OpenTab(Tab::Timeline),
    ));
    v.push(item(
        "Window/AI/Navigation",
        Action::OpenTab(Tab::Navigation),
    ));
    v.push(item(
        "Window/UI Toolkit/UI Builder",
        Action::OpenTab(Tab::UiBuilder),
    ));
    v.push(item(
        "Window/Asset Management/Localization Tables",
        Action::OpenTab(Tab::LocalizationTables),
    ));
    v.push(item(
        "Tools/ProBuilder/ProBuilder Window",
        Action::OpenTab(Tab::ProBuilder),
    ));
    v.push(item(
        "Tools/ProBuilder/Editors/UV Editor",
        Action::OpenTab(Tab::UvEditor),
    ));
    v.push(item(
        "Window/Rendering/Lighting/Lighting",
        Action::OpenTab(Tab::Lighting),
    ));
    v.push(item(
        "Window/General/Test Runner",
        Action::OpenTab(Tab::TestRunner),
    ));
    v.push(item(
        "Window/Rendering/Light Explorer",
        Action::OpenTab(Tab::LightExplorer),
    ));
    v.push(item(
        "Window/TextMeshPro/Font Asset Creator",
        Action::OpenTab(Tab::FontAssetCreator),
    ));
    v.push(item(
        "Window/Terrain/Terrain Toolbox",
        Action::OpenTab(Tab::TerrainToolbox),
    ));
    v.push(item(
        "Window/Package Manager",
        Action::OpenTab(Tab::PackageManager),
    ));
    v.push(item(
        "Window/Analysis/Profiler",
        Action::OpenTab(Tab::Profiler),
    ));
    v.push(item(
        "Window/Analysis/Frame Debugger",
        Action::OpenTab(Tab::FrameDebugger),
    ));
    v.push(item(
        "Window/Analysis/Adaptive Performance",
        Action::OpenTab(Tab::AdaptivePerformance),
    ));
    v.push(item(
        "Window/Analysis/Project Auditor",
        Action::OpenTab(Tab::ProjectAuditor),
    ));
    v.push(item(
        "Window/Rendering/Lighting/Generate Lighting",
        Action::GenerateLighting,
    ));
    v.push(item(
        "Window/Rendering/Lighting/Clear Baked Data",
        Action::ClearBakedLighting,
    ));
    v.push(sep("Window"));
    v.push(item("Window/Layouts/Default", Action::ResetLayout));
    let mut v = with_shortcuts(v, keymap);
    // Script editor extensions: custom windows and `[MenuItem]`s.
    for (path, action, shortcut) in crate::extensions::menu_items(world) {
        v.retain(|m| m.path != path);
        v.push(MenuItem {
            path,
            shortcut: shortcut.unwrap_or_default(),
            action: Some(action),
            enabled: true,
        });
    }
    v
}

/// The menu bar's top-level menus: the built-in ones, then any new ones extensions add.
pub fn top_level_menus(items: &[MenuItem]) -> Vec<String> {
    let mut tops: Vec<String> = [
        "File",
        "Edit",
        "Assets",
        "GameObject",
        "Component",
        "Tools",
        "Window",
    ]
    .map(str::to_owned)
    .to_vec();
    for m in items {
        if let Some((top, _)) = m.path.split_once('/')
            && !tops.iter().any(|t| t == top)
        {
            // Before Window, like Unity's custom menus.
            let at = tops.len() - 1;
            tops.insert(at, top.to_owned());
        }
    }
    tops
}

fn with_shortcuts(mut v: Vec<MenuItem>, keymap: &Keymap) -> Vec<MenuItem> {
    for m in &mut v {
        if let Some(a) = &m.action {
            m.shortcut = keymap.action_text(a);
        }
    }
    v
}

/// Shows `items` (paths relative to this level), nesting submenus by path segment.
fn show_level(ui: &mut egui::Ui, out: &mut Vec<Action>, items: &[Level]) {
    let mut i = 0;
    while i < items.len() {
        let (segs, shortcut, action, enabled) = &items[i];
        if segs.len() == 1 {
            if segs[0] == "-" {
                ui.separator();
            } else if let Some(a) = action {
                let b = egui::Button::new(&segs[0]).shortcut_text(shortcut.as_str());
                if ui.add_enabled(*enabled, b).clicked() {
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
            sub.push((
                items[i].0[1..].to_vec(),
                items[i].1.clone(),
                items[i].2.clone(),
                items[i].3,
            ));
            i += 1;
        }
        ui.menu_button(&head, |ui| show_level(ui, out, &sub));
    }
}

/// A menu entry relative to its level: path segments, shortcut, action, enabled.
type Level = (Vec<String>, String, Option<Action>, bool);

fn split(items: Vec<MenuItem>, prefix: &str) -> Vec<Level> {
    items
        .into_iter()
        .filter_map(|m| {
            let rest = m.path.strip_prefix(prefix)?;
            Some((
                rest.split('/').map(str::to_owned).collect(),
                m.shortcut,
                m.action,
                m.enabled,
            ))
        })
        .collect()
}

/// GameObject creation menu (Hierarchy "+" and context menus).
pub fn create_menu(st: &mut EditorState, ui: &mut egui::Ui) {
    let items = with_shortcuts(create_items(""), &st.keymap);
    show_level(ui, &mut st.actions, &split(items, ""));
}

pub fn menu_bar(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let mut items = menu_items(&engine.world, &st.keymap);
    let tops = top_level_menus(&items);
    // Extension items' validate functions (only while a menu is open: they run script code).
    if egui::Popup::is_any_open(ui.ctx()) {
        for m in &mut items {
            if let Some(Action::ExtensionMenu(p)) = &m.action {
                m.enabled = crate::extensions::validate(st, &mut engine.world, p);
            }
        }
    }
    egui::MenuBar::new().ui(ui, |ui| {
        for top in &tops {
            let prefix = format!("{top}/");
            let level: Vec<_> = split(
                items
                    .iter()
                    .filter(|m| m.path.starts_with(&prefix))
                    .map(|m| MenuItem {
                        path: m.path.clone(),
                        shortcut: m.shortcut.clone(),
                        action: m.action.clone(),
                        enabled: m.enabled,
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
        for (tool, label) in [
            (Tool::Hand, "Hand"),
            (Tool::Move, "Move"),
            (Tool::Rotate, "Rotate"),
            (Tool::Scale, "Scale"),
            (Tool::Rect, "Rect"),
            (Tool::Transform, "Transform"),
        ] {
            let key = st.keymap.action_text(&Action::SetTool(tool));
            if ui
                .selectable_label(st.tool == tool, label)
                .on_hover_text(format!("{label} tool ({key})"))
                .clicked()
            {
                st.tool = tool;
            }
        }
        ui.separator();
        let pivot = if st.center_handles { "Center" } else { "Pivot" };
        if ui
            .button(pivot)
            .on_hover_text(format!(
                "Handle position: the active object's pivot or the selection's center ({})",
                st.keymap.text_for(&Effect::TogglePivot)
            ))
            .clicked()
        {
            st.center_handles = !st.center_handles;
        }
        let space = if st.local_handles { "Local" } else { "Global" };
        if ui
            .button(space)
            .on_hover_text(format!(
                "Handle orientation ({})",
                st.keymap.text_for(&Effect::ToggleLocal)
            ))
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
