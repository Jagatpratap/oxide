//! Editor extensions from scripts (Unity's Extending the Editor): custom windows
//! (`EditorWindow`), menu items (`[MenuItem]`, with validation and shortcuts), property
//! drawers (`[CustomPropertyDrawer]`, by field type or by a `#[reflect(drawer = "...")]`
//! attribute), Scene view overlays (`[Overlay]`) and Scene view tools (`[EditorTool]` with
//! `Handles`). A Scripts library lists a registration function under `extensions:` in its
//! `scripts!` block:
//!
//! ```ignore
//! fn register(ext: &mut EditorExtensions) {
//!     ext.window::<SpawnerWindow>("Spawner", "Window/Tools/Spawner");
//!     ext.menu_item("Tools/Reset Selected", reset_selected)
//!         .validate(|ctx| !ctx.selection().is_empty())
//!         .shortcut("Ctrl+Shift+R");
//!     ext.property_drawer("Percent", percent_drawer);
//!     ext.overlay::<StatsOverlay>("Scene Stats");
//!     ext.tool::<WaypointTool>("Waypoints", "⚑");
//! }
//!
//! oxide::scripts! { Spinner => "Scripts/Spinner"; extensions: register }
//! ```
//! The editor drops every extension (and its windows' state) when the library reloads, then
//! registers the new library's. Player builds compile the same code and never call it.
//! References: <https://docs.unity3d.com/Manual/extending-the-editor.html>,
//! <https://docs.unity3d.com/Manual/overlays-custom.html>,
//! <https://docs.unity3d.com/Manual/UsingCustomEditorTools.html>

use oxide_core::reflect::{FieldInfo, Value};
use oxide_math::{Color, Ray, Vector3};
use oxide_scene::{GameObjectId, World};
use oxide_ui::egui;
use std::collections::HashMap;

/// What the editor does after an extension call.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    /// Opens (or focuses) a window by title (built-in or custom).
    OpenWindow(String),
    /// Runs a main-menu item by path (Unity `EditorApplication.ExecuteMenuItem`).
    ExecuteMenu(String),
    /// Shows a message in the status bar.
    Status(String),
    /// Activates a Scene view tool by name (a custom tool, or Move/Rotate/...).
    SetTool(String),
}

/// The editor as an extension sees it: the scene, the selection, Undo, and commands.
pub struct EditorContext<'a> {
    /// The open scenes (edit them after [`record_undo`](Self::record_undo)).
    pub world: &'a mut World,
    selection: Vec<GameObjectId>,
    selection_changed: bool,
    undo: &'a mut dyn FnMut(&World, &str),
    commands: Vec<EditorCommand>,
    playing: bool,
}

impl<'a> EditorContext<'a> {
    /// Used by the editor.
    pub fn new(
        world: &'a mut World,
        selection: Vec<GameObjectId>,
        playing: bool,
        undo: &'a mut dyn FnMut(&World, &str),
    ) -> Self {
        Self {
            world,
            selection,
            selection_changed: false,
            undo,
            commands: Vec::new(),
            playing,
        }
    }

    /// Unity `Selection.objects` (the active object last).
    pub fn selection(&self) -> &[GameObjectId] {
        &self.selection
    }

    /// Unity `Selection.activeGameObject`.
    pub fn active(&self) -> Option<GameObjectId> {
        self.selection.last().copied()
    }

    /// Sets the selection (Unity `Selection.objects = ...`).
    pub fn select(&mut self, ids: impl IntoIterator<Item = GameObjectId>) {
        self.selection = ids.into_iter().collect();
        self.selection_changed = true;
    }

    /// Unity `Undo.RecordObject`: call before changing the scene, so Edit > Undo restores
    /// it. Also marks the scene as modified. Calls with the same label in quick succession
    /// (every frame of a handle drag) make one undo step.
    pub fn record_undo(&mut self, label: &str) {
        (self.undo)(self.world, label);
    }

    /// Unity `EditorApplication.isPlaying`.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn open_window(&mut self, title: &str) {
        self.commands
            .push(EditorCommand::OpenWindow(title.to_owned()));
    }

    /// Unity `EditorApplication.ExecuteMenuItem`.
    pub fn execute_menu(&mut self, path: &str) {
        self.commands
            .push(EditorCommand::ExecuteMenu(path.to_owned()));
    }

    /// Shows a message in the status bar (and the Console).
    pub fn set_status(&mut self, text: impl Into<String>) {
        self.commands.push(EditorCommand::Status(text.into()));
    }

    /// Unity `ToolManager.SetActiveTool`.
    pub fn set_tool(&mut self, name: &str) {
        self.commands.push(EditorCommand::SetTool(name.to_owned()));
    }

    /// Used by the editor: the new selection if it changed, and the commands.
    pub fn finish(self) -> (Option<Vec<GameObjectId>>, Vec<EditorCommand>) {
        (
            self.selection_changed.then_some(self.selection),
            self.commands,
        )
    }
}

/// Unity `EditorWindow`: a dockable window drawn with egui.
pub trait EditorWindow: Send + Sync + 'static {
    /// Unity `OnGUI`.
    fn on_gui(&mut self, ctx: &mut EditorContext, ui: &mut egui::Ui);
    /// Unity `OnEnable`: the window was opened (or the scripts reloaded while it was open).
    fn on_enable(&mut self, _ctx: &mut EditorContext) {}
    /// Unity `OnSelectionChange`.
    fn on_selection_change(&mut self, _ctx: &mut EditorContext) {}
}

/// Unity `Overlay`: a panel in the Scene view (draggable, dockable, hideable from the
/// Overlay menu like the built-in ones).
pub trait Overlay: Send + Sync + 'static {
    fn on_gui(&mut self, ctx: &mut EditorContext, ui: &mut egui::Ui);
    /// Contextual overlays show only when this is true (e.g. for a selected component).
    fn applies(&self, _ctx: &EditorContext) -> bool {
        true
    }
}

/// How the Scene view maps between world space and the screen (supplied by the editor).
pub trait SceneProjection {
    /// Screen position (egui points) of a world point, `None` behind the camera.
    fn project(&self, p: Vector3) -> Option<egui::Pos2>;
    /// The world ray under a screen position.
    fn ray(&self, p: egui::Pos2) -> Ray;
    /// World size covering `pixels` on screen at `p` (for constant-size handles).
    fn world_per_pixels(&self, p: Vector3, pixels: f32) -> f32;
    fn camera_position(&self) -> Vector3;
    fn camera_forward(&self) -> Vector3;
    /// The Scene view's rect.
    fn rect(&self) -> egui::Rect;
}

/// What a Scene view tool draws and handles with (Unity `Handles` and `HandleUtility`).
pub struct SceneGui<'a, 'b> {
    pub ctx: &'a mut EditorContext<'b>,
    pub projection: &'a dyn SceneProjection,
    ui: &'a mut egui::Ui,
    /// Handle ids used this frame, in order (ids of the handle being dragged persist).
    next_id: u64,
    /// Whether a handle took the pointer this frame (then the Scene view doesn't select).
    pub used: bool,
}

impl<'a, 'b> SceneGui<'a, 'b> {
    /// Used by the editor.
    pub fn new(
        ctx: &'a mut EditorContext<'b>,
        projection: &'a dyn SceneProjection,
        ui: &'a mut egui::Ui,
    ) -> Self {
        Self {
            ctx,
            projection,
            ui,
            next_id: 0,
            used: false,
        }
    }

    /// The egui `Ui` covering the Scene view (for anything else).
    pub fn ui(&mut self) -> &mut egui::Ui {
        self.ui
    }

    fn color(c: Color) -> egui::Color32 {
        let c = oxide_math::Color32::from(c);
        egui::Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
    }

    /// Unity `Handles.DrawLine`.
    pub fn line(&mut self, a: Vector3, b: Vector3, color: Color, width: f32) {
        if let (Some(p), Some(q)) = (self.projection.project(a), self.projection.project(b)) {
            self.ui
                .painter()
                .line_segment([p, q], egui::Stroke::new(width, Self::color(color)));
        }
    }

    /// Unity `Handles.DrawPolyLine`.
    pub fn polyline(&mut self, points: &[Vector3], color: Color, width: f32) {
        for w in points.windows(2) {
            self.line(w[0], w[1], color, width);
        }
    }

    /// Unity `Handles.DrawWireDisc`.
    pub fn wire_disc(&mut self, center: Vector3, normal: Vector3, radius: f32, color: Color) {
        let n = normal.normalize_or(Vector3::Y);
        let t = n.any_orthonormal_vector();
        let b = n.cross(t);
        let pts: Vec<Vector3> = (0..=48)
            .map(|i| {
                let a = i as f32 / 48.0 * std::f32::consts::TAU;
                center + (t * a.cos() + b * a.sin()) * radius
            })
            .collect();
        self.polyline(&pts, color, 1.5);
    }

    /// Unity `Handles.Label`.
    pub fn label(&mut self, at: Vector3, text: &str, color: Color) {
        if let Some(p) = self.projection.project(at) {
            self.ui.painter().text(
                p + egui::vec2(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                text,
                egui::FontId::proportional(12.0),
                Self::color(color),
            );
        }
    }

    /// A dot of `pixels` radius at a world point.
    pub fn dot(&mut self, at: Vector3, pixels: f32, color: Color) {
        if let Some(p) = self.projection.project(at) {
            self.ui
                .painter()
                .circle_filled(p, pixels, Self::color(color));
        }
    }

    fn handle_response(&mut self, at: egui::Pos2, pixels: f32) -> egui::Response {
        self.next_id += 1;
        let id = egui::Id::new(("oxide-tool-handle", self.next_id));
        let r =
            egui::Rect::from_center_size(at, egui::vec2(pixels * 2.0 + 4.0, pixels * 2.0 + 4.0));
        let resp = self.ui.interact(r, id, egui::Sense::click_and_drag());
        if resp.hovered() || resp.dragged() || resp.clicked() {
            self.used = true;
        }
        resp
    }

    /// Unity `Handles.Button`: a clickable dot. Returns whether it was clicked.
    pub fn button(&mut self, at: Vector3, pixels: f32, color: Color) -> bool {
        let Some(p) = self.projection.project(at) else {
            self.next_id += 1;
            return false;
        };
        let resp = self.handle_response(p, pixels);
        let c = if resp.hovered() { Color::YELLOW } else { color };
        self.ui.painter().circle_filled(p, pixels, Self::color(c));
        resp.clicked()
    }

    /// Unity `Handles.FreeMoveHandle`: a dot dragged on the plane facing the camera.
    /// Returns the new position (the same while not dragged).
    pub fn free_move_handle(&mut self, at: Vector3, pixels: f32, color: Color) -> Vector3 {
        let Some(p) = self.projection.project(at) else {
            self.next_id += 1;
            return at;
        };
        let resp = self.handle_response(p, pixels);
        let c = if resp.hovered() || resp.dragged() {
            Color::YELLOW
        } else {
            color
        };
        self.ui.painter().circle_filled(p, pixels, Self::color(c));
        self.ui.painter().circle_stroke(
            p,
            pixels + 1.5,
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
        );
        if resp.dragged()
            && let Some(m) = resp.interact_pointer_pos()
        {
            let n = -self.projection.camera_forward();
            let ray = self.projection.ray(m);
            let denom = ray.direction.dot(n);
            if denom.abs() > 1e-5 {
                let t = (at - ray.origin).dot(n) / denom;
                if t > 0.0 {
                    return ray.origin + ray.direction * t;
                }
            }
        }
        at
    }

    /// Unity `Handles.Slider`: drag along `direction` (an arrow from `at`). Returns the new
    /// position.
    pub fn slider(&mut self, at: Vector3, direction: Vector3, color: Color) -> Vector3 {
        let dir = direction.normalize_or(Vector3::Y);
        let len = self.projection.world_per_pixels(at, 70.0);
        let tip = at + dir * len;
        self.line(at, tip, color, 2.5);
        let Some(p) = self.projection.project(tip) else {
            self.next_id += 1;
            return at;
        };
        let resp = self.handle_response(p, 6.0);
        let c = if resp.hovered() || resp.dragged() {
            Color::YELLOW
        } else {
            color
        };
        self.ui.painter().circle_filled(p, 6.0, Self::color(c));
        if resp.dragged()
            && let Some(m) = resp.interact_pointer_pos()
        {
            // Closest point on the axis line to the pointer ray, minus the arrow length.
            let ray = self.projection.ray(m);
            let w0 = tip - ray.origin;
            let a = dir.dot(dir);
            let b = dir.dot(ray.direction);
            let c = ray.direction.dot(ray.direction);
            let d = dir.dot(w0);
            let e = ray.direction.dot(w0);
            let den = a * c - b * b;
            if den.abs() > 1e-6 {
                let s = (b * e - c * d) / den;
                return at + dir * s;
            }
        }
        at
    }

    /// Unity `Handles.PositionHandle`: three axis sliders and a free-move center.
    pub fn position_handle(&mut self, at: Vector3) -> Vector3 {
        let mut p = at;
        for (axis, color) in [
            (Vector3::X, Color::new(0.9, 0.25, 0.2, 1.0)),
            (Vector3::Y, Color::new(0.45, 0.85, 0.2, 1.0)),
            (Vector3::Z, Color::new(0.25, 0.5, 0.95, 1.0)),
        ] {
            let q = self.slider(at, axis, color);
            if q != at {
                p = q;
            }
        }
        let q = self.free_move_handle(at, 5.0, Color::WHITE);
        if q != at {
            p = q;
        }
        p
    }

    /// The pointer's world ray over the Scene view (Unity `HandleUtility.GUIPointToWorldRay`).
    pub fn pointer_ray(&self) -> Option<Ray> {
        let p = self.ui.ctx().pointer_hover_pos()?;
        self.projection
            .rect()
            .contains(p)
            .then(|| self.projection.ray(p))
    }

    /// Whether the primary button was clicked in the Scene view this frame (not on a handle).
    pub fn clicked(&self) -> bool {
        !self.used
            && self.ui.ctx().input(|i| i.pointer.primary_clicked())
            && self
                .ui
                .ctx()
                .pointer_interact_pos()
                .is_some_and(|p| self.projection.rect().contains(p))
    }

    /// Marks the pointer as used (the Scene view then skips its click selection).
    pub fn use_event(&mut self) {
        self.used = true;
    }
}

/// Unity `EditorTool`: a Scene view tool next to Move/Rotate/Scale in the Tools overlay.
pub trait EditorTool: Send + Sync + 'static {
    /// Unity `OnToolGUI`: draws and handles the tool in the Scene view.
    fn on_tool_gui(&mut self, gui: &mut SceneGui);
    /// Unity `IsAvailable`: whether the tool can be picked now.
    fn is_available(&self, _ctx: &EditorContext) -> bool {
        true
    }
    /// Unity `OnActivated`.
    fn on_activated(&mut self, _ctx: &mut EditorContext) {}
    /// Unity `OnWillBeDeactivated`.
    fn on_deactivated(&mut self, _ctx: &mut EditorContext) {}
    /// Contents of the Tool Settings overlay while the tool is active.
    fn tool_settings(&mut self, _ctx: &mut EditorContext, _ui: &mut egui::Ui) {}
}

/// What a property drawer draws with (Unity `PropertyDrawer.OnGUI`'s position, property
/// and label).
pub struct PropertyGui<'a> {
    ui: &'a mut egui::Ui,
    world: &'a World,
    field: &'a FieldInfo,
    label: String,
    salt: String,
    default: &'a dyn Fn(&mut egui::Ui, &mut Value, &str) -> bool,
    prefix: &'a dyn Fn(&mut egui::Ui, &str) -> egui::Response,
}

impl<'a> PropertyGui<'a> {
    /// Used by the editor.
    pub fn new(
        ui: &'a mut egui::Ui,
        world: &'a World,
        field: &'a FieldInfo,
        label: String,
        salt: String,
        default: &'a dyn Fn(&mut egui::Ui, &mut Value, &str) -> bool,
        prefix: &'a dyn Fn(&mut egui::Ui, &str) -> egui::Response,
    ) -> Self {
        Self {
            ui,
            world,
            field,
            label,
            salt,
            default,
            prefix,
        }
    }

    pub fn ui(&mut self) -> &mut egui::Ui {
        self.ui
    }

    pub fn world(&self) -> &World {
        self.world
    }

    /// The field's reflection info (name, range, tooltip, attributes).
    pub fn field(&self) -> &FieldInfo {
        self.field
    }

    /// The label the Inspector would show.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// A unique id salt for widgets.
    pub fn salt(&self) -> &str {
        &self.salt
    }

    /// Unity `EditorGUI.PropertyField`: the default row for this value.
    pub fn default_property_field(&mut self, value: &mut Value) -> bool {
        (self.default)(self.ui, value, &self.label)
    }

    /// A row with the field's label on the left, in the Inspector's label column (Unity
    /// `EditorGUI.PrefixLabel`).
    pub fn row<R>(&mut self, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let label = self.label.clone();
        let tooltip = self.field.tooltip;
        let prefix = self.prefix;
        self.ui
            .horizontal(|ui| {
                let r = prefix(ui, &label);
                if let Some(t) = tooltip {
                    r.on_hover_text(t);
                }
                add(ui)
            })
            .inner
    }
}

/// Draws one property. Returns whether the value changed.
pub type PropertyDrawerFn = fn(&mut PropertyGui, &mut Value) -> bool;

type MenuFn = fn(&mut EditorContext);
type ValidateFn = fn(&EditorContext) -> bool;

/// A registered menu item.
#[derive(Clone)]
pub struct MenuItemDef {
    /// `Tools/My Tool`; a new top-level name adds a menu to the menu bar.
    pub path: String,
    /// Lower first (Unity's priority); items 11+ apart get a separator between them.
    pub priority: i32,
    /// `Ctrl+Shift+G` style (Ctrl is Cmd on macOS).
    pub shortcut: Option<String>,
    pub run: MenuFn,
    pub validate: Option<ValidateFn>,
}

/// Builder returned by [`EditorExtensions::menu_item`].
pub struct MenuItemBuilder<'a>(&'a mut MenuItemDef);

impl MenuItemBuilder<'_> {
    /// Unity's validate function (`[MenuItem(path, true)]`): the item is greyed out when
    /// this returns false.
    pub fn validate(self, f: ValidateFn) -> Self {
        self.0.validate = Some(f);
        self
    }

    pub fn priority(self, p: i32) -> Self {
        self.0.priority = p;
        self
    }

    /// A keyboard shortcut: `Ctrl+Shift+G`, `Alt+F1`, `F5` (Ctrl means Cmd on macOS).
    pub fn shortcut(self, s: &str) -> Self {
        self.0.shortcut = Some(s.to_owned());
        self
    }
}

type Factory<T> = Box<dyn Fn() -> Box<T> + Send + Sync>;

pub struct WindowDef {
    pub title: String,
    pub menu: String,
    make: Factory<dyn EditorWindow>,
}

pub struct OverlayDef {
    pub id: String,
    instance: Option<Box<dyn Overlay>>,
}

pub struct ToolDef {
    pub name: String,
    pub icon: String,
    instance: Option<Box<dyn EditorTool>>,
}

/// Every editor extension the Scripts library registered (a World resource in the editor).
#[derive(Default)]
pub struct EditorExtensions {
    windows: Vec<WindowDef>,
    open: HashMap<String, Box<dyn EditorWindow>>,
    menu_items: Vec<MenuItemDef>,
    drawers: HashMap<String, PropertyDrawerFn>,
    overlays: Vec<OverlayDef>,
    tools: Vec<ToolDef>,
}

impl EditorExtensions {
    /// Unity `EditorWindow` with `[MenuItem("Window/...")]` calling `GetWindow`: `menu`
    /// opens it.
    pub fn window<T: EditorWindow + Default>(&mut self, title: &str, menu: &str) {
        self.windows.retain(|w| w.title != title);
        self.windows.push(WindowDef {
            title: title.to_owned(),
            menu: menu.to_owned(),
            make: Box::new(|| Box::new(T::default())),
        });
    }

    /// Unity `[MenuItem(path)]`.
    pub fn menu_item(&mut self, path: &str, run: fn(&mut EditorContext)) -> MenuItemBuilder<'_> {
        self.menu_items.retain(|m| m.path != path);
        self.menu_items.push(MenuItemDef {
            path: path.to_owned(),
            priority: 1000,
            shortcut: None,
            run,
            validate: None,
        });
        MenuItemBuilder(self.menu_items.last_mut().expect("pushed"))
    }

    /// Unity `[CustomPropertyDrawer]`: draws fields whose nested type is named `name`, or
    /// fields with `#[reflect(drawer = "name")]`.
    pub fn property_drawer(&mut self, name: &str, draw: PropertyDrawerFn) {
        self.drawers.insert(name.to_owned(), draw);
    }

    /// Unity `[Overlay(typeof(SceneView), id)]`.
    pub fn overlay<T: Overlay + Default>(&mut self, id: &str) {
        self.overlays.retain(|o| o.id != id);
        self.overlays.push(OverlayDef {
            id: id.to_owned(),
            instance: Some(Box::new(T::default())),
        });
    }

    /// Unity `[EditorTool(name)]`; `icon` is a short text or emoji for the Tools overlay.
    pub fn tool<T: EditorTool + Default>(&mut self, name: &str, icon: &str) {
        self.tools.retain(|t| t.name != name);
        self.tools.push(ToolDef {
            name: name.to_owned(),
            icon: icon.to_owned(),
            instance: Some(Box::new(T::default())),
        });
    }

    pub fn windows(&self) -> &[WindowDef] {
        &self.windows
    }

    pub fn menu_items(&self) -> &[MenuItemDef] {
        &self.menu_items
    }

    pub fn menu_item_def(&self, path: &str) -> Option<&MenuItemDef> {
        self.menu_items.iter().find(|m| m.path == path)
    }

    pub fn drawer_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.drawers.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn overlay_ids(&self) -> Vec<String> {
        self.overlays.iter().map(|o| o.id.clone()).collect()
    }

    /// (name, icon) of each tool.
    pub fn tool_names(&self) -> Vec<(String, String)> {
        self.tools
            .iter()
            .map(|t| (t.name.clone(), t.icon.clone()))
            .collect()
    }

    /// The drawer for a field: by its `drawer` attribute, else its nested type's name.
    pub fn drawer_for(&self, f: &FieldInfo) -> Option<PropertyDrawerFn> {
        if let Some(d) = f.ui.drawer.and_then(|n| self.drawers.get(n)) {
            return Some(*d);
        }
        if self.drawers.is_empty() {
            return None;
        }
        let nested = (f.nested)().map(|i| i.name);
        nested
            .or(f.type_name)
            .and_then(|n| self.drawers.get(n))
            .copied()
    }

    pub fn has_window(&self, title: &str) -> bool {
        self.windows.iter().any(|w| w.title == title)
    }

    /// The open instance of a window (created on first use; `true` when just created).
    pub fn take_window(&mut self, title: &str) -> Option<(Box<dyn EditorWindow>, bool)> {
        if let Some(w) = self.open.remove(title) {
            return Some((w, false));
        }
        let def = self.windows.iter().find(|w| w.title == title)?;
        Some(((def.make)(), true))
    }

    pub fn put_window(&mut self, title: &str, w: Box<dyn EditorWindow>) {
        self.open.insert(title.to_owned(), w);
    }

    /// Forgets a closed window's state (Unity destroys the EditorWindow on close).
    pub fn close_window(&mut self, title: &str) {
        self.open.remove(title);
    }

    pub fn open_window_titles(&self) -> Vec<String> {
        self.open.keys().cloned().collect()
    }

    pub fn take_overlay(&mut self, id: &str) -> Option<Box<dyn Overlay>> {
        self.overlays
            .iter_mut()
            .find(|o| o.id == id)?
            .instance
            .take()
    }

    pub fn put_overlay(&mut self, id: &str, o: Box<dyn Overlay>) {
        if let Some(d) = self.overlays.iter_mut().find(|d| d.id == id) {
            d.instance = Some(o);
        }
    }

    pub fn take_tool(&mut self, name: &str) -> Option<Box<dyn EditorTool>> {
        self.tools
            .iter_mut()
            .find(|t| t.name == name)?
            .instance
            .take()
    }

    pub fn put_tool(&mut self, name: &str, t: Box<dyn EditorTool>) {
        if let Some(d) = self.tools.iter_mut().find(|d| d.name == name) {
            d.instance = Some(t);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
            && self.menu_items.is_empty()
            && self.drawers.is_empty()
            && self.overlays.is_empty()
            && self.tools.is_empty()
    }
}

/// Parses `Ctrl+Shift+G` style shortcuts (Ctrl and Cmd both mean the platform's command
/// key).
pub fn parse_shortcut(s: &str) -> Option<egui::KeyboardShortcut> {
    let mut mods = egui::Modifiers::NONE;
    let mut key = None;
    for part in s.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" | "%" => mods |= egui::Modifiers::COMMAND,
            "shift" | "#" => mods |= egui::Modifiers::SHIFT,
            "alt" | "option" | "&" => mods |= egui::Modifiers::ALT,
            _ => {
                key = egui::Key::from_name(part)
                    .or_else(|| egui::Key::from_name(&part.to_ascii_uppercase()))
            }
        }
    }
    key.map(|k| egui::KeyboardShortcut::new(mods, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Counter {
        n: u32,
    }

    impl EditorWindow for Counter {
        fn on_gui(&mut self, ctx: &mut EditorContext, _ui: &mut egui::Ui) {
            self.n += 1;
            ctx.set_status(format!("drawn {}", self.n));
        }
    }

    fn reset(ctx: &mut EditorContext) {
        ctx.record_undo("Reset");
        let id = ctx.world.create("Made by menu");
        ctx.select([id]);
    }

    fn pct(gui: &mut PropertyGui, v: &mut Value) -> bool {
        let _ = gui.label();
        if let Value::Float(f) = v {
            *f = f.clamp(0.0, 1.0);
        }
        false
    }

    #[test]
    fn registers_and_runs_extensions() {
        let mut ext = EditorExtensions::default();
        ext.window::<Counter>("Counter", "Window/Counter");
        ext.menu_item("Tools/Reset", reset)
            .validate(|c| c.selection().is_empty())
            .shortcut("Ctrl+Shift+R");
        ext.property_drawer("Percent", pct);
        assert!(ext.has_window("Counter"));
        let m = ext.menu_item_def("Tools/Reset").unwrap().clone();
        assert_eq!(
            parse_shortcut(m.shortcut.as_deref().unwrap()),
            Some(egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::R
            ))
        );
        let mut world = World::new();
        let mut undos = Vec::new();
        let mut record = |_: &World, l: &str| undos.push(l.to_owned());
        let mut ctx = EditorContext::new(&mut world, vec![], false, &mut record);
        assert!((m.validate.unwrap())(&ctx));
        (m.run)(&mut ctx);
        let (sel, cmds) = ctx.finish();
        assert_eq!(sel.unwrap().len(), 1);
        assert!(cmds.is_empty());
        assert_eq!(undos, ["Reset"]);
        // Window state persists between frames until closed.
        let ctx_ui = egui::Context::default();
        let mut out = ctx_ui.run_ui(Default::default(), |ui| {
            for frame in 0..2 {
                let (mut w, created) = ext.take_window("Counter").unwrap();
                assert_eq!(created, frame == 0);
                let mut rec = |_: &World, _: &str| {};
                let mut c = EditorContext::new(&mut world, vec![], false, &mut rec);
                w.on_gui(&mut c, ui);
                let (_, cmds) = c.finish();
                assert_eq!(
                    cmds,
                    [EditorCommand::Status(format!("drawn {}", frame + 1))]
                );
                ext.put_window("Counter", w);
            }
        });
        out.textures_delta.clear();
        ext.close_window("Counter");
        assert!(ext.take_window("Counter").unwrap().1);
        // Drawers by attribute.
        let mut f = FieldInfo::plain("p", oxide_core::reflect::ValueKind::Float);
        assert!(ext.drawer_for(&f).is_none());
        f.ui.drawer = Some("Percent");
        assert!(ext.drawer_for(&f).is_some());
    }
}
