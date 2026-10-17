//! Oxide game UI, drawn with egui on top of the 3D view.
//!
//! - **Immediate mode** (Unity `OnGUI`): grab `ctx.gui()` in `update` and draw with egui —
//!   windows, labels, buttons, anything.
//! - **Canvas components** (Unity uGUI): `UIText`, `UIImage` (with fill for health/fuel bars),
//!   `UIButton` (sends `on_message("OnClick")` and sets `clicked` for one frame), `UISlider`,
//!   `UIToggle`, anchored to the screen like a RectTransform. A `Canvas` on an ancestor
//!   scales them from a reference resolution (Canvas Scaler "Scale With Screen Size").
//!
//! Reference: <https://docs.unity3d.com/Manual/UIToolkits.html>

use oxide_assets::{AssetId, Handle, Texture2D};
use oxide_core::Reflect;
use oxide_math::{Color, Vector2};
use oxide_scene::{Component, Ctx, GameObjectId, System, World};
use std::collections::HashMap;

pub use egui;

pub mod accessibility;
pub mod layout;
pub mod tmp;
pub use tmp::{TextMeshProUGUI, UiTargetFormat};

/// Unity anchor presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    #[default]
    Center,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Anchor {
    fn align(self) -> egui::Align2 {
        use egui::Align2 as A;
        match self {
            Anchor::TopLeft => A::LEFT_TOP,
            Anchor::TopCenter => A::CENTER_TOP,
            Anchor::TopRight => A::RIGHT_TOP,
            Anchor::MiddleLeft => A::LEFT_CENTER,
            Anchor::Center => A::CENTER_CENTER,
            Anchor::MiddleRight => A::RIGHT_CENTER,
            Anchor::BottomLeft => A::LEFT_BOTTOM,
            Anchor::BottomCenter => A::CENTER_BOTTOM,
            Anchor::BottomRight => A::RIGHT_BOTTOM,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// Unity `Canvas` + `CanvasScaler`.
#[derive(Clone, Debug, Reflect)]
pub struct Canvas {
    /// Scale UI with the screen relative to `reference_resolution` (else 1 unit = 1 point).
    pub scale_with_screen_size: bool,
    pub reference_resolution: Vector2,
    /// 0 = match width, 1 = match height.
    #[reflect(range(0, 1))]
    pub match_width_or_height: f32,
    /// Higher sorts on top.
    pub sort_order: i32,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            scale_with_screen_size: true,
            reference_resolution: Vector2::new(1920.0, 1080.0),
            match_width_or_height: 0.5,
            sort_order: 0,
        }
    }
}

impl Component for Canvas {}

/// Unity `Text` / TextMeshPro label.
#[derive(Clone, Debug, Reflect)]
pub struct UIText {
    pub text: String,
    pub font_size: f32,
    pub color: Color,
    pub anchor: Anchor,
    /// Offset from the anchor in reference pixels (+X right, +Y up, like uGUI).
    pub position: Vector2,
    /// Box size (0 = fit text).
    pub size: Vector2,
    pub alignment: TextAlign,
    pub bold: bool,
    /// Draw a dark shadow behind the text for readability over 3D scenes.
    pub shadow: bool,
}

impl Default for UIText {
    fn default() -> Self {
        Self {
            text: "New Text".into(),
            font_size: 28.0,
            color: Color::WHITE,
            anchor: Anchor::Center,
            position: Vector2::ZERO,
            size: Vector2::ZERO,
            alignment: TextAlign::Center,
            bold: false,
            shadow: true,
        }
    }
}

impl Component for UIText {}

/// Unity `Image` (simple and filled).
#[derive(Clone, Debug, Reflect)]
pub struct UIImage {
    pub color: Color,
    pub texture: Option<Handle<Texture2D>>,
    pub anchor: Anchor,
    pub position: Vector2,
    pub size: Vector2,
    /// Image.fillAmount (horizontal fill from the left), for bars.
    #[reflect(range(0, 1))]
    pub fill_amount: f32,
    /// Background drawn behind the unfilled part.
    pub background: Color,
    pub corner_radius: f32,
}

impl Default for UIImage {
    fn default() -> Self {
        Self {
            color: Color::WHITE,
            texture: None,
            anchor: Anchor::Center,
            position: Vector2::ZERO,
            size: Vector2::new(100.0, 100.0),
            fill_amount: 1.0,
            background: Color::new(0.0, 0.0, 0.0, 0.0),
            corner_radius: 0.0,
        }
    }
}

impl Component for UIImage {}

/// Unity `Button`.
#[derive(Clone, Debug, Reflect)]
pub struct UIButton {
    pub text: String,
    pub font_size: f32,
    pub anchor: Anchor,
    pub position: Vector2,
    pub size: Vector2,
    pub color: Color,
    pub text_color: Color,
    pub interactable: bool,
    /// True for the one frame after a click.
    #[reflect(skip)]
    #[doc(hidden)]
    pub clicked: bool,
}

impl Default for UIButton {
    fn default() -> Self {
        Self {
            text: "Button".into(),
            font_size: 24.0,
            anchor: Anchor::Center,
            position: Vector2::ZERO,
            size: Vector2::new(240.0, 60.0),
            color: Color::new(0.15, 0.15, 0.18, 0.9),
            text_color: Color::WHITE,
            interactable: true,
            clicked: false,
        }
    }
}

impl UIButton {
    pub fn was_clicked(&self) -> bool {
        self.clicked
    }
}

impl Component for UIButton {}

/// Unity `Slider`.
#[derive(Clone, Debug, Reflect)]
pub struct UISlider {
    pub value: f32,
    pub min_value: f32,
    pub max_value: f32,
    pub anchor: Anchor,
    pub position: Vector2,
    pub width: f32,
    pub label: String,
}

impl Default for UISlider {
    fn default() -> Self {
        Self {
            value: 0.5,
            min_value: 0.0,
            max_value: 1.0,
            anchor: Anchor::Center,
            position: Vector2::ZERO,
            width: 300.0,
            label: String::new(),
        }
    }
}

impl Component for UISlider {}

/// Unity `Toggle`.
#[derive(Clone, Debug, Reflect)]
pub struct UIToggle {
    pub is_on: bool,
    pub text: String,
    pub anchor: Anchor,
    pub position: Vector2,
    pub font_size: f32,
}

impl Default for UIToggle {
    fn default() -> Self {
        Self {
            is_on: false,
            text: "Toggle".into(),
            anchor: Anchor::Center,
            position: Vector2::ZERO,
            font_size: 22.0,
        }
    }
}

impl Component for UIToggle {}

/// The UI resource: the egui context for this frame plus texture registrations.
pub struct Gui {
    ctx: egui::Context,
    /// Set by the engine between `begin_pass` and `end_pass`.
    pub in_frame: bool,
    /// Texture asset → egui texture id (registered by the engine's renderer).
    pub textures: HashMap<AssetId, egui::TextureId>,
    /// Textures UI components asked for that the renderer hasn't registered yet.
    pub texture_requests: Vec<Handle<Texture2D>>,
    /// Whether the UI is using the pointer/keyboard this frame (game input should ignore it).
    pub wants_pointer: bool,
    pub wants_keyboard: bool,
    /// Screen area the game UI lays out in, in egui points. `None` = the whole window; the
    /// editor sets it to the Game view.
    pub viewport: Option<egui::Rect>,
    /// Synthetic input (MCP UI testing): one batch of events is added to each frame's
    /// input, in order.
    pub injected_input: std::collections::VecDeque<Vec<egui::Event>>,
}

impl Gui {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            ctx,
            in_frame: false,
            textures: HashMap::new(),
            texture_requests: Vec::new(),
            wants_pointer: false,
            wants_keyboard: false,
            viewport: None,
            injected_input: Default::default(),
        }
    }

    /// The egui context to draw with (only between the engine's begin/end of frame).
    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }
}

fn c32(c: Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        (c.r.clamp(0.0, 1.0) * 255.0) as u8,
        (c.g.clamp(0.0, 1.0) * 255.0) as u8,
        (c.b.clamp(0.0, 1.0) * 255.0) as u8,
        (c.a.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

/// `ctx.gui()` — immediate-mode UI from scripts (Unity `OnGUI`).
pub trait GuiExt {
    fn gui(&self) -> Option<egui::Context>;
}

impl GuiExt for Ctx<'_> {
    fn gui(&self) -> Option<egui::Context> {
        self.world
            .try_resource::<Gui>()
            .filter(|g| g.in_frame)
            .map(|g| g.ctx.clone())
    }
}

pub(crate) fn canvas_scale(world: &World, id: GameObjectId, screen: egui::Vec2) -> f32 {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if let Some(cv) = world.get_component::<Canvas>(c) {
            if !cv.scale_with_screen_size {
                return 1.0;
            }
            let w = (screen.x / cv.reference_resolution.x.max(1.0)).ln();
            let h = (screen.y / cv.reference_resolution.y.max(1.0)).ln();
            return (w + (h - w) * cv.match_width_or_height).exp();
        }
        cur = world.parent(c);
    }
    1.0
}

/// Draws Canvas components every frame (after LateUpdate) and delivers button clicks.
#[derive(Default)]
pub struct UiSystem;

impl System for UiSystem {
    fn name(&self) -> &'static str {
        "UI"
    }

    fn always(&mut self, world: &mut World) {
        let Some(gui) = world.try_resource::<Gui>() else {
            return;
        };
        if !gui.in_frame {
            return;
        }
        let ctx = gui.ctx.clone();
        let textures = gui.textures.clone();
        let screen_rect = gui.viewport.unwrap_or_else(|| ctx.content_rect());
        let screen = screen_rect.size();
        // The editor's Game view is hidden: nothing to lay out.
        if screen.x < 1.0 || screen.y < 1.0 {
            return;
        }
        // RectTransform layout: content sizes, then rects.
        let intrinsic = intrinsic_sizes(world, &ctx);
        let ui_layout = layout::compute(world, &intrinsic, |id| {
            let k = canvas_scale(world, id, screen);
            layout::UiRect::new(0.0, 0.0, screen.x / k, screen.y / k)
        });
        let mut clicks = Vec::new();
        let mut requests = Vec::new();
        let mut drawn = accessibility::DrawnUi::default();
        let order = world.hierarchy_order();
        for id in order {
            if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
                continue;
            }
            let k = canvas_scale(world, id, screen);
            // The object's rect on screen when a RectTransform tree places it.
            let placed = ui_layout.rect(id).map(|r| {
                egui::Rect::from_min_size(
                    screen_rect.min + egui::vec2(r.x * k, r.y * k),
                    egui::vec2(r.w * k, r.h * k),
                )
            });
            let area = |n: u64, anchor: Anchor, pos: Vector2| {
                // Background: over the game image, under the editor's floating windows.
                let a = egui::Area::new(egui::Id::new(("oxide-ui", id.to_bits(), n)))
                    .order(egui::Order::Background);
                match placed {
                    Some(r) => a.pivot(egui::Align2::LEFT_TOP).fixed_pos(r.min),
                    None => a
                        .pivot(anchor.align())
                        .fixed_pos(
                            screen_rect.min
                                + egui::vec2(
                                    screen.x * anchor.align().x().to_factor(),
                                    screen.y * anchor.align().y().to_factor(),
                                )
                                + egui::vec2(pos.x * k, -pos.y * k),
                        )
                        .constrain_to(screen_rect),
                }
            };
            if let Some(img) = world.get_component::<UIImage>(id).cloned() {
                let tex = img
                    .texture
                    .as_ref()
                    .and_then(|h| textures.get(&h.id()).copied());
                if let Some(h) = &img.texture
                    && tex.is_none()
                {
                    requests.push(h.clone());
                }
                let shown =
                    area(1, img.anchor, img.position)
                        .interactable(false)
                        .show(&ctx, |ui| {
                            let size = placed
                                .map_or(egui::vec2(img.size.x * k, img.size.y * k), |r| r.size());
                            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                            let r = img.corner_radius * k;
                            ui.painter().rect_filled(rect, r, c32(img.background));
                            let mut fill = rect;
                            fill.max.x =
                                rect.min.x + rect.width() * img.fill_amount.clamp(0.0, 1.0);
                            match tex {
                                Some(t) => {
                                    let uv = egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(img.fill_amount.clamp(0.0, 1.0), 1.0),
                                    );
                                    ui.painter().image(t, fill, uv, c32(img.color));
                                }
                                None => {
                                    ui.painter().rect_filled(fill, r, c32(img.color));
                                }
                            }
                        });
                drawn.rects.insert(id, shown.response.rect);
            }
            if let Some(t) = world.get_component::<UIText>(id).cloned() {
                let shown = area(2, t.anchor, t.position)
                    .interactable(false)
                    .show(&ctx, |ui| {
                        let mut rt = egui::RichText::new(&t.text)
                            .size(t.font_size * k)
                            .color(c32(t.color));
                        if t.bold {
                            rt = rt.strong();
                        }
                        let galley_job = |ui: &egui::Ui, color: egui::Color32| {
                            let mut r = egui::RichText::new(&t.text)
                                .size(t.font_size * k)
                                .color(color);
                            if t.bold {
                                r = r.strong();
                            }
                            egui::WidgetText::from(r).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Extend),
                                f32::INFINITY,
                                egui::TextStyle::Body,
                            )
                        };
                        let galley = galley_job(ui, c32(t.color));
                        let size = if let Some(r) = placed {
                            r.size()
                        } else if t.size.x > 0.0 {
                            egui::vec2(t.size.x * k, galley.size().y.max(t.size.y * k))
                        } else {
                            galley.size()
                        };
                        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                        let x = match t.alignment {
                            TextAlign::Left => rect.min.x,
                            TextAlign::Center => rect.center().x - galley.size().x / 2.0,
                            TextAlign::Right => rect.max.x - galley.size().x,
                        };
                        let pos = egui::pos2(x, rect.center().y - galley.size().y / 2.0);
                        if t.shadow {
                            let shadow = galley_job(ui, egui::Color32::from_black_alpha(160));
                            ui.painter().galley(
                                pos + egui::vec2(1.5, 1.5) * k.max(0.5),
                                shadow,
                                egui::Color32::BLACK,
                            );
                        }
                        ui.painter().galley(pos, galley, c32(t.color));
                        let _ = rt;
                    });
                drawn.rects.insert(id, shown.response.rect);
            }
            if world.get_component::<TextMeshProUGUI>(id).is_some()
                && let Some(r) = tmp::draw(world, id, &ctx, screen_rect, placed)
            {
                drawn.rects.insert(id, r);
            }
            if let Some(b) = world.get_component::<UIButton>(id).cloned() {
                let mut clicked = false;
                let shown = area(3, b.anchor, b.position).show(&ctx, |ui| {
                    let text = egui::RichText::new(&b.text)
                        .size(b.font_size * k)
                        .color(c32(b.text_color));
                    let btn = egui::Button::new(text).fill(c32(b.color)).min_size(
                        placed.map_or(egui::vec2(b.size.x * k, b.size.y * k), |r| r.size()),
                    );
                    let r = ui.add_enabled(b.interactable, btn);
                    clicked = r.clicked();
                    r.id
                });
                drawn.widgets.insert(id, shown.inner);
                drawn.rects.insert(id, shown.response.rect);
                if let Some(bm) = world.get_component_mut::<UIButton>(id) {
                    bm.clicked = clicked;
                }
                if clicked {
                    clicks.push(id);
                }
            }
            if let Some(s) = world.get_component::<UISlider>(id).cloned() {
                let mut v = s.value;
                let shown = area(4, s.anchor, s.position).show(&ctx, |ui| {
                    ui.spacing_mut().slider_width = match placed {
                        // The rect holds the slider and its label.
                        Some(r) => {
                            let label = intrinsic.get(&id).map_or(0.0, |v| v.x);
                            (r.width() - label * k).max(8.0)
                        }
                        None => s.width * k,
                    };
                    let mut sl = egui::Slider::new(&mut v, s.min_value..=s.max_value);
                    if !s.label.is_empty() {
                        sl = sl.text(&s.label);
                    }
                    ui.add(sl).id
                });
                drawn.widgets.insert(id, shown.inner);
                drawn.rects.insert(id, shown.response.rect);
                if let Some(sm) = world.get_component_mut::<UISlider>(id) {
                    sm.value = v;
                }
            }
            if let Some(t) = world.get_component::<UIToggle>(id).cloned() {
                let mut on = t.is_on;
                let shown = area(5, t.anchor, t.position).show(&ctx, |ui| {
                    ui.checkbox(&mut on, egui::RichText::new(&t.text).size(t.font_size * k))
                        .id
                });
                drawn.widgets.insert(id, shown.inner);
                drawn.rects.insert(id, shown.response.rect);
                if let Some(tm) = world.get_component_mut::<UIToggle>(id) {
                    tm.is_on = on;
                }
            }
        }
        if let Some(g) = world.try_resource_mut::<Gui>() {
            g.texture_requests.extend(requests);
        }
        world.resources.insert(ui_layout);
        // Screen readers: Accessible objects and the script hierarchy.
        accessibility::update(world, &ctx, screen_rect, &drawn);
        for id in clicks {
            world.send_message(id, "OnClick", oxide_core::reflect::Value::Null);
        }
    }
}

/// Content sizes (canvas units) of text-bearing UI components: what layout groups and
/// Content Size Fitters see as their preferred size.
fn intrinsic_sizes(world: &mut World, ctx: &egui::Context) -> HashMap<GameObjectId, Vector2> {
    let measure = |text: &str, size: f32| {
        let s = ctx.fonts_mut(|f| {
            f.layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(size.max(1.0)),
                egui::Color32::WHITE,
            )
            .size()
        });
        Vector2::new(s.x, s.y)
    };
    let mut out = HashMap::new();
    for (id, t) in world.query::<UIText>() {
        out.insert(id, measure(&t.text, t.font_size));
    }
    for (id, t) in world.query::<UIToggle>() {
        let m = measure(&t.text, t.font_size);
        out.insert(id, Vector2::new(m.x + 24.0, m.y.max(18.0)));
    }
    for (id, s) in world.query::<UISlider>() {
        if !s.label.is_empty() {
            out.insert(id, Vector2::new(measure(&s.label, 14.0).x + 8.0, 0.0));
        }
    }
    let tmps: Vec<(GameObjectId, TextMeshProUGUI)> = world
        .query::<TextMeshProUGUI>()
        .map(|(id, t)| (id, t.clone()))
        .collect();
    for (id, t) in tmps {
        out.insert(id, tmp::measure(world, id, &t));
    }
    out
}

/// Installs UI components (the engine inserts the `Gui` resource); returns the system.
pub fn install(world: &mut World) -> Box<dyn System> {
    if world
        .try_resource::<accessibility::AssistiveSupport>()
        .is_none()
    {
        world
            .resources
            .insert(accessibility::AssistiveSupport::default());
    }
    let r = &mut world.registry;
    r.register::<Canvas>("UI/Canvas");
    r.register::<UIText>("UI/Text");
    r.register::<TextMeshProUGUI>("UI/Text - TextMeshPro");
    r.register::<UIImage>("UI/Image");
    r.register::<UIButton>("UI/Button");
    r.register::<UISlider>("UI/Slider");
    r.register::<UIToggle>("UI/Toggle");
    r.register::<layout::RectTransform>("Layout/Rect Transform");
    r.register::<layout::HorizontalLayoutGroup>("Layout/Horizontal Layout Group");
    r.register::<layout::VerticalLayoutGroup>("Layout/Vertical Layout Group");
    r.register::<layout::GridLayoutGroup>("Layout/Grid Layout Group");
    r.register::<layout::LayoutElement>("Layout/Layout Element");
    r.register::<layout::ContentSizeFitter>("Layout/Content Size Fitter");
    r.register::<accessibility::Accessible>("UI/Accessible");
    Box::new(UiSystem)
}
