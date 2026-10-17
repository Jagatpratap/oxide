//! Scene view overlays (Unity 2021+): small tool panels inside the Scene view that can be
//! dragged, docked to an edge (top, bottom, left, right), collapsed, hidden from the
//! Overlay menu (`` ` `` over the Scene view) and saved as presets.
//! Reference: <https://docs.unity3d.com/Manual/overlays.html>

use crate::{EditorState, Tool};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use oxide_engine::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Where an overlay sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dock {
    Floating,
    Top,
    Bottom,
    Left,
    Right,
}

impl Dock {
    pub const ALL: [Dock; 5] = [
        Dock::Floating,
        Dock::Top,
        Dock::Bottom,
        Dock::Left,
        Dock::Right,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Dock::Floating => "Floating",
            Dock::Top => "Top",
            Dock::Bottom => "Bottom",
            Dock::Left => "Left",
            Dock::Right => "Right",
        }
    }

    /// Docked overlays in a top or bottom toolbar lay their contents out in a row.
    fn horizontal(self) -> bool {
        matches!(self, Dock::Top | Dock::Bottom)
    }
}

/// One overlay's saved placement (in Preferences, like Unity's per-user Scene view layout).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OverlayLayout {
    pub id: String,
    pub visible: bool,
    pub collapsed: bool,
    pub dock: Dock,
    /// Floating position as a fraction of the Scene view (top-left corner).
    pub pos: [f32; 2],
}

/// The overlays the Scene view has, with their default placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    Tools,
    ToolSettings,
    GridAndSnap,
    ViewOptions,
    SceneCamera,
    Orientation,
    Cameras,
    ProBuilder,
}

impl OverlayKind {
    pub const ALL: [OverlayKind; 8] = [
        OverlayKind::Tools,
        OverlayKind::ToolSettings,
        OverlayKind::GridAndSnap,
        OverlayKind::ViewOptions,
        OverlayKind::SceneCamera,
        OverlayKind::Orientation,
        OverlayKind::Cameras,
        OverlayKind::ProBuilder,
    ];

    pub fn id(self) -> &'static str {
        match self {
            OverlayKind::Tools => "Tools",
            OverlayKind::ToolSettings => "Tool Settings",
            OverlayKind::GridAndSnap => "Grid and Snap",
            OverlayKind::ViewOptions => "View Options",
            OverlayKind::SceneCamera => "Scene Camera",
            OverlayKind::Orientation => "Orientation",
            OverlayKind::Cameras => "Cameras",
            OverlayKind::ProBuilder => "ProBuilder",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    fn default_layout(self) -> OverlayLayout {
        let (dock, pos) = match self {
            OverlayKind::Tools => (Dock::Left, [0.0, 0.0]),
            OverlayKind::ToolSettings
            | OverlayKind::GridAndSnap
            | OverlayKind::ViewOptions
            | OverlayKind::SceneCamera => (Dock::Top, [0.0, 0.0]),
            OverlayKind::Orientation => (Dock::Right, [0.0, 0.0]),
            OverlayKind::Cameras => (Dock::Floating, [0.7, 0.62]),
            OverlayKind::ProBuilder => (Dock::Bottom, [0.0, 0.0]),
        };
        OverlayLayout {
            id: self.id().to_owned(),
            visible: true,
            collapsed: false,
            dock,
            pos,
        }
    }

    /// Contextual overlays appear only when they apply (Unity's "contextual" overlays).
    fn applies(self, st: &EditorState, engine: &Engine) -> bool {
        match self {
            OverlayKind::Cameras => selected_camera(st, engine).is_some(),
            OverlayKind::ProBuilder => crate::probuilder_window::has_target(st, &engine.world),
            OverlayKind::Orientation => !st.scene_ui.mode_2d,
            _ => true,
        }
    }
}

/// The default placement of every overlay.
pub fn default_layouts() -> Vec<OverlayLayout> {
    OverlayKind::ALL
        .iter()
        .map(|k| k.default_layout())
        .collect()
}

/// Runtime state (not saved): measured sizes, the drag in progress, the Overlay menu.
#[derive(Default)]
pub struct OverlayState {
    sizes: HashMap<String, Vec2>,
    drag: Option<(String, Vec2)>,
    menu_at: Option<Pos2>,
    /// Height of the top toolbar (all its rows), from the last frame.
    top_height: f32,
    preset_name: String,
    /// Camera preview for the Cameras overlay.
    pub preview: crate::view::ViewTarget,
    /// The camera shown in the Cameras overlay (`None`: the selected one).
    pub preview_camera: Option<oxide_scene::GameObjectId>,
}

fn layout_mut<'a>(st: &'a mut EditorState, id: &str) -> &'a mut OverlayLayout {
    let layouts = &mut st.prefs.overlays;
    if !layouts.iter().any(|l| l.id == id) {
        let l = OverlayKind::from_id(id)
            .map(OverlayKind::default_layout)
            .unwrap_or(OverlayLayout {
                id: id.to_owned(),
                visible: true,
                collapsed: false,
                dock: Dock::Floating,
                pos: [0.1, 0.1],
            });
        layouts.push(l);
    }
    layouts.iter_mut().find(|l| l.id == id).expect("inserted")
}

fn layout(st: &EditorState, id: &str) -> OverlayLayout {
    st.prefs
        .overlays
        .iter()
        .find(|l| l.id == id)
        .cloned()
        .or_else(|| OverlayKind::from_id(id).map(OverlayKind::default_layout))
        // A script's overlay starts floating, like Unity's custom overlays.
        .unwrap_or(OverlayLayout {
            id: id.to_owned(),
            visible: true,
            collapsed: false,
            dock: Dock::Floating,
            pos: [0.1, 0.1],
        })
}

/// A built-in overlay or one from a script's editor extensions.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Builtin(OverlayKind),
    Custom(String),
}

impl Entry {
    fn id(&self) -> &str {
        match self {
            Entry::Builtin(k) => k.id(),
            Entry::Custom(id) => id,
        }
    }
}

/// The Camera component shown in the Cameras overlay.
pub fn selected_camera(st: &EditorState, engine: &Engine) -> Option<oxide_scene::GameObjectId> {
    let w = &engine.world;
    st.overlays
        .preview_camera
        .filter(|&id| w.has_component::<oxide_scene::Camera>(id))
        .or_else(|| {
            st.active()
                .filter(|&id| w.has_component::<oxide_scene::Camera>(id))
        })
}

const MARGIN: f32 = 6.0;
const GAP: f32 = 4.0;

/// Draws every visible overlay over the Scene view `rect`.
pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, rect: Rect) {
    let ctx = ui.ctx().clone();
    // ` over the Scene view opens the Overlay menu (Unity).
    let hovered = ctx.pointer_hover_pos().is_some_and(|p| rect.contains(p));
    if hovered
        && !ctx.egui_wants_keyboard_input()
        && ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Text(t) if t == "`"))
        })
    {
        st.overlays.menu_at = match st.overlays.menu_at {
            Some(_) => None,
            None => ctx.pointer_hover_pos(),
        };
    }

    let mut kinds: Vec<Entry> = OverlayKind::ALL
        .into_iter()
        .filter(|k| layout(st, k.id()).visible && k.applies(st, engine))
        .map(Entry::Builtin)
        .collect();
    let custom: Vec<String> = crate::extensions::all_overlay_ids(&engine.world);
    if !custom.is_empty() {
        let visible: Vec<String> = custom
            .iter()
            .filter(|id| layout(st, id).visible)
            .cloned()
            .collect();
        if !visible.is_empty() {
            let applies = crate::extensions::overlay_ids(st, &mut engine.world);
            kinds.extend(
                visible
                    .into_iter()
                    .filter(|id| applies.contains(id))
                    .map(Entry::Custom),
            );
        }
    }
    // Docked overlays flow along their edge: top/bottom left to right, left/right top down
    // (below the top toolbar).
    // The top toolbar wraps onto more rows when it doesn't fit; side docks start below it
    // (measured last frame).
    let top_h = st.overlays.top_height;
    let mut top_row_h: f32 = 0.0;
    let mut top_bottom = rect.top();
    let mut cursor: HashMap<&'static str, Pos2> = HashMap::from([
        ("Top", rect.left_top() + vec2(MARGIN, MARGIN)),
        ("Bottom", rect.left_bottom() + vec2(MARGIN, -MARGIN)),
        ("Left", rect.left_top() + vec2(MARGIN, top_h + GAP)),
        ("Right", rect.right_top() + vec2(-MARGIN, top_h + GAP)),
    ]);
    for kind in kinds {
        let id = kind.id().to_owned();
        let id = id.as_str();
        let l = layout(st, id);
        let size = st
            .overlays
            .sizes
            .get(id)
            .copied()
            .unwrap_or(vec2(120.0, 28.0));
        let dragging = st.overlays.drag.as_ref().is_some_and(|(d, _)| d == id);
        let pos = if dragging {
            let (_, grab) = st.overlays.drag.clone().expect("dragging");
            ctx.pointer_interact_pos().unwrap_or(rect.center()) - grab
        } else {
            match l.dock {
                Dock::Floating => {
                    rect.min + vec2(l.pos[0] * rect.width(), l.pos[1] * rect.height())
                }
                Dock::Top => {
                    let c = cursor.get_mut("Top").expect("top");
                    if c.x + size.x > rect.right() - MARGIN && c.x > rect.left() + MARGIN {
                        c.x = rect.left() + MARGIN;
                        c.y += top_row_h + GAP;
                        top_row_h = 0.0;
                    }
                    let p = *c;
                    c.x += size.x + GAP;
                    top_row_h = top_row_h.max(size.y);
                    top_bottom = top_bottom.max(p.y + size.y);
                    p
                }
                Dock::Bottom => {
                    let c = cursor.get_mut("Bottom").expect("bottom");
                    let p = *c - vec2(0.0, size.y);
                    c.x += size.x + GAP;
                    p
                }
                Dock::Left => {
                    let c = cursor.get_mut("Left").expect("left");
                    let p = *c;
                    c.y += size.y + GAP;
                    p
                }
                Dock::Right => {
                    let c = cursor.get_mut("Right").expect("right");
                    let p = *c - vec2(size.x, 0.0);
                    c.y += size.y + GAP;
                    p
                }
            }
        };
        // Keep it inside the view.
        let max = (rect.max - size).max(rect.min);
        let pos = pos.clamp(rect.min, max);
        let horizontal = l.dock.horizontal() && !dragging;
        let area = egui::Area::new(egui::Id::new(("scene-overlay", id)))
            .fixed_pos(pos)
            .order(egui::Order::Middle)
            .constrain_to(rect)
            .show(&ctx, |ui| {
                overlay_frame(ui)
                    .show(ui, |ui| {
                        let grip = |ui: &mut egui::Ui| -> egui::Response {
                            let (r, resp) = ui.allocate_exact_size(
                                if horizontal {
                                    vec2(8.0, 20.0)
                                } else {
                                    vec2(20.0, 8.0)
                                },
                                Sense::click_and_drag(),
                            );
                            let c = Color32::from_gray(if resp.hovered() { 200 } else { 120 });
                            for i in 0..3 {
                                let o = (i as f32 - 1.0) * 4.0;
                                let p = if horizontal {
                                    r.center() + vec2(0.0, o)
                                } else {
                                    r.center() + vec2(o, 0.0)
                                };
                                ui.painter().circle_filled(p, 1.2, c);
                            }
                            resp.on_hover_text(format!(
                                "{id} (drag to move, right-click for options)"
                            ))
                        };
                        let mut grip_resp = None;
                        let mut body = |ui: &mut egui::Ui| {
                            grip_resp = Some(grip(ui));
                            if l.collapsed {
                                if ui.small_button(id).on_hover_text("Expand").clicked() {
                                    layout_mut(st, id).collapsed = false;
                                    st.prefs_changed = true;
                                }
                            } else {
                                if !horizontal && kind != Entry::Builtin(OverlayKind::Orientation) {
                                    ui.label(egui::RichText::new(id).small().weak());
                                }
                                match &kind {
                                    Entry::Builtin(k) => content(*k, st, engine, ui, horizontal),
                                    Entry::Custom(c) => {
                                        crate::extensions::overlay_ui(st, engine, ui, c)
                                    }
                                }
                            }
                        };
                        if horizontal {
                            ui.horizontal(|ui| body(ui));
                        } else {
                            ui.vertical(|ui| body(ui));
                        }
                        grip_resp.expect("grip drawn")
                    })
                    .inner
            });
        st.overlays
            .sizes
            .insert(id.to_owned(), area.response.rect.size());
        let grip = area.inner;
        if grip.drag_started()
            && let Some(p) = grip.interact_pointer_pos()
        {
            st.overlays.drag = Some((id.to_owned(), p - area.response.rect.min));
        }
        if dragging && grip.drag_stopped() {
            let r = area.response.rect;
            let edge = 28.0;
            let dock = if r.top() - rect.top() < edge {
                Dock::Top
            } else if rect.bottom() - r.bottom() < edge {
                Dock::Bottom
            } else if r.left() - rect.left() < edge {
                Dock::Left
            } else if rect.right() - r.right() < edge {
                Dock::Right
            } else {
                Dock::Floating
            };
            let l = layout_mut(st, id);
            l.dock = dock;
            l.pos = [
                ((r.left() - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((r.top() - rect.top()) / rect.height()).clamp(0.0, 1.0),
            ];
            st.overlays.drag = None;
            st.prefs_changed = true;
        }
        if grip.double_clicked() {
            let l = layout_mut(st, id);
            l.collapsed = !l.collapsed;
            st.prefs_changed = true;
        }
        grip.context_menu(|ui| {
            let collapsed = layout(st, id).collapsed;
            if ui
                .button(if collapsed { "Expand" } else { "Collapse" })
                .clicked()
            {
                layout_mut(st, id).collapsed = !collapsed;
                st.prefs_changed = true;
                ui.close();
            }
            ui.menu_button("Dock", |ui| {
                for d in Dock::ALL {
                    if ui.radio(layout(st, id).dock == d, d.name()).clicked() {
                        layout_mut(st, id).dock = d;
                        st.prefs_changed = true;
                        ui.close();
                    }
                }
            });
            if ui.button("Hide").clicked() {
                layout_mut(st, id).visible = false;
                st.prefs_changed = true;
                ui.close();
            }
            ui.separator();
            if ui.button("Overlay Menu…").clicked() {
                st.overlays.menu_at = ui.ctx().pointer_interact_pos();
                ui.close();
            }
        });
    }
    let measured = (top_bottom - rect.top()).max(0.0);
    if (measured - st.overlays.top_height).abs() > 0.5 {
        st.overlays.top_height = measured;
        ctx.request_repaint();
    }
    if st.overlays.drag.is_some() && !ctx.input(|i| i.pointer.any_down()) {
        st.overlays.drag = None;
    }
    overlay_menu(st, &ctx, rect, &custom);
}

fn overlay_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(40, 40, 40, 235))
        .stroke(Stroke::new(1.0, Color32::from_gray(25)))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(4, 3))
        .shadow(egui::epaint::Shadow {
            offset: [0, 1],
            blur: 4,
            spread: 0,
            color: Color32::from_black_alpha(80),
        })
        .multiply_with_opacity(if ui.is_enabled() { 1.0 } else { 0.6 })
}

/// Unity's Overlay menu: show or hide each overlay, and overlay presets.
fn overlay_menu(st: &mut EditorState, ctx: &egui::Context, rect: Rect, custom: &[String]) {
    let Some(at) = st.overlays.menu_at else {
        return;
    };
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("scene-overlay-menu"))
        .fixed_pos(at)
        .order(egui::Order::Foreground)
        .constrain_to(rect)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(210.0);
                ui.horizontal(|ui| {
                    ui.strong("Overlays");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            close = true;
                        }
                    });
                });
                ui.separator();
                let ids: Vec<String> = OverlayKind::ALL
                    .iter()
                    .map(|k| k.id().to_owned())
                    .chain(custom.iter().cloned())
                    .collect();
                for id in &ids {
                    let mut v = layout(st, id).visible;
                    if ui.checkbox(&mut v, id.as_str()).changed() {
                        layout_mut(st, id).visible = v;
                        st.prefs_changed = true;
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Show All").clicked() {
                        for id in &ids {
                            layout_mut(st, id).visible = true;
                        }
                        st.prefs_changed = true;
                    }
                    if ui.button("Hide All").clicked() {
                        for id in &ids {
                            layout_mut(st, id).visible = false;
                        }
                        st.prefs_changed = true;
                    }
                });
                ui.separator();
                ui.strong("Overlay Presets");
                if ui.button("Default").clicked() {
                    st.prefs.overlays = default_layouts();
                    st.prefs_changed = true;
                }
                let names: Vec<String> = st.prefs.overlay_presets.keys().cloned().collect();
                for name in names {
                    ui.horizontal(|ui| {
                        if ui.button(&name).clicked()
                            && let Some(p) = st.prefs.overlay_presets.get(&name)
                        {
                            st.prefs.overlays = p.clone();
                            st.prefs_changed = true;
                        }
                        if ui
                            .small_button("🗑")
                            .on_hover_text("Delete preset")
                            .clicked()
                        {
                            st.prefs.overlay_presets.remove(&name);
                            st.prefs_changed = true;
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut st.overlays.preset_name)
                            .hint_text("Preset name")
                            .desired_width(110.0),
                    );
                    let name = st.overlays.preset_name.trim().to_owned();
                    if ui
                        .add_enabled(!name.is_empty(), egui::Button::new("Save Preset"))
                        .clicked()
                    {
                        let all: Vec<OverlayLayout> = OverlayKind::ALL
                            .iter()
                            .map(|k| layout(st, k.id()))
                            .collect();
                        st.prefs.overlay_presets.insert(name, all);
                        st.overlays.preset_name.clear();
                        st.prefs_changed = true;
                    }
                });
            });
        });
    if close
        || ctx.input(|i| i.key_pressed(egui::Key::Escape))
        || (area.response.clicked_elsewhere() && !ctx.input(|i| i.pointer.any_down()))
    {
        st.overlays.menu_at = None;
    }
}

// ---- overlay contents ---------------------------------------------------------------------

fn content(
    kind: OverlayKind,
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &mut egui::Ui,
    h: bool,
) {
    let flow = |ui: &mut egui::Ui, f: &mut dyn FnMut(&mut egui::Ui)| {
        if h {
            ui.horizontal(|ui| f(ui));
        } else {
            ui.vertical(|ui| f(ui));
        }
    };
    match kind {
        OverlayKind::Tools => flow(ui, &mut |ui| {
            for (t, label, tip) in [
                (Tool::Hand, "✋", "View Tool (Q)"),
                (Tool::Move, "✥", "Move Tool (W)"),
                (Tool::Rotate, "⟳", "Rotate Tool (E)"),
                (Tool::Scale, "🗖", "Scale Tool (R)"),
                (Tool::Rect, "▭", "Rect Tool (T)"),
                (Tool::Transform, "⊕", "Transform Tool (Y)"),
            ] {
                let on = st.tool == t && st.custom_tool.is_none();
                if ui
                    .add(egui::Button::selectable(on, label).min_size(vec2(24.0, 22.0)))
                    .on_hover_text(tip)
                    .clicked()
                {
                    crate::extensions::set_custom_tool(st, &mut engine.world, None);
                    st.tool = t;
                }
            }
            // Script tools (Unity `[EditorTool]`), after the built-in ones.
            for (name, icon, available) in crate::extensions::tools(st, &mut engine.world) {
                let on = st.custom_tool.as_deref() == Some(name.as_str());
                let label = if icon.is_empty() { name.clone() } else { icon };
                if ui
                    .add_enabled(
                        available,
                        egui::Button::selectable(on, label).min_size(vec2(24.0, 22.0)),
                    )
                    .on_hover_text(&name)
                    .clicked()
                {
                    let next = (!on).then_some(name);
                    crate::extensions::set_custom_tool(st, &mut engine.world, next);
                }
            }
        }),
        OverlayKind::ToolSettings => flow(ui, &mut |ui| {
            if crate::extensions::tool_settings_ui(st, engine, ui) {
                return;
            }
            let pivot = if st.center_handles { "Center" } else { "Pivot" };
            if ui
                .button(pivot)
                .on_hover_text("Tool handle position (Z)")
                .clicked()
            {
                st.center_handles = !st.center_handles;
            }
            let space = if st.local_handles { "Local" } else { "Global" };
            if ui
                .button(space)
                .on_hover_text("Tool handle rotation (X)")
                .clicked()
            {
                st.local_handles = !st.local_handles;
            }
        }),
        OverlayKind::GridAndSnap => flow(ui, &mut |ui| {
            ui.toggle_value(&mut st.scene_ui.grid, "#")
                .on_hover_text("Show grid");
            let p = &mut st.prefs;
            let mut changed = false;
            ui.label("Move");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut p.move_snap)
                        .range(0.001..=100.0)
                        .speed(0.01),
                )
                .on_hover_text("Move snap increment (hold ⌘/Ctrl while dragging)")
                .changed();
            ui.label("Rotate");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut p.rotate_snap)
                        .range(0.1..=180.0)
                        .suffix("°"),
                )
                .changed();
            ui.label("Scale");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut p.scale_snap)
                        .range(0.001..=10.0)
                        .speed(0.01),
                )
                .changed();
            if changed {
                st.prefs_changed = true;
            }
        }),
        OverlayKind::ViewOptions => flow(ui, &mut |ui| crate::scene_view::view_options(st, ui)),
        OverlayKind::SceneCamera => flow(ui, &mut |ui| crate::scene_view::camera_options(st, ui)),
        OverlayKind::Orientation => {
            let (r, _) = ui.allocate_exact_size(vec2(96.0, 108.0), Sense::hover());
            crate::scene_view::orientation_gizmo_at(st, ui, r.center() - vec2(0.0, 8.0), r);
        }
        OverlayKind::Cameras => cameras_overlay(st, engine, ui),
        OverlayKind::ProBuilder => flow(ui, &mut |ui| {
            crate::probuilder_window::mode_buttons(st, ui);
        }),
    }
}

/// Unity's Cameras overlay: a live view through a scene camera.
fn cameras_overlay(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &engine.world;
    let cams: Vec<(oxide_scene::GameObjectId, String)> = w
        .find_objects_of_type::<oxide_scene::Camera>(false)
        .into_iter()
        .filter_map(|id| w.get(id).map(|o| (id, o.name.clone())))
        .collect();
    let current = selected_camera(st, engine);
    let name = current
        .and_then(|c| cams.iter().find(|(id, _)| *id == c))
        .map_or("(none)".to_owned(), |(_, n)| n.clone());
    egui::ComboBox::from_id_salt("overlay-cameras")
        .selected_text(name)
        .width(160.0)
        .show_ui(ui, |ui| {
            for (id, n) in &cams {
                if ui.selectable_label(current == Some(*id), n).clicked() {
                    st.overlays.preview_camera = Some(*id);
                }
            }
        });
    let size = vec2(224.0, 126.0);
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let ppp = ui.ctx().pixels_per_point();
    let tex = st.overlays.preview.texture_id(
        engine,
        (size.x * ppp).round() as u32,
        (size.y * ppp).round() as u32,
    );
    st.overlays.preview.rect = Some(r);
    ui.painter().image(
        tex,
        r,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// Renders the Cameras overlay's preview (after the Scene view).
pub fn render_preview(st: &mut EditorState, engine: &mut Engine) {
    if st.overlays.preview.rect.take().is_none() {
        return;
    }
    let Some(id) = selected_camera(st, engine) else {
        return;
    };
    let Some(view) = st.overlays.preview.view() else {
        return;
    };
    let w = &engine.world;
    let Some(cam) = w.get_component::<oxide_scene::Camera>(id) else {
        return;
    };
    let mut cv = oxide_engine::extract::camera_view(w, id, cam);
    cv.viewport = [0.0, 0.0, 1.0, 1.0];
    let [pw, ph] = st.overlays.preview.size;
    let rf = oxide_engine::extract::extract(w, Some(vec![cv]));
    let assets = w.resource::<oxide_assets::AssetServer>();
    engine.renderer.render(
        &engine.gpu,
        assets,
        &rf,
        view,
        crate::view::VIEW_FORMAT,
        pw,
        ph,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_round_trip_through_prefs_yaml() {
        let mut p = crate::prefs::EditorPrefs::default();
        p.overlays[0].dock = Dock::Right;
        p.overlays[0].collapsed = true;
        p.overlay_presets.insert("Mine".into(), p.overlays.clone());
        let text = serde_yaml_ng::to_string(&p).unwrap();
        let back: crate::prefs::EditorPrefs = serde_yaml_ng::from_str(&text).unwrap();
        assert_eq!(back, p);
        // Old prefs files without overlays get the defaults.
        let old: crate::prefs::EditorPrefs =
            serde_yaml_ng::from_str("auto_refresh: true\n").unwrap();
        assert_eq!(old.overlays, default_layouts());
    }
}
