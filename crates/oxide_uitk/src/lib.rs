//! Oxide UI Toolkit: Unity's retained-mode UI for games and tools.
//!
//! A [`UIDocument`] shows a `.uxml` document ([`uxml`]) styled by `.uss` style sheets
//! ([`uss`]) on the screen, scaled by its [`PanelSettings`]. Elements lay out with flexbox
//! (USS `flex-*` properties), Unity's controls respond to the pointer and keyboard, and
//! scripts read clicks and values and change text, classes and styles by element name.
//! Reference: <https://docs.unity3d.com/Manual/UIElements.html>

pub mod panel;
pub mod style;
pub mod uss;
pub mod uxml;

pub use panel::{CONTROLS, KeyInput, Panel, PanelInput, SheetRef, UiEvent, UiEventKind, Value};

use oxide_assets::{Asset, AssetId, AssetServer, AssetTypeInfo, Handle};
use oxide_core::Reflect;
use oxide_math::Vector2;
use oxide_scene::{Component, GameObjectId, System, World};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

/// Marker for `.uxml` handles (Unity `VisualTreeAsset`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisualTreeAsset;

impl Asset for VisualTreeAsset {
    const TYPE_NAME: &'static str = "VisualTreeAsset";
}

/// Marker for `.uss` handles (Unity `StyleSheet`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleSheetAsset;

impl Asset for StyleSheetAsset {
    const TYPE_NAME: &'static str = "StyleSheet";
}

/// Unity `PanelScaleMode`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelScaleMode {
    ConstantPixelSize,
    #[default]
    ScaleWithScreenSize,
}

/// Unity `PanelScreenMatchMode`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenMatchMode {
    #[default]
    MatchWidthOrHeight,
    Shrink,
    Expand,
}

/// Unity `PanelSettings` (`.panelsettings`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct PanelSettings {
    /// Theme style sheet; empty for the built-in Default Runtime Theme.
    pub theme_style_sheet: Option<Handle<StyleSheetAsset>>,
    pub scale_mode: PanelScaleMode,
    /// Constant Pixel Size: the scale.
    pub scale: f32,
    pub reference_resolution: Vector2,
    pub screen_match_mode: ScreenMatchMode,
    /// 0 = match width, 1 = match height.
    pub match_width_or_height: f32,
    pub sort_order: f32,
}

impl Default for PanelSettings {
    fn default() -> Self {
        Self {
            theme_style_sheet: None,
            scale_mode: PanelScaleMode::ScaleWithScreenSize,
            scale: 1.0,
            reference_resolution: Vector2::new(1200.0, 800.0),
            screen_match_mode: ScreenMatchMode::MatchWidthOrHeight,
            match_width_or_height: 0.0,
            sort_order: 0.0,
        }
    }
}

impl Asset for PanelSettings {
    const TYPE_NAME: &'static str = "PanelSettings";
}

impl PanelSettings {
    /// Screen points per panel pixel for a screen size.
    pub fn scale_for(&self, screen: Vector2) -> f32 {
        match self.scale_mode {
            PanelScaleMode::ConstantPixelSize => self.scale.max(0.01),
            PanelScaleMode::ScaleWithScreenSize => {
                let rx = screen.x / self.reference_resolution.x.max(1.0);
                let ry = screen.y / self.reference_resolution.y.max(1.0);
                let s = match self.screen_match_mode {
                    ScreenMatchMode::MatchWidthOrHeight => {
                        let m = self.match_width_or_height.clamp(0.0, 1.0);
                        (rx.ln() * (1.0 - m) + ry.ln() * m).exp()
                    }
                    ScreenMatchMode::Shrink => rx.min(ry),
                    ScreenMatchMode::Expand => rx.max(ry),
                };
                s.max(0.01)
            }
        }
    }
}

/// A change a script asked for before the document's panel was built.
#[derive(Clone, Debug)]
enum Op {
    Text(String, String),
    Value(String, Value),
    Class(String, String, Option<bool>),
    Display(String, bool),
    Style(String, String, String),
}

#[derive(Default)]
#[doc(hidden)]
pub struct DocumentState {
    pub panel: Option<Panel>,
    /// What the panel was built from (document and style sheet file times).
    stamp: Vec<Option<SystemTime>>,
    pending: Vec<Op>,
    /// Events of the last UI pass (read by scripts in the next Update).
    pub events: Vec<UiEvent>,
    /// Screen rect of the panel last frame (egui points).
    pub screen: Option<egui::Rect>,
    pub scale: f32,
    pub error: Option<String>,
}

impl Clone for DocumentState {
    fn clone(&self) -> Self {
        // A copied document builds its own panel.
        Self::default()
    }
}

impl std::fmt::Debug for DocumentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentState")
            .field("built", &self.panel.is_some())
            .finish()
    }
}

/// Unity `UIDocument`.
#[derive(Clone, Debug, Reflect)]
pub struct UIDocument {
    pub panel_settings: Option<Handle<PanelSettings>>,
    pub source_asset: Option<Handle<VisualTreeAsset>>,
    pub sort_order: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub state: DocumentState,
}

impl Default for UIDocument {
    fn default() -> Self {
        Self {
            panel_settings: None,
            source_asset: None,
            sort_order: 0.0,
            state: DocumentState::default(),
        }
    }
}

impl Component for UIDocument {}

impl UIDocument {
    pub fn new(source: &str) -> Self {
        Self {
            source_asset: Some(Handle::from_path(source.to_owned())),
            ..Default::default()
        }
    }

    fn with<R>(&mut self, name: &str, f: impl FnOnce(&mut panel::Element) -> R) -> Option<R> {
        let p = self.state.panel.as_mut()?;
        let i = p.find(name)?;
        Some(f(&mut p.elements[i]))
    }

    /// The element was clicked in the last UI pass (Unity `Button.clicked`).
    pub fn clicked(&self, name: &str) -> bool {
        self.state
            .events
            .iter()
            .any(|e| e.kind == UiEventKind::Click && e.element == name)
    }

    /// The control's new value if it changed in the last UI pass (`ChangeEvent`).
    pub fn changed(&self, name: &str) -> Option<Value> {
        self.state
            .events
            .iter()
            .rev()
            .find(|e| e.kind == UiEventKind::Change && e.element == name)
            .map(|e| e.value.clone())
    }

    /// Enter was pressed in the text field.
    pub fn submitted(&self, name: &str) -> bool {
        self.state
            .events
            .iter()
            .any(|e| e.kind == UiEventKind::Submit && e.element == name)
    }

    pub fn events(&self) -> &[UiEvent] {
        &self.state.events
    }

    /// A control's value (Toggle: Bool, Slider: Float, TextField: Text, ...).
    pub fn value(&self, name: &str) -> Option<Value> {
        let p = self.state.panel.as_ref()?;
        Some(p.elements[p.find(name)?].value.clone())
    }

    pub fn text(&self, name: &str) -> Option<String> {
        let p = self.state.panel.as_ref()?;
        let e = &p.elements[p.find(name)?];
        Some(match e.kind.as_str() {
            "Label" | "Button" | "TextField" => e.value.as_text(),
            _ => e.text(),
        })
    }

    /// Unity `label.text = ...` (and `TextField.value`).
    pub fn set_text(&mut self, name: &str, text: &str) {
        let t = text.to_owned();
        if self
            .with(name, |e| match e.kind.as_str() {
                "Label" | "Button" | "TextField" => e.value = Value::Text(t.clone()),
                _ => match e.attrs.iter_mut().find(|(k, _)| k == "text") {
                    Some((_, v)) => *v = t.clone(),
                    None => e.attrs.push(("text".into(), t.clone())),
                },
            })
            .is_none()
        {
            self.state.pending.push(Op::Text(name.into(), text.into()));
        }
    }

    /// `control.value = ...` (without a change event, like `SetValueWithoutNotify`).
    pub fn set_value(&mut self, name: &str, value: Value) {
        let v = value.clone();
        if self.with(name, |e| e.value = v).is_none() {
            self.state.pending.push(Op::Value(name.into(), value));
        }
    }

    pub fn add_class(&mut self, name: &str, class: &str) {
        self.class(name, class, Some(true));
    }

    pub fn remove_class(&mut self, name: &str, class: &str) {
        self.class(name, class, Some(false));
    }

    pub fn toggle_class(&mut self, name: &str, class: &str) {
        self.class(name, class, None);
    }

    fn class(&mut self, name: &str, class: &str, on: Option<bool>) {
        let c = class.to_owned();
        let done = self.with(name, |e| {
            let has = e.classes.contains(&c);
            match (on.unwrap_or(!has), has) {
                (true, false) => e.classes.push(c.clone()),
                (false, true) => e.classes.retain(|x| *x != c),
                _ => {}
            }
        });
        if done.is_none() {
            self.state
                .pending
                .push(Op::Class(name.into(), class.into(), on));
        }
    }

    /// `style.display = Flex / None`.
    pub fn set_display(&mut self, name: &str, shown: bool) {
        self.set_style(name, "display", if shown { "flex" } else { "none" });
        if self.state.panel.is_none() {
            self.state.pending.pop();
            self.state.pending.push(Op::Display(name.into(), shown));
        }
    }

    /// An inline style (`element.style.width = 100` is `set_style(name, "width", "100px")`).
    pub fn set_style(&mut self, name: &str, prop: &str, value: &str) {
        let (p, v) = (prop.to_owned(), value.to_owned());
        let done = self.with(name, |e| match e.inline.iter_mut().find(|(k, _)| *k == p) {
            Some((_, x)) => *x = v.clone(),
            None => e.inline.push((p.clone(), v.clone())),
        });
        if done.is_none() {
            self.state
                .pending
                .push(Op::Style(name.into(), prop.into(), value.into()));
        }
    }

    fn apply_pending(&mut self) {
        for op in std::mem::take(&mut self.state.pending) {
            match op {
                Op::Text(n, t) => self.set_text(&n, &t),
                Op::Value(n, v) => self.set_value(&n, v),
                Op::Class(n, c, on) => self.class(&n, &c, on),
                Op::Display(n, s) => self.set_display(&n, s),
                Op::Style(n, p, v) => self.set_style(&n, &p, &v),
            }
        }
    }
}

/// A parsed document and the file time it was read at.
type Loaded = (Option<SystemTime>, Result<Arc<uxml::VisualTree>, String>);

/// Parsed documents and style sheets by path, reloaded when the file changes.
#[derive(Default)]
pub struct UiAssets {
    trees: HashMap<String, Loaded>,
    sheets: HashMap<String, (Option<SystemTime>, Arc<uss::StyleSheet>)>,
    checked: HashMap<String, f64>,
    theme: Option<Arc<uss::StyleSheet>>,
}

fn mtime(assets: &AssetServer, path: &str) -> Option<SystemTime> {
    std::fs::metadata(assets.full_path(path))
        .ok()?
        .modified()
        .ok()
}

fn folder(path: &str) -> String {
    path.rsplit_once('/')
        .map_or(String::new(), |(d, _)| d.to_owned())
}

impl UiAssets {
    /// Files are checked for changes at most twice a second.
    fn fresh(&mut self, path: &str, now: f64) -> bool {
        let last = self.checked.get(path).copied().unwrap_or(-1.0);
        if now - last < 0.5 {
            return true;
        }
        self.checked.insert(path.to_owned(), now);
        false
    }

    pub fn tree(
        &mut self,
        assets: &AssetServer,
        path: &str,
        now: f64,
    ) -> (Option<SystemTime>, Result<Arc<uxml::VisualTree>, String>) {
        if self.fresh(path, now)
            && let Some(t) = self.trees.get(path)
        {
            return t.clone();
        }
        let m = mtime(assets, path);
        if let Some(t) = self.trees.get(path)
            && t.0 == m
            && m.is_some()
        {
            return t.clone();
        }
        let r = std::fs::read_to_string(assets.full_path(path))
            .map_err(|e| format!("{path}: {e}"))
            .and_then(|text| uxml::parse(&text, &folder(path)).map_err(|e| format!("{path}: {e}")))
            .map(Arc::new);
        self.trees.insert(path.to_owned(), (m, r.clone()));
        (m, r)
    }

    pub fn sheet(
        &mut self,
        assets: &AssetServer,
        path: &str,
        now: f64,
    ) -> (Option<SystemTime>, Arc<uss::StyleSheet>) {
        if self.fresh(path, now)
            && let Some(s) = self.sheets.get(path)
        {
            return s.clone();
        }
        let m = mtime(assets, path);
        if let Some(s) = self.sheets.get(path)
            && s.0 == m
            && m.is_some()
        {
            return s.clone();
        }
        let text = std::fs::read_to_string(assets.full_path(path)).unwrap_or_default();
        let (sheet, errors) = uss::parse(&text);
        for e in errors {
            oxide_core::debug_log_warning!("{path}: {e}");
        }
        let v = (m, Arc::new(sheet));
        self.sheets.insert(path.to_owned(), v.clone());
        v
    }

    pub fn theme(&mut self) -> Arc<uss::StyleSheet> {
        self.theme
            .get_or_insert_with(|| Arc::new(uss::parse(uss::DEFAULT_THEME).0))
            .clone()
    }

    /// Forgets cached files (the editor saved one).
    pub fn invalidate(&mut self, path: &str) {
        self.trees.remove(path);
        self.sheets.remove(path);
        self.checked.remove(path);
    }
}

/// Registers `.uxml`, `.uss` and `.panelsettings`.
pub fn register_assets(assets: &mut AssetServer) {
    assets.types.register(
        AssetTypeInfo::new("VisualTreeAsset", &["uxml"])
            .menu("UI Toolkit/UI Document", "NewUXMLTemplate", 260)
            .color(130, 200, 240),
    );
    assets.types.register(
        AssetTypeInfo::new("StyleSheet", &["uss", "tss"])
            .menu("UI Toolkit/Style Sheet", "NewUSSFile", 261)
            .color(240, 200, 130),
    );
    assets.register_data_asset::<PanelSettings>(
        AssetTypeInfo::new("PanelSettings", &["panelsettings"])
            .menu("UI Toolkit/Panel Settings Asset", "PanelSettings", 262)
            .color(180, 160, 240),
    );
}

/// Draws and runs every UI Document (Edit mode too, like Unity's Game view).
#[derive(Default)]
pub struct UitkSystem;

impl System for UitkSystem {
    fn name(&self) -> &'static str {
        "UI Toolkit"
    }

    fn always(&mut self, world: &mut World) {
        run(world);
    }
}

/// One UI pass: input, style, layout and paint of every document.
pub fn run(world: &mut World) {
    let Some(gui) = world.try_resource::<oxide_ui::Gui>() else {
        return;
    };
    if !gui.in_frame {
        return;
    }
    let ctx = gui.ctx().clone();
    let screen = gui.viewport.unwrap_or_else(|| ctx.content_rect());
    if screen.width() < 1.0 || screen.height() < 1.0 {
        return;
    }
    if !world.resources.contains::<UiAssets>() {
        world.resources.insert(UiAssets::default());
    }
    let now = oxide_core::time::realtime_since_startup();
    let mut docs: Vec<(GameObjectId, f32)> = world
        .query::<UIDocument>()
        .map(|(id, d)| (id, d.sort_order))
        .collect();
    docs.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut wants_pointer = false;
    let mut wants_keyboard = false;
    let mut requests: Vec<Handle<oxide_assets::Texture2D>> = Vec::new();
    let textures = world.resource::<oxide_ui::Gui>().textures.clone();
    let (pointer, pressed, released, down, scroll, keyboard) = ctx.input(|i| {
        let mut keyboard = Vec::new();
        for e in &i.events {
            match e {
                egui::Event::Text(t) | egui::Event::Paste(t) => {
                    keyboard.push(KeyInput::Text(t.clone()))
                }
                egui::Event::Key {
                    key, pressed: true, ..
                } => keyboard.push(KeyInput::Key(*key)),
                _ => {}
            }
        }
        (
            i.pointer.latest_pos(),
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.primary_down(),
            i.smooth_scroll_delta.y,
            keyboard,
        )
    });
    for (k, (id, _)) in docs.iter().enumerate() {
        let (src, ps_path) = {
            let Some(d) = world.get_component::<UIDocument>(*id) else {
                continue;
            };
            (
                d.source_asset.as_ref().map(|h| h.path().to_owned()),
                d.panel_settings.as_ref().map(|h| h.path().to_owned()),
            )
        };
        let Some(src) = src else { continue };
        // Panel settings (defaults without an asset).
        let settings = {
            let assets = world.resource_mut::<AssetServer>();
            ps_path
                .and_then(|p| {
                    let h = Handle::<PanelSettings>::from_path(p);
                    assets.data_asset(&h).cloned()
                })
                .unwrap_or_default()
        };
        let mut cache = world.resources.remove::<UiAssets>().unwrap_or_default();
        let assets = world.resource::<AssetServer>();
        let (tm, tree) = cache.tree(assets, &src, now);
        let mut stamp = vec![tm];
        let mut sheets = vec![SheetRef {
            sheet: cache.theme(),
            base: String::new(),
        }];
        if let Some(t) = &settings.theme_style_sheet {
            let (m, s) = cache.sheet(assets, t.path(), now);
            stamp.push(m);
            sheets.push(SheetRef {
                sheet: s,
                base: folder(t.path()),
            });
        }
        if let Ok(t) = &tree {
            for s in &t.styles {
                let (m, sh) = cache.sheet(assets, s, now);
                stamp.push(m);
                sheets.push(SheetRef {
                    sheet: sh,
                    base: folder(s),
                });
            }
        }
        world.resources.insert(cache);
        let Some(d) = world.get_component_mut::<UIDocument>(*id) else {
            continue;
        };
        let tree = match tree {
            Ok(t) => t,
            Err(e) => {
                if d.state.error.as_deref() != Some(&e) {
                    oxide_core::debug_log_error!("{e}");
                }
                d.state.error = Some(e);
                d.state.panel = None;
                continue;
            }
        };
        d.state.error = None;
        if d.state.panel.is_none() || d.state.stamp != stamp {
            d.state.panel = Some(Panel::new(&tree));
            d.state.stamp = stamp;
            d.apply_pending();
        }
        let scale = settings.scale_for(Vector2::new(screen.width(), screen.height()));
        d.state.scale = scale;
        d.state.screen = Some(screen);
        let size = screen.size() / scale;
        let to_panel = |p: egui::Pos2| ((p - screen.min) / scale).to_pos2();
        let inside = pointer.is_some_and(|p| screen.contains(p));
        let input = PanelInput {
            pointer: pointer.filter(|_| inside).map(to_panel),
            pressed: pressed && inside,
            released,
            down,
            scroll: if inside { scroll / scale } else { 0.0 },
            keyboard: keyboard.clone(),
        };
        let panel = d.state.panel.as_mut().expect("built");
        panel.input(&input);
        panel.style(&sheets);
        panel.layout(size, &ctx);
        d.state.events = panel.events.clone();
        wants_pointer |= panel.wants_pointer;
        wants_keyboard |= panel.focus.is_some_and(|f| {
            matches!(
                panel.elements[f].kind.as_str(),
                "TextField" | "IntegerField" | "FloatField"
            )
        });
        let painter = ctx
            .layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new(("oxide-uitk", id.to_bits(), k)),
            ))
            .with_clip_rect(screen);
        let mut tex = |path: &str| {
            let aid = AssetId::from_path(path);
            match textures.get(&aid) {
                Some(t) => Some(*t),
                None => {
                    requests.push(Handle::from_path(path.to_owned()));
                    None
                }
            }
        };
        panel.paint(&painter, screen.min, scale, &mut tex);
    }
    let gui = world.resource_mut::<oxide_ui::Gui>();
    gui.wants_pointer |= wants_pointer;
    gui.wants_keyboard |= wants_keyboard;
    for r in requests {
        if !gui.texture_requests.iter().any(|h| h.path() == r.path()) {
            gui.texture_requests.push(r);
        }
    }
    // Images used by style sheets need loading.
    let paths: Vec<String> = world
        .resource::<oxide_ui::Gui>()
        .texture_requests
        .iter()
        .map(|h| h.path().to_owned())
        .collect();
    let assets = world.resource_mut::<AssetServer>();
    for p in paths {
        if assets.textures.handle_of(&p).is_none()
            && let Err(e) = assets.ensure_loaded(&p)
        {
            oxide_core::debug_log_warning!("{e}");
        }
    }
}

/// Installs the UI Document component; returns the system that draws documents.
pub fn install(world: &mut World) -> Box<dyn System> {
    world
        .registry
        .register::<UIDocument>("UI Toolkit/UI Document");
    if !world.resources.contains::<UiAssets>() {
        world.resources.insert(UiAssets::default());
    }
    Box::new(UitkSystem)
}

/// The document on `id` (scripts: `ctx.world.ui_document(id)`).
pub trait UiDocumentExt {
    fn ui_document(&mut self, id: GameObjectId) -> Option<&mut UIDocument>;
}

impl UiDocumentExt for World {
    fn ui_document(&mut self, id: GameObjectId) -> Option<&mut UIDocument> {
        self.get_component_mut::<UIDocument>(id)
    }
}

/// Unity's template for Create → UI Toolkit → Style Sheet.
pub const USS_TEMPLATE: &str = "";
