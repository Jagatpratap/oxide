//! Oxide editor: Unity's main window around the running engine.
//!
//! Docked windows (Hierarchy, Scene, Game, Inspector, Project, Console), the main menu and
//! toolbar, Undo/Redo, and Play mode (the scene is snapshotted on Play and restored on Stop).
//! Every menu item is an [`Action`], so menus, shortcuts and the MCP tools in [`mcp`] share
//! one code path.
//!
//! References: <https://docs.unity3d.com/Manual/UsingTheEditor.html>,
//! <https://docs.unity3d.com/Manual/GameView.html>, <https://docs.unity3d.com/Manual/UndoWindow.html>

mod console;
mod hierarchy;
mod inspector;
pub mod launch;
pub mod mcp;
mod menus;
mod project;
pub mod project_files;
mod scene_view;
pub mod scripting;
mod theme;
mod undo;
mod view;

pub use view::SceneCamera;

use egui_dock::{DockArea, DockState, NodeIndex};
use oxide_assets::{AssetServer, PrimitiveType};
use oxide_core::{Time, debug_log, debug_log_warning};
use oxide_engine::{Engine, GameViewport, Shell, objects};
use oxide_input::{CursorLockMode, Input};
use oxide_math::{Color, Quaternion, QuaternionExt, Vector3};
use oxide_scene::{GameObjectId, Light, LightType, World};
use oxide_ui::Gui;

/// A dockable editor window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Scene,
    Game,
    Hierarchy,
    Inspector,
    Project,
    Console,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Scene,
        Tab::Game,
        Tab::Hierarchy,
        Tab::Inspector,
        Tab::Project,
        Tab::Console,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Scene => "Scene",
            Tab::Game => "Game",
            Tab::Hierarchy => "Hierarchy",
            Tab::Inspector => "Inspector",
            Tab::Project => "Project",
            Tab::Console => "Console",
        }
    }
}

/// Scene view transform tool (Unity's Q/W/E/R).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Hand,
    Move,
    Rotate,
    Scale,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Hand => "hand",
            Tool::Move => "move",
            Tool::Rotate => "rotate",
            Tool::Scale => "scale",
        }
    }

    pub fn parse(s: &str) -> Option<Tool> {
        [Tool::Hand, Tool::Move, Tool::Rotate, Tool::Scale]
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(s))
    }
}

/// Objects the GameObject menu can create.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Create {
    Empty,
    EmptyChild,
    Primitive(PrimitiveType),
    DirectionalLight,
    PointLight,
    SpotLight,
    Camera,
}

/// Everything the menus, shortcuts and MCP can ask the editor to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    NewScene,
    OpenScene(String),
    SaveScene,
    SaveSceneAs(String),
    /// Asks for a path, then saves.
    SaveSceneAsDialog,
    OpenSceneDialog,
    Undo,
    Redo,
    TogglePlay,
    TogglePause,
    Step,
    Create(Create),
    Duplicate,
    Delete,
    SelectAll,
    FrameSelected,
    Rename,
    AddComponent(String),
    SetTool(Tool),
    OpenTab(Tab),
    ResetLayout,
    InstantiateModel(String),
    InstantiatePrefab(String),
    /// Saves the active object as a prefab at this path.
    CreatePrefab(String),
    CreatePrefabVariant(String),
    ApplyPrefab,
    RevertPrefab,
    UnpackPrefab,
    /// New script component in the project's Scripts (name as typed).
    CreateScript(String),
    RecompileScripts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Edit,
    Playing,
}

enum Modal {
    SaveAs(String),
    Open,
}

/// Editor state the windows read and write (everything except the dock layout).
pub struct EditorState {
    /// Selected objects; the last one is the active object (shown in the Inspector).
    pub selection: Vec<GameObjectId>,
    pub tool: Tool,
    /// Handles follow the object's rotation (Unity "Local") instead of world axes.
    pub local_handles: bool,
    pub play: PlayState,
    play_snapshot: Option<undo::Snapshot>,
    /// Undo depth when Play started; entries recorded during Play are dropped on Stop.
    play_undo_depth: usize,
    pub undo: undo::UndoStack,
    pub scene_cam: SceneCamera,
    scene_view: view::ViewTarget,
    game_view: view::ViewTarget,
    /// The Game view has keyboard focus (game input is live).
    pub game_focused: bool,
    /// Unsaved changes in the open scene.
    pub dirty: bool,
    /// Requested from inside window code; applied after the UI pass.
    pub actions: Vec<Action>,
    hierarchy: hierarchy::HierarchyState,
    inspector: inspector::InspectorState,
    console: console::ConsoleState,
    project: project::ProjectState,
    scene_ui: scene_view::SceneViewState,
    /// Game view aspect (`None` = free aspect).
    game_aspect: Option<f32>,
    game_stats: bool,
    modal: Option<Modal>,
    title: String,
    /// Status messages from actions (shown in the status bar).
    pub status: String,
    /// The project has a Scripts crate (Create > Rust Script is available).
    pub scripts_available: bool,
}

impl EditorState {
    fn new() -> Self {
        Self {
            selection: Vec::new(),
            tool: Tool::Move,
            local_handles: false,
            play: PlayState::Edit,
            play_snapshot: None,
            play_undo_depth: 0,
            undo: undo::UndoStack::default(),
            scene_cam: SceneCamera::default(),
            scene_view: view::ViewTarget::default(),
            game_view: view::ViewTarget::default(),
            game_focused: false,
            dirty: false,
            actions: Vec::new(),
            hierarchy: Default::default(),
            inspector: Default::default(),
            console: Default::default(),
            project: Default::default(),
            scene_ui: Default::default(),
            game_aspect: None,
            game_stats: false,
            modal: None,
            title: String::new(),
            status: String::new(),
            scripts_available: false,
        }
    }

    pub fn active(&self) -> Option<GameObjectId> {
        self.selection.last().copied()
    }

    pub fn is_selected(&self, id: GameObjectId) -> bool {
        self.selection.contains(&id)
    }

    pub fn select(&mut self, id: GameObjectId) {
        self.selection = vec![id];
    }

    /// Shift/Cmd-click: adds, or removes if already selected.
    pub fn toggle_select(&mut self, id: GameObjectId) {
        if let Some(i) = self.selection.iter().position(|&s| s == id) {
            self.selection.remove(i);
        } else {
            self.selection.push(id);
        }
    }

    /// Selected objects without the ones whose ancestor is also selected (so a move applies
    /// once per hierarchy).
    pub fn top_level_selection(&self, world: &World) -> Vec<GameObjectId> {
        self.selection
            .iter()
            .copied()
            .filter(|&id| world.is_alive(id))
            .filter(|&id| {
                let mut p = world.parent(id);
                while let Some(x) = p {
                    if self.selection.contains(&x) {
                        return false;
                    }
                    p = world.parent(x);
                }
                true
            })
            .collect()
    }

    /// Records the scene before an edit (Unity `Undo.RecordObject`). Edits with the same
    /// `coalesce` key in quick succession (one drag) become one undo step.
    pub fn record(&mut self, world: &World, label: &str, coalesce: Option<u64>, now: f64) {
        if self
            .undo
            .record(world, label, &self.selection, coalesce, now)
        {
            self.dirty = true;
        }
    }

    fn playing(&self) -> bool {
        self.play == PlayState::Playing
    }
}

/// The editor shell. Install with `App::shell(Box::new(Editor::new()))`.
pub struct Editor {
    dock: DockState<Tab>,
    pub st: EditorState,
    styled: bool,
    /// Last saved layout (JSON) and when it was last checked.
    saved_layout: Option<String>,
    layout_checked: f64,
    /// Compiles and hot-reloads the project's Scripts (only in a project editor that links
    /// the engine dynamically).
    pub scripts: Option<scripting::ScriptHost>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Scene, Tab::Game]);
    let tree = dock.main_surface_mut();
    let [center, _] = tree.split_right(NodeIndex::root(), 0.77, vec![Tab::Inspector]);
    let [center, _] = tree.split_below(center, 0.68, vec![Tab::Project, Tab::Console]);
    // For left/above splits egui_dock's `fraction` is the new (left/top) node's share.
    let [_, _] = tree.split_left(center, 0.24, vec![Tab::Hierarchy]);
    dock
}

struct Viewer<'a> {
    st: &'a mut EditorState,
    engine: &'a mut Engine,
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(("oxide-tab", *tab))
    }

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Scene => scene_view::ui(self.st, self.engine, ui),
            Tab::Game => game_view_ui(self.st, self.engine, ui),
            Tab::Hierarchy => hierarchy::ui(self.st, self.engine, ui),
            Tab::Inspector => inspector::ui(self.st, self.engine, ui),
            Tab::Project => project::ui(self.st, self.engine, ui),
            Tab::Console => console::ui(self.st, ui),
        }
    }

    fn scroll_bars(&self, _: &Tab) -> [bool; 2] {
        [false, false]
    }

    fn clear_background(&self, tab: &Tab) -> bool {
        !matches!(tab, Tab::Scene | Tab::Game)
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            dock: default_layout(),
            st: EditorState::new(),
            styled: false,
            saved_layout: None,
            layout_checked: 0.0,
            scripts: None,
        }
    }

    /// Enables script compilation and hot reload.
    pub fn with_scripts(mut self, host: scripting::ScriptHost) -> Self {
        self.scripts = Some(host);
        self
    }

    fn layout_path(engine: &Engine) -> std::path::PathBuf {
        engine
            .world
            .resource::<AssetServer>()
            .full_path("Library/Oxide/EditorLayout.json")
    }

    /// Restores the window layout saved by a previous session (Unity keeps it in Library/).
    fn load_layout(&mut self, engine: &Engine) {
        let Ok(text) = std::fs::read_to_string(Self::layout_path(engine)) else {
            return;
        };
        match serde_json::from_str::<DockState<Tab>>(&text) {
            Ok(dock) if dock.iter_all_tabs().next().is_some() => {
                self.dock = dock;
                self.saved_layout = Some(text);
            }
            Ok(_) => {}
            Err(e) => debug_log_warning!("ignoring saved editor layout: {e}"),
        }
    }

    /// Saves the layout when it changed (checked every couple of seconds).
    fn save_layout(&mut self, engine: &Engine) {
        let now = oxide_core::time::realtime_since_startup();
        if now - self.layout_checked < 2.0 {
            return;
        }
        self.layout_checked = now;
        let Ok(text) = serde_json::to_string(&self.dock) else {
            return;
        };
        if self.saved_layout.as_deref() == Some(text.as_str()) {
            return;
        }
        let path = Self::layout_path(engine);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&path, &text).is_ok() {
            self.saved_layout = Some(text);
        }
    }

    pub fn open_tabs(&self) -> Vec<Tab> {
        self.dock.iter_all_tabs().map(|(_, t)| *t).collect()
    }

    fn focus_tab(&mut self, tab: Tab) {
        let found = self
            .dock
            .iter_all_tabs()
            .find(|(_, t)| **t == tab)
            .map(|(p, _)| p);
        if let Some(path) = found {
            let _ = self.dock.set_active_tab(path);
        } else {
            self.dock.push_to_focused_leaf(tab);
        }
    }

    /// Runs one editor action. Returns a short description of what happened.
    pub fn apply(&mut self, engine: &mut Engine, action: Action) -> Result<String, String> {
        let now = oxide_core::time::realtime_since_startup();
        let st = &mut self.st;
        let w = &mut engine.world;
        let msg = match action {
            Action::NewScene => {
                if st.playing() {
                    return Err("stop Play mode first".into());
                }
                w.clear(false);
                w.scene_name = "Untitled".into();
                w.scene_path = None;
                objects::create_default_scene(w);
                st.selection.clear();
                st.undo.clear();
                st.dirty = false;
                engine.start_systems();
                "New scene".into()
            }
            Action::OpenScene(path) => {
                if st.playing() {
                    return Err("stop Play mode first".into());
                }
                let warnings = oxide_engine::load_scene(w, &path, false)?;
                launch::remember_scene(&w.resource::<AssetServer>().full_path(""), &path);
                st.selection.clear();
                st.undo.clear();
                st.dirty = false;
                engine.start_systems();
                format!("Opened {path} ({} warnings)", warnings.len())
            }
            Action::SaveScene => match w.scene_path.clone() {
                Some(path) => {
                    let n = oxide_engine::save_scene(w, &path)?;
                    launch::remember_scene(&w.resource::<AssetServer>().full_path(""), &path);
                    st.dirty = false;
                    format!("Saved {n} objects to {path}")
                }
                None => {
                    st.modal = Some(Modal::SaveAs(format!(
                        "Assets/Scenes/{}.scene",
                        w.scene_name
                    )));
                    "Choose where to save".into()
                }
            },
            Action::SaveSceneAs(path) => {
                let n = oxide_engine::save_scene(w, &path)?;
                launch::remember_scene(&w.resource::<AssetServer>().full_path(""), &path);
                st.dirty = false;
                format!("Saved {n} objects to {path}")
            }
            Action::SaveSceneAsDialog => {
                let p = w
                    .scene_path
                    .clone()
                    .unwrap_or_else(|| format!("Assets/Scenes/{}.scene", w.scene_name));
                st.modal = Some(Modal::SaveAs(p));
                String::new()
            }
            Action::OpenSceneDialog => {
                st.modal = Some(Modal::Open);
                String::new()
            }
            Action::Undo => match st.undo.undo(w, &st.selection) {
                Some((label, sel)) => {
                    st.selection = sel;
                    st.dirty = true;
                    engine.start_systems();
                    format!("Undo {label}")
                }
                None => return Err("nothing to undo".into()),
            },
            Action::Redo => match st.undo.redo(w, &st.selection) {
                Some((label, sel)) => {
                    st.selection = sel;
                    st.dirty = true;
                    engine.start_systems();
                    format!("Redo {label}")
                }
                None => return Err("nothing to redo".into()),
            },
            Action::TogglePlay => {
                if st.playing() {
                    self.exit_play(engine);
                    "Stopped".into()
                } else {
                    self.enter_play(engine);
                    "Playing".into()
                }
            }
            Action::TogglePause => {
                engine.paused = !engine.paused;
                if engine.paused { "Paused" } else { "Resumed" }.into()
            }
            Action::Step => {
                if !st.playing() {
                    self.enter_play(engine);
                }
                engine.paused = true;
                engine.pending_steps += 1;
                "Step".into()
            }
            Action::Create(kind) => {
                st.record(w, "Create Object", None, now);
                let id = create(w, kind, st.active());
                // Unity places new root objects at the Scene view pivot.
                if w.parent(id).is_none()
                    && !matches!(kind, Create::DirectionalLight)
                    && let Some(t) = w.transform_mut(id)
                {
                    t.local_position = st.scene_cam.pivot;
                }
                st.select(id);
                st.hierarchy.reveal(w, id);
                st.hierarchy.renaming = None;
                format!("Created {}", w.get(id).map_or("", |o| o.name.as_str()))
            }
            Action::Duplicate => {
                let tops = st.top_level_selection(w);
                if tops.is_empty() {
                    return Err("nothing selected".into());
                }
                st.record(w, "Duplicate", None, now);
                let mut new = Vec::new();
                for id in tops {
                    let parent = w.parent(id);
                    let index = w.sibling_index(id);
                    if let Some(c) = w.instantiate(id) {
                        if let Some(p) = parent {
                            let _ = w.set_parent(c, Some(p), false);
                        }
                        w.set_sibling_index(c, index + 1);
                        let base = w.get(id).map(|o| o.name.clone()).unwrap_or_default();
                        if let Some(o) = w.get_mut(c) {
                            o.name = unique_name(&base);
                        }
                        new.push(c);
                    }
                }
                let n = new.len();
                st.selection = new;
                format!("Duplicated {n} object(s)")
            }
            Action::Delete => {
                let tops = st.top_level_selection(w);
                if tops.is_empty() {
                    return Err("nothing selected".into());
                }
                st.record(w, "Delete", None, now);
                for id in &tops {
                    w.destroy_immediate(*id);
                }
                st.selection.clear();
                format!("Deleted {} object(s)", tops.len())
            }
            Action::SelectAll => {
                st.selection = w.hierarchy_order();
                format!("Selected {}", st.selection.len())
            }
            Action::FrameSelected => {
                st.scene_ui.frame_request = true;
                String::new()
            }
            Action::Rename => {
                if let Some(id) = st.active() {
                    let name = w.get(id).map(|o| o.name.clone()).unwrap_or_default();
                    st.hierarchy.renaming = Some((id, name));
                }
                String::new()
            }
            Action::AddComponent(type_name) => {
                let targets: Vec<_> = st.selection.clone();
                if targets.is_empty() {
                    return Err("select an object first".into());
                }
                let Some(reg) = w.registry.get(&type_name) else {
                    return Err(format!("unknown component `{type_name}`"));
                };
                let name = reg.info.name;
                st.record(w, &format!("Add {name}"), None, now);
                let mut added = 0;
                let mut last_err = String::new();
                for id in targets {
                    match w.add_component_by_name(id, name) {
                        Ok(_) => added += 1,
                        Err(e) => last_err = e,
                    }
                }
                if added == 0 {
                    return Err(format!("{name} could not be added: {last_err}"));
                }
                format!("Added {name}")
            }
            Action::SetTool(t) => {
                st.tool = t;
                String::new()
            }
            Action::OpenTab(tab) => {
                self.focus_tab(tab);
                return Ok(format!("Opened {}", tab.title()));
            }
            Action::ResetLayout => {
                self.dock = default_layout();
                return Ok("Layout reset".into());
            }
            Action::InstantiatePrefab(path) => {
                st.record(w, "Instantiate Prefab", None, now);
                let id = w.instantiate_prefab(&path, None)?;
                w.set_position(id, st.scene_cam.pivot);
                st.select(id);
                format!("Instantiated {path}")
            }
            Action::CreatePrefab(path) => {
                let id = st.active().ok_or("select an object first")?;
                st.record(w, "Create Prefab", None, now);
                w.create_prefab(id, &path)?;
                st.project.cache = None;
                format!("Created prefab {path}")
            }
            Action::CreatePrefabVariant(path) => {
                let id = st.active().ok_or("select an object first")?;
                let root = w
                    .prefab_instance_root(id)
                    .ok_or("the selection is not a prefab instance")?;
                st.record(w, "Create Prefab Variant", None, now);
                w.create_prefab_variant(root, &path)?;
                st.project.cache = None;
                format!("Created prefab variant {path}")
            }
            Action::ApplyPrefab => {
                let id = st.active().ok_or("select a prefab instance first")?;
                st.record(w, "Apply Prefab", None, now);
                let n = w.apply_prefab_instance(id)?;
                st.inspector.invalidate_overrides();
                format!("Applied prefab; {n} other instance(s) updated")
            }
            Action::RevertPrefab => {
                let id = st.active().ok_or("select a prefab instance first")?;
                st.record(w, "Revert Prefab", None, now);
                w.revert_prefab_instance(id)?;
                st.inspector.invalidate_overrides();
                "Reverted prefab overrides".into()
            }
            Action::UnpackPrefab => {
                let id = st.active().ok_or("select a prefab instance first")?;
                let root = w
                    .prefab_instance_root(id)
                    .ok_or("the selection is not a prefab instance")?;
                st.record(w, "Unpack Prefab", None, now);
                w.unpack_prefab(root);
                st.inspector.invalidate_overrides();
                "Unpacked prefab".into()
            }
            Action::CreateScript(name) => {
                let host = self.scripts.as_ref().ok_or("this project has no Scripts")?;
                let ty = project_files::type_name(&name)
                    .ok_or_else(|| format!("`{name}` is not a valid script name"))?;
                let file = host.create_script(&ty)?;
                format!("Created script {ty} ({}); compiling...", file.display())
            }
            Action::RecompileScripts => {
                let host = self.scripts.as_mut().ok_or("this project has no Scripts")?;
                host.recompile();
                "Compiling scripts...".into()
            }
            Action::InstantiateModel(path) => {
                let h = w
                    .resource_mut::<AssetServer>()
                    .load_model(&path)
                    .map_err(|e| format!("cannot import {path}: {e}"))?;
                st.record(w, "Instantiate Model", None, now);
                let id = objects::instantiate_model(w, &h).ok_or("model has no nodes")?;
                st.select(id);
                format!("Instantiated {path}")
            }
        };
        if !msg.is_empty() {
            debug_log!("{msg}");
            self.st.status = msg.clone();
        }
        Ok(msg)
    }

    fn enter_play(&mut self, engine: &mut Engine) {
        let st = &mut self.st;
        let w = &mut engine.world;
        st.play_snapshot = Some(undo::Snapshot::take(w, "Play", &st.selection));
        st.play_undo_depth = st.undo.depth();
        if st.console.clear_on_play {
            oxide_core::debug::clear();
            st.console.clear();
        }
        // Fresh game clock, keeping the project's fixed timestep and time scale.
        let (fixed, scale) = {
            let t = w.resource::<Time>();
            (t.fixed_delta_time(), t.time_scale())
        };
        let mut t = Time::new();
        t.set_fixed_delta_time(fixed);
        t.set_time_scale(scale);
        w.resources.insert(t);
        w.start_scripts();
        engine.start_systems();
        engine.simulate = true;
        engine.paused = false;
        st.play = PlayState::Playing;
        st.game_focused = true;
        self.focus_tab(Tab::Game);
    }

    fn exit_play(&mut self, engine: &mut Engine) {
        let st = &mut self.st;
        engine.simulate = false;
        engine.paused = false;
        engine.pending_steps = 0;
        let w = &mut engine.world;
        // Scripts see OnDisable/OnDestroy as the play-mode objects go away.
        w.clear(false);
        w.run_scripts = false;
        if let Some(snap) = st.play_snapshot.take() {
            st.selection = snap.restore(w);
        }
        st.undo.truncate(st.play_undo_depth);
        let input = w.resource_mut::<Input>();
        input.release_all();
        input.cursor_lock = CursorLockMode::None;
        input.cursor_visible = true;
        engine.start_systems();
        st.play = PlayState::Edit;
        st.game_focused = false;
    }

    fn modal_ui(&mut self, ctx: &egui::Context, engine: &mut Engine) {
        let Some(modal) = self.st.modal.as_mut() else {
            return;
        };
        let mut close = false;
        let mut action = None;
        match modal {
            Modal::SaveAs(path) => {
                egui::Modal::new(egui::Id::new("save-as")).show(ctx, |ui| {
                    ui.heading("Save Scene As");
                    ui.label("Path (relative to the project):");
                    let r = ui.add(egui::TextEdit::singleline(path).desired_width(360.0));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() || enter {
                            let mut p = path.trim().to_owned();
                            if !p.ends_with(".scene") {
                                p.push_str(".scene");
                            }
                            action = Some(Action::SaveSceneAs(p));
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Modal::Open => {
                let root = engine.world.resource::<AssetServer>().full_path("");
                let scenes = project::find_files(&root, "scene");
                egui::Modal::new(egui::Id::new("open-scene")).show(ctx, |ui| {
                    ui.heading("Open Scene");
                    if scenes.is_empty() {
                        ui.label("No .scene files in this project yet.");
                    }
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for s in &scenes {
                                if ui.button(s).clicked() {
                                    action = Some(Action::OpenScene(s.clone()));
                                    close = true;
                                }
                            }
                        });
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            }
        }
        if close {
            self.st.modal = None;
        }
        if let Some(a) = action {
            self.st.actions.push(a);
        }
    }

    /// Tells the engine where the game's screen is and whether it has focus.
    fn update_game_viewport(&mut self, engine: &mut Engine, ctx: &egui::Context) {
        let st = &mut self.st;
        let ppp = ctx.pixels_per_point();
        let focused = st.playing() && st.game_focused;
        let rect = st.game_view.rect;
        engine.game_viewport = Some(match rect {
            Some(r) => GameViewport {
                x: r.min.x * ppp,
                y: r.min.y * ppp,
                width: r.width() * ppp,
                height: r.height() * ppp,
                focused,
            },
            None => GameViewport::default(),
        });
        let input = engine.world.resource_mut::<Input>();
        input.enabled = focused;
        if let Some(r) = rect {
            input.screen_height = r.height() * ppp;
        }
        engine.world.resource_mut::<Gui>().viewport = Some(rect.unwrap_or(egui::Rect::ZERO));
    }
}

/// `Cube` → `Cube (1)` → `Cube (2)`, like Unity's duplicate naming.
fn unique_name(base: &str) -> String {
    if let Some(open) = base.rfind(" (")
        && base.ends_with(')')
        && let Ok(n) = base[open + 2..base.len() - 1].parse::<u32>()
    {
        return format!("{} ({})", &base[..open], n + 1);
    }
    format!("{base} (1)")
}

fn create(w: &mut World, kind: Create, active: Option<GameObjectId>) -> GameObjectId {
    match kind {
        Create::Empty => w.create("GameObject"),
        Create::EmptyChild => {
            let id = w.create("GameObject");
            if let Some(p) = active.filter(|&p| w.is_alive(p)) {
                let _ = w.set_parent(id, Some(p), false);
            }
            id
        }
        Create::Primitive(p) => objects::create_primitive(w, p),
        Create::DirectionalLight => objects::create_directional_light(w),
        Create::PointLight => {
            objects::create_point_light(w, Vector3::ZERO, Color::WHITE, 10.0, 1.0)
        }
        Create::SpotLight => {
            let id = w.create_at(
                "Spot Light",
                Vector3::ZERO,
                Quaternion::euler(90.0, 0.0, 0.0),
            );
            w.add_component(
                id,
                Light {
                    light_type: LightType::Spot,
                    range: 10.0,
                    ..Default::default()
                },
            );
            id
        }
        Create::Camera => {
            let id = objects::create_camera(w, "Camera");
            if let Some(o) = w.get_mut(id) {
                o.tag = "Untagged".into();
            }
            id
        }
    }
}

fn game_view_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label("Display 1");
        let label = match st.game_aspect {
            None => "Free Aspect".to_owned(),
            Some(a) => GAME_ASPECTS
                .iter()
                .find(|(_, v)| (*v - a).abs() < 1e-3)
                .map_or_else(|| format!("{a:.2}"), |(n, _)| (*n).to_owned()),
        };
        egui::ComboBox::from_id_salt("game-aspect")
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut st.game_aspect, None, "Free Aspect");
                for (name, a) in GAME_ASPECTS {
                    ui.selectable_value(&mut st.game_aspect, Some(*a), *name);
                }
            });
        ui.toggle_value(&mut st.game_stats, "Stats");
    });
    let avail = ui.available_rect_before_wrap();
    let rect = match st.game_aspect {
        None => avail,
        Some(a) => {
            let (w, h) = if avail.width() / avail.height().max(1.0) > a {
                (avail.height() * a, avail.height())
            } else {
                (avail.width(), avail.width() / a)
            };
            egui::Rect::from_center_size(avail.center(), egui::vec2(w, h))
        }
    };
    let resp = ui.allocate_rect(avail, egui::Sense::click());
    ui.painter().rect_filled(avail, 0.0, egui::Color32::BLACK);
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let id = st.game_view.texture_id(
        engine,
        (rect.width() * ppp).round() as u32,
        (rect.height() * ppp).round() as u32,
    );
    st.game_view.rect = Some(rect);
    ui.painter().image(
        id,
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    if oxide_engine::extract::cameras(&engine.world).is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Display 1\nNo cameras rendering",
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(200),
        );
    }
    if resp.clicked() || resp.secondary_clicked() {
        st.game_focused = true;
    }
    if st.playing() && st.game_focused {
        ui.painter().rect_stroke(
            avail.shrink(0.5),
            0.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(58, 121, 187)),
            egui::StrokeKind::Inside,
        );
    }
    if st.game_stats {
        let s = &engine.stats;
        let text = format!(
            "{:.0} FPS ({:.1} ms)\nDraw calls: {}\nTris: {}",
            engine.fps,
            1000.0 / engine.fps.max(1.0),
            s.draw_calls,
            s.triangles
        );
        let pos = rect.right_top() + egui::vec2(-8.0, 8.0);
        let galley =
            ui.painter()
                .layout_no_wrap(text, egui::FontId::monospace(12.0), egui::Color32::WHITE);
        let bg = egui::Rect::from_min_size(
            pos - egui::vec2(galley.size().x + 8.0, 0.0),
            galley.size() + egui::vec2(8.0, 6.0),
        );
        ui.painter()
            .rect_filled(bg, 3.0, egui::Color32::from_black_alpha(170));
        ui.painter()
            .galley(bg.min + egui::vec2(4.0, 3.0), galley, egui::Color32::WHITE);
    }
}

const GAME_ASPECTS: &[(&str, f32)] = &[
    ("16:9", 16.0 / 9.0),
    ("16:10", 16.0 / 10.0),
    ("4:3", 4.0 / 3.0),
    ("21:9", 21.0 / 9.0),
    ("9:16 Portrait", 9.0 / 16.0),
];

impl Shell for Editor {
    fn install(&mut self, world: &mut World) {
        // Edit mode: components are added but no script callbacks run until Play.
        world.run_scripts = false;
    }

    fn ui(&mut self, engine: &mut Engine) {
        let ctx = engine.world.resource::<Gui>().ctx().clone();
        if !self.styled {
            theme::apply(&ctx);
            self.load_layout(engine);
            self.styled = true;
        }
        if let Some(host) = self.scripts.as_mut() {
            host.poll(engine);
        }
        self.st.scripts_available = self
            .scripts
            .as_ref()
            .is_some_and(|h| h.state != scripting::ScriptState::None);
        {
            let w = &engine.world;
            self.st.selection.retain(|&id| w.is_alive(id));
        }
        menus::shortcuts(&mut self.st, engine, &ctx);

        let mut root = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("oxide-editor"),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(ctx.content_rect()),
        );
        let playing = self.st.playing();
        egui::Panel::top(egui::Id::new("editor-menu"))
            .frame(theme::bar_frame(false))
            .show(&mut root, |ui| menus::menu_bar(&mut self.st, engine, ui));
        egui::Panel::top(egui::Id::new("editor-toolbar"))
            .frame(theme::bar_frame(playing))
            .show(&mut root, |ui| menus::toolbar(&mut self.st, engine, ui));
        egui::Panel::bottom(egui::Id::new("editor-status"))
            .frame(theme::bar_frame(false))
            .show(&mut root, |ui| {
                menus::status_bar(&mut self.st, engine, self.scripts.as_ref(), ui)
            });

        self.st.scene_view.rect = None;
        self.st.game_view.rect = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(&mut root, |ui| {
                let style = theme::dock_style(ui.style());
                let mut viewer = Viewer {
                    st: &mut self.st,
                    engine,
                };
                DockArea::new(&mut self.dock)
                    .style(style)
                    .show_leaf_collapse_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut viewer);
            });
        self.modal_ui(&ctx, engine);

        for a in std::mem::take(&mut self.st.actions) {
            if let Err(e) = self.apply(engine, a) {
                debug_log_warning!("{e}");
                self.st.status = e;
            }
        }
        self.update_game_viewport(engine, &ctx);
        self.save_layout(engine);

        let title = format!(
            "{}{} - Oxide Editor{}",
            engine.world.scene_name,
            if self.st.dirty { "*" } else { "" },
            if self.st.playing() { " [Play]" } else { "" }
        );
        if title != self.st.title {
            engine.window.set_title(&title);
            self.st.title = title;
        }
    }

    fn render(&mut self, engine: &mut Engine) {
        scene_view::render(&mut self.st, engine);
        if let (Some(view), [w, h]) = (self.st.game_view.view(), self.st.game_view.size)
            && self.st.game_view.rect.is_some()
        {
            let rf = oxide_engine::extract::extract(&engine.world, None);
            let assets = engine.world.resource::<AssetServer>();
            engine.stats =
                engine
                    .renderer
                    .render(&engine.gpu, assets, &rf, view, view::VIEW_FORMAT, w, h);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
