//! Oxide editor: Unity's main window around the running engine.
//!
//! Docked windows (Hierarchy, Scene, Game, Inspector, Project, Console), the main menu and
//! toolbar, Undo/Redo, and Play mode (the scene is snapshotted on Play and restored on Stop).
//! Every menu item is an [`Action`], so menus, shortcuts and the MCP tools in [`mcp`] share
//! one code path.
//!
//! References: <https://docs.unity3d.com/Manual/UsingTheEditor.html>,
//! <https://docs.unity3d.com/Manual/GameView.html>, <https://docs.unity3d.com/Manual/UndoWindow.html>

mod asset_inspector;
mod asset_ops;
mod console;
mod curve_editor;
pub(crate) mod game_view;
mod hierarchy;
mod inspector;
pub mod launch;
pub mod mcp;
mod menus;
pub mod player_build;
pub mod prefs;
mod previews;
mod project;
pub mod project_files;
mod scene_view;
pub mod scripting;
mod search_window;
pub mod settings_window;
pub mod shortcuts;
mod theme;
mod undo;
mod view;
mod visibility;

pub use view::SceneCamera;

use egui_dock::{DockArea, DockState, NodeIndex};
use oxide_assets::{AssetServer, PrimitiveType};
use oxide_core::{Time, debug_log, debug_log_warning};
use oxide_engine::{Engine, GameViewport, Shell, objects};
use oxide_input::{CursorLockMode, Input};
use oxide_math::{Color, Quaternion, QuaternionExt, Vector3};
use oxide_scene::{GameObjectId, Light, LightType, World};
use oxide_ui::Gui;
use std::collections::HashMap;

/// A dockable editor window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Scene,
    Game,
    Hierarchy,
    Inspector,
    Project,
    Console,
    ProjectSettings,
    Preferences,
    UndoHistory,
    Shortcuts,
}

impl Tab {
    /// Window → General.
    pub const ALL: [Tab; 7] = [
        Tab::Scene,
        Tab::Game,
        Tab::Hierarchy,
        Tab::Inspector,
        Tab::Project,
        Tab::Console,
        Tab::UndoHistory,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Scene => "Scene",
            Tab::Game => "Game",
            Tab::Hierarchy => "Hierarchy",
            Tab::Inspector => "Inspector",
            Tab::Project => "Project",
            Tab::Console => "Console",
            Tab::ProjectSettings => "Project Settings",
            Tab::Preferences => "Preferences",
            Tab::UndoHistory => "Undo History",
            Tab::Shortcuts => "Shortcuts",
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
    /// Resize on the plane facing the camera (Unity's Rect tool, T).
    Rect,
    /// Move, rotate and scale handles together (Unity's Transform tool, Y).
    Transform,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Hand => "hand",
            Tool::Move => "move",
            Tool::Rotate => "rotate",
            Tool::Scale => "scale",
            Tool::Rect => "rect",
            Tool::Transform => "transform",
        }
    }

    pub const ALL: [Tool; 6] = [
        Tool::Hand,
        Tool::Move,
        Tool::Rotate,
        Tool::Scale,
        Tool::Rect,
        Tool::Transform,
    ];

    pub fn parse(s: &str) -> Option<Tool> {
        Tool::ALL
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
    ReflectionProbe,
    LightProbeGroup,
    GlobalVolume,
    BoxVolume,
    SphereVolume,
    Camera,
}

impl Create {
    /// What gets created (for the undo step).
    fn label(self) -> &'static str {
        match self {
            Create::Empty | Create::EmptyChild => "GameObject",
            Create::Primitive(p) => p.name(),
            Create::DirectionalLight => "Directional Light",
            Create::PointLight => "Point Light",
            Create::SpotLight => "Spot Light",
            Create::ReflectionProbe => "Reflection Probe",
            Create::LightProbeGroup => "Light Probe Group",
            Create::GlobalVolume => "Global Volume",
            Create::BoxVolume => "Box Volume",
            Create::SphereVolume => "Sphere Volume",
            Create::Camera => "Camera",
        }
    }
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
    BuildSettingsDialog,
    /// Build the player with the saved Build Settings (and run it if `true`).
    BuildPlayer(bool),
    /// Split the main scene into streaming cells of this size (m).
    SplitIntoCells(f32),
    /// Ragdoll Wizard on the active (humanoid) object.
    CreateRagdoll,
    /// Lighting window: bake lightmaps and light probes in the background.
    GenerateLighting,
    ClearBakedLighting,
    /// Edit → Project Settings, on this page.
    OpenProjectSettings(settings_window::Page),
    OpenPreferences,
    /// Preferences, on this page.
    OpenPreferencesPage(prefs::Page),
    /// Edit → Search All (⌘K): the Search popup.
    OpenSearch,
    /// Assets → Refresh: reimport files changed on disk.
    RefreshAssets,
    /// Undo History: undo or redo until this many steps are applied.
    UndoTo(usize),
    /// Edit → Selection → Back / Forward through previous selections.
    SelectionBack,
    SelectionForward,
    /// Edit → Selection → Save / Load Selection N (1..9).
    SaveSelection(usize),
    LoadSelection(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Edit,
    Playing,
}

enum Modal {
    SaveAs(String),
    Open,
    BuildSettings(player_build::BuildSettings),
}

/// Editor state the windows read and write (everything except the dock layout).
pub struct EditorState {
    /// Selected objects; the last one is the active object (shown in the Inspector).
    pub selection: Vec<GameObjectId>,
    pub tool: Tool,
    /// Handles follow the object's rotation (Unity "Local") instead of world axes.
    pub local_handles: bool,
    /// Handles sit at the selection's bounds center (Unity "Center") instead of the active
    /// object's pivot, and rotate/scale move objects around it.
    pub center_handles: bool,
    /// Scene view hide/pick toggles.
    pub visibility: visibility::SceneVisibility,
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
    pub(crate) inspector: inspector::InspectorState,
    console: console::ConsoleState,
    project: project::ProjectState,
    /// Asset thumbnails (Project window, Inspector).
    previews: previews::Previews,
    asset_inspector: asset_inspector::AssetInspectorState,
    /// Shortcut bindings (defaults plus the user's changes from Preferences).
    pub keymap: shortcuts::Keymap,
    shortcuts_ui: shortcuts::ShortcutsWindow,
    pub(crate) search: search_window::SearchState,
    scene_ui: scene_view::SceneViewState,
    pub(crate) game: game_view::GameViewState,
    modal: Option<Modal>,
    title: String,
    /// Status messages from actions (shown in the status bar).
    pub status: String,
    /// The project has a Scripts crate (Create > Rust Script is available).
    pub scripts_available: bool,
    pub builder: player_build::PlayerBuilder,
    /// Per-user Preferences, where they're saved, and whether they changed this frame.
    pub prefs: prefs::EditorPrefs,
    prefs_path: std::path::PathBuf,
    pub(crate) prefs_page: prefs::Page,
    pub(crate) prefs_changed: bool,
    pub(crate) settings: settings_window::SettingsState,
    /// Previous selections (Back / Forward) and the position in that list.
    sel_history: Vec<Vec<GameObjectId>>,
    sel_pos: usize,
    /// Edit → Selection → Save Selection 1..9.
    saved_selections: [Vec<GameObjectId>; 9],
}

impl EditorState {
    fn new() -> Self {
        Self {
            selection: Vec::new(),
            tool: Tool::Move,
            local_handles: false,
            center_handles: false,
            visibility: Default::default(),
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
            previews: Default::default(),
            asset_inspector: Default::default(),
            keymap: Default::default(),
            shortcuts_ui: Default::default(),
            search: Default::default(),
            scene_ui: Default::default(),
            game: Default::default(),
            modal: None,
            title: String::new(),
            status: String::new(),
            scripts_available: false,
            builder: Default::default(),
            prefs: Default::default(),
            prefs_path: prefs::EditorPrefs::default_path(),
            prefs_page: Default::default(),
            prefs_changed: false,
            settings: Default::default(),
            sel_history: Vec::new(),
            sel_pos: 0,
            saved_selections: Default::default(),
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

    /// Paths of the objects in the selection history and saved selections, taken before
    /// the scene is rebuilt (undo, redo, Stop) so they can be found again afterwards.
    fn remember_paths(&self, w: &World) -> HashMap<GameObjectId, ObjectPath> {
        self.sel_history
            .iter()
            .chain(self.saved_selections.iter())
            .flatten()
            .chain(&self.visibility.hidden)
            .chain(&self.visibility.unpickable)
            .filter(|&&id| w.is_alive(id))
            .map(|&id| (id, object_path(w, id)))
            .collect()
    }

    /// Points the selection history and saved selections at the rebuilt objects.
    fn remap_paths(&mut self, w: &World, before: &HashMap<GameObjectId, ObjectPath>) {
        let now: HashMap<ObjectPath, GameObjectId> = w
            .hierarchy_order()
            .into_iter()
            .map(|id| (object_path(w, id), id))
            .collect();
        let map = |list: &mut Vec<GameObjectId>| {
            *list = list
                .iter()
                .filter_map(|id| before.get(id).and_then(|p| now.get(p)).copied())
                .collect();
        };
        self.sel_history.iter_mut().for_each(map);
        self.saved_selections.iter_mut().for_each(map);
        let map_set = |set: &mut std::collections::HashSet<GameObjectId>| {
            *set = set
                .iter()
                .filter_map(|id| before.get(id).and_then(|p| now.get(p)).copied())
                .collect();
        };
        map_set(&mut self.visibility.hidden);
        map_set(&mut self.visibility.unpickable);
    }

    /// Adds the selection to the Back / Forward history when it changed.
    fn track_selection(&mut self, world: &World) {
        for s in &mut self.sel_history {
            s.retain(|&id| world.is_alive(id));
        }
        self.sel_history.dedup();
        self.sel_history.retain(|s| !s.is_empty());
        self.sel_pos = self.sel_pos.min(self.sel_history.len().saturating_sub(1));
        if self.selection.is_empty() || self.sel_history.get(self.sel_pos) == Some(&self.selection)
        {
            return;
        }
        self.sel_history.truncate(self.sel_pos + 1);
        self.sel_history.push(self.selection.clone());
        if self.sel_history.len() > 50 {
            self.sel_history.remove(0);
        }
        self.sel_pos = self.sel_history.len() - 1;
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
    /// The layout from before Play while Maximize On Play shows only the Game view.
    unmaximized: Option<DockState<Tab>>,
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
            Tab::Game => game_view::ui(self.st, self.engine, ui),
            Tab::Hierarchy => hierarchy::ui(self.st, self.engine, ui),
            Tab::Inspector => inspector::ui(self.st, self.engine, ui),
            Tab::Project => project::ui(self.st, self.engine, ui),
            Tab::Console => console::ui(self.st, self.engine, ui),
            Tab::ProjectSettings => settings_window::ui(self.st, self.engine, ui),
            Tab::Preferences => prefs::ui(self.st, self.engine, ui),
            Tab::UndoHistory => undo::window_ui(self.st, ui),
            Tab::Shortcuts => shortcuts::window_ui(self.st, ui),
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
            unmaximized: None,
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
            Ok(mut dock) if dock.iter_all_tabs().next().is_some() => {
                // Settings windows open on demand at full size (floating sizes aren't saved).
                for tab in [Tab::ProjectSettings, Tab::Preferences, Tab::Shortcuts] {
                    while let Some(path) = dock.find_tab(&tab) {
                        dock.remove_tab(path);
                    }
                }
                self.dock = dock;
                self.saved_layout = Some(text);
            }
            Ok(_) => {}
            Err(e) => debug_log_warning!("ignoring saved editor layout: {e}"),
        }
    }

    /// Saves the layout when it changed (checked every couple of seconds).
    fn save_layout(&mut self, engine: &Engine) {
        if self.unmaximized.is_some() {
            return;
        }
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

    pub(crate) fn focus_tab(&mut self, tab: Tab) {
        let found = self
            .dock
            .iter_all_tabs()
            .find(|(_, t)| **t == tab)
            .map(|(p, _)| p);
        if let Some(path) = found {
            let _ = self.dock.set_active_tab(path);
        } else if matches!(
            tab,
            Tab::ProjectSettings | Tab::Preferences | Tab::Shortcuts
        ) {
            // Unity opens its settings in their own window.
            let surface = self.dock.add_window(vec![tab]);
            if let Some(ws) = self.dock.get_window_state_mut(surface) {
                ws.set_position(egui::pos2(220.0, 110.0))
                    .set_size(egui::vec2(760.0, 560.0));
            }
        } else {
            self.dock.push_to_focused_leaf(tab);
        }
    }

    /// Runs one editor action. Returns a short description of what happened.
    pub fn apply(&mut self, engine: &mut Engine, action: Action) -> Result<String, String> {
        let now = oxide_core::time::realtime_since_startup();
        let st = &mut self.st;
        // Rename / Duplicate / Delete act on assets after the Project window was clicked.
        if st.project.focused
            && let Some(sel) = st.project.selected.clone()
        {
            match action {
                Action::Rename => {
                    st.project.start_rename();
                    return Ok(String::new());
                }
                Action::Delete => {
                    st.project.ask_delete();
                    return Ok(String::new());
                }
                Action::Duplicate => {
                    let root = engine.world.resource::<AssetServer>().full_path("");
                    let new = asset_ops::duplicate_asset(&root, &sel)?;
                    st.project.selected = Some(new.clone());
                    st.project.invalidate();
                    return Ok(format!("Duplicated {sel} as {new}"));
                }
                _ => {}
            }
        }
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
            Action::Undo | Action::Redo => {
                let undo = action == Action::Undo;
                let paths = st.remember_paths(w);
                let step = if undo {
                    st.undo.undo(w, &st.selection)
                } else {
                    st.undo.redo(w, &st.selection)
                };
                let (label, sel) = step.ok_or(if undo {
                    "nothing to undo"
                } else {
                    "nothing to redo"
                })?;
                st.remap_paths(w, &paths);
                st.selection = sel;
                st.dirty = true;
                engine.start_systems();
                format!("{} {label}", if undo { "Undo" } else { "Redo" })
            }
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
                st.record(w, &format!("Create {}", kind.label()), None, now);
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
            Action::SplitIntoCells(size) => {
                if st.play != PlayState::Edit {
                    return Err("exit Play mode first".into());
                }
                st.record(w, "Split Scene into Cells", None, now);
                let opts = oxide_engine::streaming::SplitOptions {
                    cell_size: size,
                    folder: format!("Assets/Scenes/{}_Cells", w.scene_name),
                    only_static: true,
                };
                let r = oxide_engine::streaming::split_scene_into_cells(w, &opts)?;
                st.selection.clear();
                format!(
                    "Moved {} objects into {} cells under {}; save the scene to keep the World Streamer",
                    r.moved_roots,
                    r.cells.len(),
                    opts.folder
                )
            }
            Action::GenerateLighting => {
                let n = oxide_engine::lighting::start(w)?;
                format!("Baking lighting for {n} static objects…")
            }
            Action::ClearBakedLighting => {
                st.record(w, "Clear Baked Data", None, now);
                oxide_engine::lighting::clear(w);
                "Cleared baked lighting".into()
            }
            Action::CreateRagdoll => {
                let id = st.active().ok_or("select a humanoid character first")?;
                st.record(w, "Create Ragdoll", None, now);
                let bodies = oxide_engine::prelude::create_ragdoll(w, id, 70.0)?;
                format!("Added a ragdoll with {} bodies", bodies.len())
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
            Action::OpenProjectSettings(page) => {
                st.settings.page = page;
                self.focus_tab(Tab::ProjectSettings);
                return Ok("Opened Project Settings".into());
            }
            Action::OpenPreferences => {
                self.focus_tab(Tab::Preferences);
                return Ok("Opened Preferences".into());
            }
            Action::OpenPreferencesPage(page) => {
                st.prefs_page = page;
                self.focus_tab(Tab::Preferences);
                return Ok("Opened Preferences".into());
            }
            Action::OpenSearch => {
                st.search.open = !st.search.open;
                String::new()
            }
            Action::RefreshAssets => {
                let n = w.resource_mut::<AssetServer>().reload_changed().len();
                st.project.invalidate();
                st.previews.clear();
                format!("Refreshed assets ({n} reimported)")
            }
            Action::UndoTo(target) => {
                let paths = st.remember_paths(w);
                let mut label = String::new();
                while st.undo.depth() > target {
                    let Some((l, sel)) = st.undo.undo(w, &st.selection) else {
                        break;
                    };
                    st.selection = sel;
                    label = format!("Undo {l}");
                }
                while st.undo.depth() < target {
                    let Some((l, sel)) = st.undo.redo(w, &st.selection) else {
                        break;
                    };
                    st.selection = sel;
                    label = format!("Redo {l}");
                }
                st.remap_paths(w, &paths);
                st.dirty = true;
                engine.start_systems();
                label
            }
            Action::SelectionBack | Action::SelectionForward => {
                st.track_selection(w);
                let back = action == Action::SelectionBack;
                let n = st.sel_history.len();
                let pos = if back {
                    st.sel_pos.checked_sub(1)
                } else {
                    (st.sel_pos + 1 < n).then_some(st.sel_pos + 1)
                };
                let pos = pos.ok_or("no more selections in history")?;
                st.sel_pos = pos;
                st.selection = st.sel_history[pos].clone();
                format!("Selected {} object(s)", st.selection.len())
            }
            Action::SaveSelection(n) => {
                let slot = st
                    .saved_selections
                    .get_mut(n.wrapping_sub(1))
                    .ok_or("selection slots are 1..9")?;
                *slot = st.selection.clone();
                format!("Saved selection {n}")
            }
            Action::LoadSelection(n) => {
                let sel = st
                    .saved_selections
                    .get(n.wrapping_sub(1))
                    .ok_or("selection slots are 1..9")?;
                let sel: Vec<_> = sel.iter().copied().filter(|&id| w.is_alive(id)).collect();
                if sel.is_empty() {
                    return Err(format!("selection {n} is empty"));
                }
                st.selection = sel;
                format!("Loaded selection {n}")
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
            Action::BuildSettingsDialog => {
                let project = w.resource::<AssetServer>().full_path("");
                let mut settings = player_build::BuildSettings::load(&project);
                if settings.scenes.is_empty()
                    && let Some(p) = &w.scene_path
                {
                    settings.scenes.push(p.clone());
                }
                st.modal = Some(Modal::BuildSettings(settings));
                String::new()
            }
            Action::BuildPlayer(run) => {
                let project = w.resource::<AssetServer>().full_path("");
                let settings = player_build::BuildSettings::load(&project);
                if st.dirty {
                    debug_log_warning!(
                        "The open scene has unsaved changes; the build uses the saved file"
                    );
                }
                st.builder.start(project, settings, run)?;
                "Building player...".into()
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
        if st.prefs.clear_on_play {
            oxide_core::debug::clear();
            st.console.clear();
        }
        // Fresh game clock, keeping the project's fixed timestep and time scale.
        let (fixed, max, scale) = {
            let t = w.resource::<Time>();
            (t.fixed_delta_time(), t.maximum_delta_time(), t.time_scale())
        };
        let mut t = Time::new();
        t.set_fixed_delta_time(fixed);
        t.set_maximum_delta_time(max);
        t.set_time_scale(scale);
        w.resources.insert(t);
        w.start_scripts();
        engine.start_systems();
        engine.simulate = true;
        engine.paused = false;
        st.play = PlayState::Playing;
        st.game_focused = true;
        if st.game.maximize_on_play {
            self.unmaximized = Some(std::mem::replace(
                &mut self.dock,
                DockState::new(vec![Tab::Game]),
            ));
        }
        self.focus_tab(Tab::Game);
    }

    fn exit_play(&mut self, engine: &mut Engine) {
        let st = &mut self.st;
        engine.simulate = false;
        engine.paused = false;
        engine.pending_steps = 0;
        let w = &mut engine.world;
        // Scripts see OnDisable/OnDestroy as the play-mode objects go away.
        let paths = st.remember_paths(w);
        w.clear(false);
        w.run_scripts = false;
        if let Some(snap) = st.play_snapshot.take() {
            st.selection = snap.restore(w);
        }
        st.remap_paths(w, &paths);
        st.undo.truncate(st.play_undo_depth);
        let input = w.resource_mut::<Input>();
        input.release_all();
        input.cursor_lock = CursorLockMode::None;
        input.cursor_visible = true;
        engine.start_systems();
        st.play = PlayState::Edit;
        st.game_focused = false;
        if let Some(dock) = self.unmaximized.take() {
            self.dock = dock;
        }
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
            Modal::BuildSettings(settings) => {
                let project = engine.world.resource::<AssetServer>().full_path("");
                let open_scene = engine.world.scene_path.clone();
                let building = self.st.builder.is_building();
                egui::Modal::new(egui::Id::new("build-settings")).show(ctx, |ui| {
                    ui.set_min_width(460.0);
                    ui.heading("Build Settings");
                    ui.label("Scenes In Build (the first one opens at startup):");
                    egui::Frame::new()
                        .fill(egui::Color32::from_gray(42))
                        .inner_margin(6)
                        .show(ui, |ui| {
                            ui.set_min_height(80.0);
                            let mut remove = None;
                            let mut up = None;
                            for (i, scene) in settings.scenes.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.monospace(format!("{i}"));
                                    ui.label(scene);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.small_button("Remove").clicked() {
                                                remove = Some(i);
                                            }
                                            if i > 0 && ui.small_button("Up").clicked() {
                                                up = Some(i);
                                            }
                                        },
                                    );
                                });
                            }
                            if let Some(i) = remove {
                                settings.scenes.remove(i);
                            }
                            if let Some(i) = up {
                                settings.scenes.swap(i, i - 1);
                            }
                        });
                    let can_add = open_scene
                        .as_ref()
                        .is_some_and(|p| !settings.scenes.contains(p));
                    if ui
                        .add_enabled(can_add, egui::Button::new("Add Open Scene"))
                        .on_disabled_hover_text("Save the scene first, or it is already in the list")
                        .clicked()
                        && let Some(p) = open_scene.clone()
                    {
                        settings.scenes.push(p);
                    }
                    ui.separator();
                    egui::Grid::new("player-settings").num_columns(2).show(ui, |ui| {
                        ui.label("Product Name");
                        ui.text_edit_singleline(&mut settings.product_name);
                        ui.end_row();
                        ui.label("Resolution");
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut settings.width).range(320..=7680));
                            ui.label("×");
                            ui.add(egui::DragValue::new(&mut settings.height).range(240..=4320));
                        });
                        ui.end_row();
                        ui.label("Fullscreen");
                        ui.checkbox(&mut settings.fullscreen, "");
                        ui.end_row();
                        ui.label("Output Folder");
                        ui.text_edit_singleline(&mut settings.output);
                        ui.end_row();
                    });
                    ui.weak(format!(
                        "Platform: {} ({}). Release build of the engine and your Scripts; Assets are copied next to the game.",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    ));
                    ui.separator();
                    ui.horizontal(|ui| {
                        let save = |s: &player_build::BuildSettings| {
                            if let Err(e) = s.save(&project) {
                                debug_log_warning!("cannot save build settings: {e}");
                            }
                        };
                        if ui.add_enabled(!building, egui::Button::new("Build")).clicked() {
                            save(settings);
                            action = Some(Action::BuildPlayer(false));
                            close = true;
                        }
                        if ui
                            .add_enabled(!building, egui::Button::new("Build And Run"))
                            .clicked()
                        {
                            save(settings);
                            action = Some(Action::BuildPlayer(true));
                            close = true;
                        }
                        if ui.button("Close").clicked() {
                            save(settings);
                            close = true;
                        }
                        if building {
                            ui.spinner();
                            ui.label("Building...");
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
                scale: st.game.pixel_scale,
            },
            None => GameViewport::default(),
        });
        let input = engine.world.resource_mut::<Input>();
        input.enabled = focused;
        if let Some(r) = rect {
            input.screen_height = r.height() * ppp * st.game.pixel_scale;
        }
        engine.world.resource_mut::<Gui>().viewport = Some(rect.unwrap_or(egui::Rect::ZERO));
    }
}

/// An object's place in the hierarchy: (sibling index, name) from the root down.
type ObjectPath = Vec<(usize, String)>;

fn object_path(w: &World, id: GameObjectId) -> ObjectPath {
    let mut path = Vec::new();
    let mut cur = Some(id);
    while let Some(x) = cur {
        path.push((
            w.sibling_index(x),
            w.get(x).map(|o| o.name.clone()).unwrap_or_default(),
        ));
        cur = w.parent(x);
    }
    path.reverse();
    path
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
        Create::ReflectionProbe => {
            let id = w.create("Reflection Probe");
            w.add_component(id, oxide_scene::ReflectionProbe::default());
            id
        }
        Create::LightProbeGroup => {
            let id = w.create("Light Probe Group");
            w.add_component(id, oxide_scene::LightProbeGroup::default());
            id
        }
        Create::GlobalVolume => {
            let id = w.create("Global Volume");
            w.add_component(id, oxide_scene::Volume::default());
            id
        }
        Create::BoxVolume | Create::SphereVolume => {
            let sphere = kind == Create::SphereVolume;
            let id = w.create(if sphere {
                "Sphere Volume"
            } else {
                "Box Volume"
            });
            w.add_component(
                id,
                oxide_scene::Volume {
                    is_global: false,
                    ..Default::default()
                },
            );
            if sphere {
                w.add_component(
                    id,
                    oxide_physics::SphereCollider {
                        is_trigger: true,
                        ..Default::default()
                    },
                );
            } else {
                w.add_component(
                    id,
                    oxide_physics::BoxCollider {
                        is_trigger: true,
                        ..Default::default()
                    },
                );
            }
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
            self.st.prefs = prefs::EditorPrefs::load(&self.st.prefs_path);
            self.st.keymap = shortcuts::Keymap::new(&self.st.prefs.shortcuts);
            self.styled = true;
        }
        self.st.prefs.apply(&ctx);
        engine.auto_refresh = self.st.prefs.auto_refresh;
        if let Some(host) = self.scripts.as_mut() {
            host.poll(engine);
        }
        self.st.builder.poll();
        self.st.scripts_available = self
            .scripts
            .as_ref()
            .is_some_and(|h| h.state != scripting::ScriptState::None);
        {
            let w = &engine.world;
            self.st.selection.retain(|&id| w.is_alive(id));
        }
        project::sync_asset_types(&engine.world.resource::<oxide_assets::AssetServer>().types);
        shortcuts::dispatch(&mut self.st, engine, &ctx);
        self.st.previews.update(engine, &ctx);
        // The Project window claims keyboard commands when it is clicked (it sets this).
        if ctx.input(|i| i.pointer.any_pressed()) {
            self.st.project.focused = false;
        }

        let mut root = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("oxide-editor"),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(ctx.content_rect()),
        );
        let playing = self.st.playing();
        egui::Panel::top(egui::Id::new("editor-menu"))
            .frame(theme::bar_frame(None))
            .show(&mut root, |ui| menus::menu_bar(&mut self.st, engine, ui));
        let tint = playing.then(|| self.st.prefs.play_tint());
        egui::Panel::top(egui::Id::new("editor-toolbar"))
            .frame(theme::bar_frame(tint))
            .show(&mut root, |ui| menus::toolbar(&mut self.st, engine, ui));
        egui::Panel::bottom(egui::Id::new("editor-status"))
            .frame(theme::bar_frame(None))
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
        search_window::ui(&mut self.st, engine, &ctx);

        for a in std::mem::take(&mut self.st.actions) {
            if let Err(e) = self.apply(engine, a) {
                debug_log_warning!("{e}");
                self.st.status = e;
            }
        }
        self.st.track_selection(&engine.world);
        self.update_game_viewport(engine, &ctx);
        self.save_layout(engine);
        settings_window::flush(
            &mut self.st,
            engine,
            oxide_core::time::realtime_since_startup(),
        );
        if std::mem::take(&mut self.st.prefs_changed)
            && let Err(e) = self.st.prefs.save(&self.st.prefs_path)
        {
            self.st.status = format!("cannot save Preferences: {e}");
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_history_tracks_and_drops_dead_objects() {
        let mut w = World::new();
        let a = w.create("A");
        let b = w.create("B");
        let mut st = EditorState::new();
        st.select(a);
        st.track_selection(&w);
        st.select(b);
        st.track_selection(&w);
        st.track_selection(&w);
        assert_eq!(st.sel_history, vec![vec![a], vec![b]]);
        assert_eq!(st.sel_pos, 1);
        w.destroy_immediate(a);
        st.track_selection(&w);
        assert_eq!(st.sel_history, vec![vec![b]]);
        assert_eq!(st.sel_pos, 0);

        // A snapshot restore (Undo, Stop) recreates every object; history follows by path.
        let c = w.create("C");
        w.set_parent(c, Some(b), false).unwrap();
        st.select(c);
        st.track_selection(&w);
        st.saved_selections[0] = vec![b];
        let snap = undo::Snapshot::take(&w, "x", &[]);
        let paths = st.remember_paths(&w);
        snap.restore(&mut w);
        st.remap_paths(&w, &paths);
        let (b2, c2) = (w.find("B").unwrap(), w.find("C").unwrap());
        assert_ne!(b2, b);
        assert_eq!(st.sel_history, vec![vec![b2], vec![c2]]);
        assert_eq!(st.saved_selections[0], vec![b2]);
    }
}
