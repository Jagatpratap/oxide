//! Inspector fields and editor windows for curves and gradients: Unity's Curve Editor
//! (keys, tangent handles, tangent modes, wrap modes, presets) and Gradient Editor
//! (colour and alpha keys, Blend/Fixed), plus `MinMaxCurve` / `MinMaxGradient` fields.
//! References: <https://docs.unity3d.com/Manual/EditingCurves.html>,
//! <https://docs.unity3d.com/Manual/EditingValueProperties.html>

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use oxide_core::reflect::{FieldValue, Value};
use oxide_core::{
    AnimationCurve, CurveWrapMode, Gradient, GradientAlphaKey, GradientColorKey, GradientMode,
    Keyframe, MinMaxCurve, MinMaxCurveMode, MinMaxGradient, MinMaxGradientMode, TangentMode,
};
use oxide_math::Color;

const ROW_H: f32 = 20.0;

/// Presets from the project's `.curves` / `.gradients` libraries (the editor refreshes
/// them; fields have no AssetServer), and presets saved from the editors, waiting to be
/// written by the editor.
#[derive(Default)]
pub struct PresetLibraries {
    pub curves: Vec<(String, AnimationCurve)>,
    pub gradients: Vec<(String, Gradient)>,
    pub save_curves: Vec<(String, AnimationCurve)>,
    pub save_gradients: Vec<(String, Gradient)>,
}

pub static PRESETS: std::sync::Mutex<PresetLibraries> = std::sync::Mutex::new(PresetLibraries {
    curves: Vec::new(),
    gradients: Vec::new(),
    save_curves: Vec::new(),
    save_gradients: Vec::new(),
});

fn presets() -> std::sync::MutexGuard<'static, PresetLibraries> {
    PRESETS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
const CURVE_COLOR: Color32 = Color32::from_rgb(230, 80, 80);
const MIN_COLOR: Color32 = Color32::from_rgb(90, 160, 240);

/// Refreshes [`PRESETS`] from the project's preset libraries and writes presets saved from
/// the editors into `Assets/Editor/Presets.curves` / `.gradients`. The editor calls this
/// about once a second.
pub fn sync_presets(assets: &mut oxide_assets::AssetServer) {
    use oxide_scene::{CurvePreset, CurvePresetLibrary, GradientPreset, GradientPresetLibrary};
    let (save_c, save_g) = {
        let mut p = presets();
        (
            std::mem::take(&mut p.save_curves),
            std::mem::take(&mut p.save_gradients),
        )
    };
    if !save_c.is_empty() {
        let path = "Assets/Editor/Presets.curves";
        let mut lib = if assets.load_data(path).is_ok() {
            assets
                .data
                .get_dyn(path)
                .and_then(|d| d.as_any().downcast_ref::<CurvePresetLibrary>().cloned())
                .unwrap_or_default()
        } else {
            CurvePresetLibrary::default()
        };
        lib.presets
            .extend(save_c.into_iter().map(|(name, curve)| CurvePreset { name, curve }));
        if let Err(e) = assets.save_data(path, Box::new(lib)) {
            oxide_core::debug_log_warning!("{path}: {e}");
        }
    }
    if !save_g.is_empty() {
        let path = "Assets/Editor/Presets.gradients";
        let mut lib = if assets.load_data(path).is_ok() {
            assets
                .data
                .get_dyn(path)
                .and_then(|d| d.as_any().downcast_ref::<GradientPresetLibrary>().cloned())
                .unwrap_or_default()
        } else {
            GradientPresetLibrary::default()
        };
        lib.presets
            .extend(save_g.into_iter().map(|(name, gradient)| GradientPreset { name, gradient }));
        if let Err(e) = assets.save_data(path, Box::new(lib)) {
            oxide_core::debug_log_warning!("{path}: {e}");
        }
    }
    let mut curves = vec![];
    for path in assets.paths_of_type("CurvePresetLibrary") {
        if !assets.data.contains(&path) {
            let _ = assets.load_data(&path);
        }
        if let Some(lib) = assets
            .data
            .get_dyn(&path)
            .and_then(|d| d.as_any().downcast_ref::<CurvePresetLibrary>())
        {
            curves.extend(lib.presets.iter().map(|p| (p.name.clone(), p.curve.clone())));
        }
    }
    let mut gradients = vec![];
    for path in assets.paths_of_type("GradientPresetLibrary") {
        if !assets.data.contains(&path) {
            let _ = assets.load_data(&path);
        }
        if let Some(lib) = assets
            .data
            .get_dyn(&path)
            .and_then(|d| d.as_any().downcast_ref::<GradientPresetLibrary>())
        {
            gradients.extend(
                lib.presets
                    .iter()
                    .map(|p| (p.name.clone(), p.gradient.clone())),
            );
        }
    }
    let mut p = presets();
    p.curves = curves;
    p.gradients = gradients;
}

/// Type names drawn by [`value_field`].
pub fn handles(type_name: Option<&str>) -> bool {
    matches!(
        type_name,
        Some("AnimationCurve" | "Gradient" | "MinMaxCurve" | "MinMaxGradient")
    )
}

/// Draws a curve or gradient [`Value`] (by its reflected type name). Returns whether it
/// changed.
pub fn value_field(ui: &mut egui::Ui, type_name: Option<&str>, v: &mut Value, salt: &str) -> bool {
    fn edit<T: FieldValue>(v: &mut Value, f: impl FnOnce(&mut T) -> bool) -> bool {
        let Some(mut t) = T::from_value(v.clone()) else {
            return false;
        };
        let changed = f(&mut t);
        if changed {
            *v = t.to_value();
        }
        changed
    }
    match type_name {
        Some("AnimationCurve") => edit(v, |c: &mut AnimationCurve| curve_field(ui, c, salt)),
        Some("Gradient") => edit(v, |g: &mut Gradient| gradient_field(ui, g, salt)),
        Some("MinMaxCurve") => edit(v, |c: &mut MinMaxCurve| min_max_curve_field(ui, c, salt)),
        Some("MinMaxGradient") => edit(v, |g: &mut MinMaxGradient| {
            min_max_gradient_field(ui, g, salt)
        }),
        _ => false,
    }
}

fn avail(ui: &egui::Ui) -> f32 {
    crate::inspector::avail(ui)
}

// ---------------------------------------------------------------------------------------
// Curves

/// Per-editor state kept in egui's memory under the field's id.
#[derive(Clone, Default)]
struct CurveState {
    open: bool,
    /// `(curve, key)` of the selected key.
    selected: Option<(usize, usize)>,
    /// Visible area in curve space: `[time0, value0, time1, value1]`; `None` frames all.
    view: Option<[f32; 4]>,
    drag: Option<Drag>,
    /// Key under the last right-click.
    menu_key: Option<(usize, usize)>,
    menu_time: f32,
}

#[derive(Clone, Copy)]
enum Drag {
    Key(usize, usize),
    InTangent(usize, usize),
    OutTangent(usize, usize),
    Pan,
}

fn state_id(salt: &str) -> egui::Id {
    egui::Id::new(("curve-editor", salt))
}

fn load<T: Clone + Default + Send + Sync + 'static>(ui: &egui::Ui, id: egui::Id) -> T {
    ui.ctx().data(|d| d.get_temp::<T>(id)).unwrap_or_default()
}

fn store<T: Clone + Send + Sync + 'static>(ui: &egui::Ui, id: egui::Id, v: T) {
    ui.ctx().data_mut(|d| d.insert_temp(id, v));
}

/// A curve field: a small preview that opens the Curve Editor when clicked.
pub fn curve_field(ui: &mut egui::Ui, c: &mut AnimationCurve, salt: &str) -> bool {
    let id = state_id(salt);
    let (rect, r) = ui.allocate_exact_size(vec2(avail(ui), ROW_H), Sense::click());
    draw_curve_preview(ui.painter(), rect, &[(&*c, CURVE_COLOR)], r.hovered());
    let mut st: CurveState = load(ui, id);
    if r.on_hover_text("Click to edit the curve").clicked() {
        st.open = !st.open;
        st.view = None;
    }
    let mut changed = false;
    if st.open {
        let mut open = true;
        egui::Window::new("Curve Editor")
            .id(id.with("window"))
            .open(&mut open)
            .default_size([520.0, 340.0])
            .resizable(true)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                changed = curve_editor(ui, &mut st, &mut [(c, CURVE_COLOR)], None);
            });
        st.open &= open;
    }
    store(ui, id, st);
    changed
}

/// The Curve Editor drawn in place (the Animation window's Curves view). `salt` keeps its
/// view and selection; returns whether a curve changed.
pub fn curves_inline(
    ui: &mut egui::Ui,
    salt: &str,
    curves: &mut [(&mut AnimationCurve, Color32)],
) -> bool {
    let id = state_id(salt);
    let mut st: CurveState = load(ui, id);
    let changed = curve_editor(ui, &mut st, curves, None);
    store(ui, id, st);
    changed
}

/// Thumbnail of one or more curves framed to fit `rect` (Unity's curve field swatch).
fn draw_curve_preview(
    painter: &egui::Painter,
    rect: Rect,
    curves: &[(&AnimationCurve, Color32)],
    hovered: bool,
) {
    painter.rect_filled(rect, 2.0, Color32::from_gray(38));
    let stroke = if hovered {
        Stroke::new(1.0, Color32::from_gray(140))
    } else {
        Stroke::new(1.0, Color32::from_gray(70))
    };
    painter.rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
    let view = frame_curves(curves.iter().map(|(c, _)| *c));
    let inner = rect.shrink(3.0);
    for (c, col) in curves {
        painter.add(Shape::line(
            curve_points(c, view, inner, 48),
            Stroke::new(1.5, *col),
        ));
    }
}

/// Curve-space area that shows every key of `curves`, with a little padding.
fn frame_curves<'a>(curves: impl Iterator<Item = &'a AnimationCurve>) -> [f32; 4] {
    let mut t0 = f32::INFINITY;
    let mut t1 = f32::NEG_INFINITY;
    let mut v0 = f32::INFINITY;
    let mut v1 = f32::NEG_INFINITY;
    for c in curves {
        let (a, b) = c.time_range();
        let (lo, hi) = c.value_range();
        t0 = t0.min(a);
        t1 = t1.max(b);
        v0 = v0.min(lo);
        v1 = v1.max(hi);
    }
    if !t0.is_finite() {
        return [0.0, 0.0, 1.0, 1.0];
    }
    if t1 - t0 < 1e-4 {
        t0 -= 0.5;
        t1 += 0.5;
    }
    if v1 - v0 < 1e-4 {
        v0 -= 0.5;
        v1 += 0.5;
    }
    let pv = (v1 - v0) * 0.08;
    let pt = (t1 - t0) * 0.03;
    [t0 - pt, v0 - pv, t1 + pt, v1 + pv]
}

fn to_screen(view: [f32; 4], rect: Rect, t: f32, v: f32) -> Pos2 {
    pos2(
        rect.left() + (t - view[0]) / (view[2] - view[0]) * rect.width(),
        rect.bottom() - (v - view[1]) / (view[3] - view[1]) * rect.height(),
    )
}

fn to_curve(view: [f32; 4], rect: Rect, p: Pos2) -> (f32, f32) {
    (
        view[0] + (p.x - rect.left()) / rect.width() * (view[2] - view[0]),
        view[1] + (rect.bottom() - p.y) / rect.height() * (view[3] - view[1]),
    )
}

/// Screen points along `c` across the visible time range (wrap modes included).
fn curve_points(c: &AnimationCurve, view: [f32; 4], rect: Rect, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let t = view[0] + (view[2] - view[0]) * i as f32 / n as f32;
            let p = to_screen(view, rect, t, c.evaluate(t));
            pos2(p.x, p.y.clamp(rect.top() - 2000.0, rect.bottom() + 2000.0))
        })
        .collect()
}

/// A "nice" grid step (1, 2 or 5 × 10ⁿ) giving about `count` lines over `range`.
fn grid_step(range: f32, count: f32) -> f32 {
    let raw = (range / count).max(1e-6);
    let p = 10f32.powf(raw.log10().floor());
    let m = raw / p;
    p * if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    }
}

fn fmt_num(x: f32, step: f32) -> String {
    let decimals = (-step.log10().floor()).max(0.0) as usize;
    format!("{x:.decimals$}")
}

/// Unity tangent presets for a key (the right-click menu's first section).
#[derive(Clone, Copy)]
enum KeyPreset {
    ClampedAuto,
    Auto,
    FreeSmooth,
    Flat,
    Broken,
}

fn apply_preset(k: &mut Keyframe, p: KeyPreset) {
    match p {
        KeyPreset::ClampedAuto => {
            k.in_mode = TangentMode::ClampedAuto;
            k.out_mode = TangentMode::ClampedAuto;
            k.broken = false;
        }
        KeyPreset::Auto => {
            k.in_mode = TangentMode::Auto;
            k.out_mode = TangentMode::Auto;
            k.broken = false;
        }
        KeyPreset::FreeSmooth => {
            k.in_mode = TangentMode::Free;
            k.out_mode = TangentMode::Free;
            k.broken = false;
            k.out_tangent = k.in_tangent;
        }
        KeyPreset::Flat => {
            k.in_mode = TangentMode::Free;
            k.out_mode = TangentMode::Free;
            k.broken = false;
            k.in_tangent = 0.0;
            k.out_tangent = 0.0;
        }
        KeyPreset::Broken => {
            k.in_mode = TangentMode::Free;
            k.out_mode = TangentMode::Free;
            k.broken = true;
        }
    }
}

/// The Curve Editor body for one or more curves (`MinMaxCurve` shows min and max, with
/// the area between them shaded). `multiplier` labels the value axis scale. Returns
/// whether any curve changed.
fn curve_editor(
    ui: &mut egui::Ui,
    st: &mut CurveState,
    curves: &mut [(&mut AnimationCurve, Color32)],
    shade_between: Option<(usize, usize)>,
) -> bool {
    let mut changed = false;
    // Toolbar: selected key's numbers and tangent modes, wrap modes, Frame.
    ui.horizontal(|ui| {
        if let Some((ci, ki)) = st.selected
            && let Some((c, _)) = curves.get_mut(ci)
            && ki < c.keys.len()
        {
            let mut k = c.keys[ki];
            ui.label("Time");
            let t = ui.add(
                egui::DragValue::new(&mut k.time)
                    .speed(0.01)
                    .max_decimals(3),
            );
            ui.label("Value");
            let v = ui.add(
                egui::DragValue::new(&mut k.value)
                    .speed(0.01)
                    .max_decimals(3),
            );
            if t.changed() || v.changed() {
                st.selected = Some((ci, c.move_key(ki, k.time, k.value)));
                changed = true;
            }
        } else {
            ui.weak("Double-click the graph to add a key");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("Frame")
                .on_hover_text("Frame all keys (F)")
                .clicked()
            {
                st.view = None;
            }
            if let Some((c, _)) = curves.first_mut() {
                changed |= wrap_combo(ui, "post", &mut c.post_wrap_mode, "After the last key");
                changed |= wrap_combo(ui, "pre", &mut c.pre_wrap_mode, "Before the first key");
                // Min and max curves wrap together.
                let (pre, post) = (c.pre_wrap_mode, c.post_wrap_mode);
                for (o, _) in curves.iter_mut().skip(1) {
                    o.pre_wrap_mode = pre;
                    o.post_wrap_mode = post;
                }
            }
        });
    });

    let presets_h = 34.0;
    let size = vec2(
        ui.available_width(),
        (ui.available_height() - presets_h).max(120.0),
    );
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let view = st
        .view
        .unwrap_or_else(|| frame_curves(curves.iter().map(|(c, _)| &**c)));
    // Keep the frame so drags don't re-frame mid-edit.
    st.view = Some(view);

    // Grid and labels.
    painter.rect_filled(rect, 0.0, Color32::from_gray(32));
    let font = FontId::proportional(10.0);
    let tstep = grid_step(view[2] - view[0], rect.width() / 70.0);
    let vstep = grid_step(view[3] - view[1], rect.height() / 40.0);
    let mut t = (view[0] / tstep).ceil() * tstep;
    while t <= view[2] {
        let x = to_screen(view, rect, t, 0.0).x;
        let strong = (t / tstep).round() as i64 % 5 == 0;
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(if strong { 58 } else { 44 })),
        );
        painter.text(
            pos2(x + 2.0, rect.bottom() - 2.0),
            Align2::LEFT_BOTTOM,
            fmt_num(t, tstep),
            font.clone(),
            Color32::from_gray(130),
        );
        t += tstep;
    }
    let mut v = (view[1] / vstep).ceil() * vstep;
    while v <= view[3] {
        let y = to_screen(view, rect, 0.0, v).y;
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(
                1.0,
                Color32::from_gray(if v.abs() < vstep * 0.01 { 70 } else { 44 }),
            ),
        );
        painter.text(
            pos2(rect.left() + 2.0, y - 1.0),
            Align2::LEFT_BOTTOM,
            fmt_num(v, vstep),
            font.clone(),
            Color32::from_gray(130),
        );
        v += vstep;
    }

    // Shaded band between min and max curves.
    if let Some((a, b)) = shade_between
        && a < curves.len()
        && b < curves.len()
    {
        let n = 64;
        for i in 0..n {
            let ta = view[0] + (view[2] - view[0]) * i as f32 / n as f32;
            let tb = view[0] + (view[2] - view[0]) * (i + 1) as f32 / n as f32;
            let pts = vec![
                to_screen(view, rect, ta, curves[a].0.evaluate(ta)),
                to_screen(view, rect, tb, curves[a].0.evaluate(tb)),
                to_screen(view, rect, tb, curves[b].0.evaluate(tb)),
                to_screen(view, rect, ta, curves[b].0.evaluate(ta)),
            ];
            painter.add(Shape::convex_polygon(
                pts,
                Color32::from_rgba_unmultiplied(200, 120, 120, 40),
                Stroke::NONE,
            ));
        }
    }

    // Curves: dimmer outside the keyed range, where wrap modes apply.
    for (c, col) in curves.iter() {
        let pts = curve_points(c, view, rect, (rect.width() / 3.0) as usize);
        let (k0, k1) = c.time_range();
        let x0 = to_screen(view, rect, k0, 0.0).x;
        let x1 = to_screen(view, rect, k1, 0.0).x;
        let mut seg: Vec<Pos2> = Vec::new();
        let mut inside_prev = None;
        for p in pts {
            let inside = p.x >= x0 - 0.5 && p.x <= x1 + 0.5;
            if inside_prev.is_some_and(|i| i != inside) && !seg.is_empty() {
                let last = *seg.last().unwrap_or(&p);
                let a = if inside_prev == Some(true) { 255 } else { 110 };
                painter.add(Shape::line(
                    std::mem::take(&mut seg),
                    Stroke::new(2.0, col.gamma_multiply(a as f32 / 255.0)),
                ));
                seg.push(last);
            }
            seg.push(p);
            inside_prev = Some(inside);
        }
        let a = if inside_prev == Some(true) { 1.0 } else { 0.43 };
        painter.add(Shape::line(seg, Stroke::new(2.0, col.gamma_multiply(a))));
    }

    // Keys and the selected key's tangent handles.
    const HANDLE_LEN: f32 = 44.0;
    let handle_pos = |k: &Keyframe, slope: f32, out: bool| -> Pos2 {
        let p = to_screen(view, rect, k.time, k.value);
        let sx = rect.width() / (view[2] - view[0]);
        let sy = rect.height() / (view[3] - view[1]);
        let d = vec2(sx, -slope * sy).normalized() * HANDLE_LEN;
        if out { p + d } else { p - d }
    };
    for (ci, (c, col)) in curves.iter().enumerate() {
        for (ki, k) in c.keys.iter().enumerate() {
            let p = to_screen(view, rect, k.time, k.value);
            let sel = st.selected == Some((ci, ki));
            if sel {
                for (slope, out, mode) in [
                    (k.in_tangent, false, k.in_mode),
                    (k.out_tangent, true, k.out_mode),
                ] {
                    let has_side = if out { ki + 1 < c.keys.len() } else { ki > 0 };
                    if !has_side || matches!(mode, TangentMode::Linear | TangentMode::Constant) {
                        continue;
                    }
                    let h = handle_pos(k, slope, out);
                    painter.line_segment([p, h], Stroke::new(1.0, Color32::from_gray(200)));
                    painter.circle_filled(h, 3.5, Color32::from_gray(220));
                }
            }
            painter.circle_filled(
                p,
                if sel { 5.0 } else { 4.0 },
                if sel { Color32::WHITE } else { *col },
            );
            painter.circle_stroke(
                p,
                if sel { 5.0 } else { 4.0 },
                Stroke::new(1.0, Color32::BLACK),
            );
        }
    }

    // Picking: tangent handles of the selection first, then keys.
    let pick = |pos: Pos2,
                curves: &[(&mut AnimationCurve, Color32)],
                selected: Option<(usize, usize)>|
     -> Option<Drag> {
        if let Some((ci, ki)) = selected
            && let Some((c, _)) = curves.get(ci)
            && let Some(k) = c.keys.get(ki)
        {
            if ki + 1 < c.keys.len() && handle_pos(k, k.out_tangent, true).distance(pos) < 7.0 {
                return Some(Drag::OutTangent(ci, ki));
            }
            if ki > 0 && handle_pos(k, k.in_tangent, false).distance(pos) < 7.0 {
                return Some(Drag::InTangent(ci, ki));
            }
        }
        for (ci, (c, _)) in curves.iter().enumerate().rev() {
            for (ki, k) in c.keys.iter().enumerate() {
                if to_screen(view, rect, k.time, k.value).distance(pos) < 8.0 {
                    return Some(Drag::Key(ci, ki));
                }
            }
        }
        None
    };

    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    // Zoom around the pointer with the scroll wheel (Shift / Ctrl zoom one axis).
    if resp.hovered() {
        crate::shortcuts::claim_keys(ui.ctx());
        let (scroll, mods) = ui.input(|i| (i.smooth_scroll_delta.y, i.modifiers));
        if scroll != 0.0
            && let Some(p) = pointer
        {
            let f = (-scroll * 0.003).exp();
            let (ct, cv) = to_curve(view, rect, p);
            let mut nv = view;
            if !mods.shift {
                nv[0] = ct + (view[0] - ct) * f;
                nv[2] = ct + (view[2] - ct) * f;
            }
            if !mods.command {
                nv[1] = cv + (view[1] - cv) * f;
                nv[3] = cv + (view[3] - cv) * f;
            }
            if nv[2] - nv[0] > 1e-4 && nv[3] - nv[1] > 1e-4 {
                st.view = Some(nv);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::F)) {
            st.view = None;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && let Some((ci, ki)) = st.selected.take()
            && let Some((c, _)) = curves.get_mut(ci)
            && c.keys.len() > 1
        {
            c.remove_key(ki);
            changed = true;
        }
    }

    // Pick what was under the pointer when the button went down, not after the drag
    // threshold moved it.
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).or(pointer)
    {
        let middle = ui.input(|i| i.pointer.middle_down() || i.modifiers.alt);
        st.drag = if middle {
            Some(Drag::Pan)
        } else {
            let d = pick(p, curves, st.selected);
            if let Some(Drag::Key(ci, ki)) = d {
                st.selected = Some((ci, ki));
            }
            d.or(Some(Drag::Pan))
        };
    }
    if resp.dragged()
        && let Some(p) = pointer
    {
        match st.drag {
            Some(Drag::Key(ci, ki)) => {
                let (mut t, mut v) = to_curve(view, rect, p);
                // Snap to the grid with Ctrl/Cmd held.
                if ui.input(|i| i.modifiers.command) {
                    t = (t / tstep).round() * tstep;
                    v = (v / vstep).round() * vstep;
                }
                if let Some((c, _)) = curves.get_mut(ci) {
                    let ni = c.move_key(ki, t, v);
                    st.drag = Some(Drag::Key(ci, ni));
                    st.selected = Some((ci, ni));
                    changed = true;
                }
            }
            Some(Drag::InTangent(ci, ki) | Drag::OutTangent(ci, ki)) => {
                let out = matches!(st.drag, Some(Drag::OutTangent(..)));
                if let Some((c, _)) = curves.get_mut(ci)
                    && let Some(k) = c.keys.get(ki).copied()
                {
                    let (t, v) = to_curve(view, rect, p);
                    let dt = t - k.time;
                    let dt = if out { dt.max(1e-5) } else { dt.min(-1e-5) };
                    let slope = ((v - k.value) / dt).clamp(-1e6, 1e6);
                    let k = &mut c.keys[ki];
                    if out {
                        k.out_tangent = slope;
                        k.out_mode = TangentMode::Free;
                        if !k.broken {
                            k.in_tangent = slope;
                            k.in_mode = TangentMode::Free;
                        }
                    } else {
                        k.in_tangent = slope;
                        k.in_mode = TangentMode::Free;
                        if !k.broken {
                            k.out_tangent = slope;
                            k.out_mode = TangentMode::Free;
                        }
                    }
                    c.update_tangents();
                    changed = true;
                }
            }
            Some(Drag::Pan) => {
                let d = resp.drag_delta();
                let dt = -d.x / rect.width() * (view[2] - view[0]);
                let dv = d.y / rect.height() * (view[3] - view[1]);
                st.view = Some([view[0] + dt, view[1] + dv, view[2] + dt, view[3] + dv]);
            }
            None => {}
        }
    }
    if resp.drag_stopped() {
        st.drag = None;
    }
    if resp.clicked()
        && let Some(p) = pointer
    {
        st.selected = match pick(p, curves, st.selected) {
            Some(Drag::Key(ci, ki)) => Some((ci, ki)),
            Some(_) => st.selected,
            None => None,
        };
    }
    if resp.double_clicked()
        && let Some(p) = pointer
        && pick(p, curves, None).is_none()
    {
        // Add a key on the curve nearest the pointer (vertically).
        let (t, _) = to_curve(view, rect, p);
        let ci = curves
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let da = (to_screen(view, rect, t, a.1.0.evaluate(t)).y - p.y).abs();
                let db = (to_screen(view, rect, t, b.1.0.evaluate(t)).y - p.y).abs();
                da.total_cmp(&db)
            })
            .map_or(0, |(i, _)| i);
        if let Some((c, _)) = curves.get_mut(ci) {
            let ki = c.add_key_at(t);
            st.selected = Some((ci, ki));
            changed = true;
        }
    }
    if resp.secondary_clicked()
        && let Some(p) = pointer
    {
        st.menu_key = match pick(p, curves, None) {
            Some(Drag::Key(ci, ki)) => Some((ci, ki)),
            _ => None,
        };
        st.menu_time = to_curve(view, rect, p).0;
        if let Some(k) = st.menu_key {
            st.selected = Some(k);
        }
    }
    resp.context_menu(|ui| {
        changed |= key_menu(ui, st, curves);
    });

    // Presets.
    ui.horizontal(|ui| {
        ui.weak("Presets");
        let (t0, t1) = curves.first().map_or((0.0, 1.0), |(c, _)| c.time_range());
        let (lo, hi) = match curves.first().map(|(c, _)| c.value_range()) {
            Some((a, b)) if b - a > 1e-4 => (a, b),
            _ => (0.0, 1.0),
        };
        let presets: [(&str, AnimationCurve); 6] = [
            ("Constant", AnimationCurve::constant(t0, t1, hi)),
            ("Linear", AnimationCurve::linear(t0, lo, t1, hi)),
            ("Linear down", AnimationCurve::linear(t0, hi, t1, lo)),
            ("Ease in-out", AnimationCurve::ease_in_out(t0, lo, t1, hi)),
            (
                "Ease in",
                AnimationCurve::new([
                    Keyframe::with_tangents(t0, lo, 0.0, 0.0),
                    Keyframe::with_tangents(
                        t1,
                        hi,
                        2.0 * (hi - lo) / (t1 - t0).max(1e-4),
                        2.0 * (hi - lo) / (t1 - t0).max(1e-4),
                    ),
                ]),
            ),
            (
                "Ease out",
                AnimationCurve::new([
                    Keyframe::with_tangents(
                        t0,
                        lo,
                        2.0 * (hi - lo) / (t1 - t0).max(1e-4),
                        2.0 * (hi - lo) / (t1 - t0).max(1e-4),
                    ),
                    Keyframe::with_tangents(t1, hi, 0.0, 0.0),
                ]),
            ),
        ];
        let project: Vec<(String, AnimationCurve)> = self::presets().curves.clone();
        let all = presets
            .into_iter()
            .map(|(n, c)| (n.to_owned(), c, false))
            .chain(project.into_iter().map(|(n, c)| (n, c, true)));
        for (name, preset, from_project) in all {
            let (r, resp) = ui.allocate_exact_size(vec2(40.0, 26.0), Sense::click());
            draw_curve_preview(ui.painter(), r, &[(&preset, CURVE_COLOR)], resp.hovered());
            if from_project {
                ui.painter().rect_stroke(
                    r,
                    2.0,
                    Stroke::new(1.0, Color32::from_rgb(140, 220, 160)),
                    StrokeKind::Inside,
                );
            }
            let tip = if from_project {
                format!("{name} (project preset)")
            } else {
                name.clone()
            };
            if resp.on_hover_text(tip).clicked()
                && let Some((c, _)) = curves.first_mut()
            {
                let (pre, post) = (c.pre_wrap_mode, c.post_wrap_mode);
                **c = preset;
                if !from_project {
                    c.pre_wrap_mode = pre;
                    c.post_wrap_mode = post;
                }
                st.selected = None;
                changed = true;
            }
        }
        if let Some((c, _)) = curves.first()
            && ui
                .small_button("+")
                .on_hover_text("Save this curve as a preset (Assets/Editor/Presets.curves)")
                .clicked()
        {
            let mut p = self::presets();
            let n = p.curves.len() + p.save_curves.len() + 1;
            p.save_curves.push((format!("Curve {n}"), (*c).clone()));
        }
    });
    changed
}

fn wrap_combo(ui: &mut egui::Ui, salt: &str, m: &mut CurveWrapMode, tip: &str) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(("wrap", salt))
        .selected_text(format!("{m:?}"))
        .width(80.0)
        .show_ui(ui, |ui| {
            for o in [
                CurveWrapMode::Clamp,
                CurveWrapMode::Loop,
                CurveWrapMode::PingPong,
            ] {
                changed |= ui.selectable_value(m, o, format!("{o:?}")).changed();
            }
        })
        .response
        .on_hover_text(tip);
    changed
}

/// Right-click menu: a key's tangent modes and Delete, or Add Key on empty space.
fn key_menu(
    ui: &mut egui::Ui,
    st: &mut CurveState,
    curves: &mut [(&mut AnimationCurve, Color32)],
) -> bool {
    let Some((ci, ki)) = st.menu_key else {
        if ui.button("Add Key").clicked() {
            if let Some((c, _)) = curves.first_mut() {
                let ki = c.add_key_at(st.menu_time);
                st.selected = Some((0, ki));
            }
            ui.close();
            return true;
        }
        return false;
    };
    let Some((c, _)) = curves.get_mut(ci) else {
        return false;
    };
    if ki >= c.keys.len() {
        return false;
    }
    let mut changed = false;
    if ui
        .add_enabled(c.keys.len() > 1, egui::Button::new("Delete Key"))
        .clicked()
    {
        c.remove_key(ki);
        st.selected = None;
        ui.close();
        return true;
    }
    ui.separator();
    let k = c.keys[ki];
    let both = |m: TangentMode| k.in_mode == m && k.out_mode == m;
    let presets = [
        (
            "Clamped Auto",
            KeyPreset::ClampedAuto,
            both(TangentMode::ClampedAuto),
        ),
        ("Auto", KeyPreset::Auto, both(TangentMode::Auto)),
        (
            "Free Smooth",
            KeyPreset::FreeSmooth,
            both(TangentMode::Free) && !k.broken,
        ),
        (
            "Flat",
            KeyPreset::Flat,
            both(TangentMode::Free) && !k.broken && k.in_tangent == 0.0,
        ),
        ("Broken", KeyPreset::Broken, k.broken),
    ];
    for (name, p, on) in presets {
        if ui.selectable_label(on, name).clicked() {
            apply_preset(&mut c.keys[ki], p);
            c.update_tangents();
            changed = true;
            ui.close();
        }
    }
    ui.separator();
    for (label, sides) in [
        ("Left Tangent", (true, false)),
        ("Right Tangent", (false, true)),
        ("Both Tangents", (true, true)),
    ] {
        ui.menu_button(label, |ui| {
            for (name, m) in [
                ("Free", TangentMode::Free),
                ("Linear", TangentMode::Linear),
                ("Constant", TangentMode::Constant),
            ] {
                let on = (!sides.0 || k.in_mode == m) && (!sides.1 || k.out_mode == m);
                if ui.selectable_label(on, name).clicked() {
                    let key = &mut c.keys[ki];
                    if sides.0 {
                        key.in_mode = m;
                    }
                    if sides.1 {
                        key.out_mode = m;
                    }
                    // Setting one side on its own breaks the key, as in Unity.
                    if sides != (true, true) {
                        key.broken = true;
                    }
                    c.update_tangents();
                    changed = true;
                    ui.close();
                }
            }
        });
    }
    changed
}

// ---------------------------------------------------------------------------------------
// Gradients

fn color32(c: Color) -> Color32 {
    egui::Rgba::from_rgba_unmultiplied(c.r, c.g, c.b, c.a.clamp(0.0, 1.0)).into()
}

/// Draws `g` across `rect` over a checkerboard (so transparency shows).
fn draw_gradient(painter: &egui::Painter, rect: Rect, g: &Gradient) {
    let cell = (rect.height() / 2.0).max(3.0);
    let cols = (rect.width() / cell).ceil() as i32;
    for i in 0..cols {
        for j in 0..2 {
            let r = Rect::from_min_size(
                pos2(rect.left() + i as f32 * cell, rect.top() + j as f32 * cell),
                vec2(cell, cell),
            )
            .intersect(rect);
            let shade = if (i + j) % 2 == 0 { 200 } else { 140 };
            painter.rect_filled(r, 0.0, Color32::from_gray(shade));
        }
    }
    let n = (rect.width() / 2.0).ceil().max(1.0) as usize;
    for i in 0..n {
        let x0 = rect.left() + rect.width() * i as f32 / n as f32;
        let x1 = rect.left() + rect.width() * (i + 1) as f32 / n as f32;
        let t = (i as f32 + 0.5) / n as f32;
        painter.rect_filled(
            Rect::from_min_max(pos2(x0, rect.top()), pos2(x1 + 0.5, rect.bottom())),
            0.0,
            color32(g.evaluate(t)),
        );
    }
}

#[derive(Clone, Default)]
struct GradientState {
    open: bool,
    /// Selected key: `(alpha?, index)`.
    selected: Option<(bool, usize)>,
    dragging: Option<(bool, usize)>,
}

/// A gradient field: a swatch that opens the Gradient Editor when clicked.
pub fn gradient_field(ui: &mut egui::Ui, g: &mut Gradient, salt: &str) -> bool {
    let id = egui::Id::new(("gradient-editor", salt));
    let (rect, r) = ui.allocate_exact_size(vec2(avail(ui), ROW_H - 2.0), Sense::click());
    draw_gradient(ui.painter(), rect, g);
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(if r.hovered() { 160 } else { 70 })),
        StrokeKind::Inside,
    );
    let mut st: GradientState = load(ui, id);
    if r.on_hover_text("Click to edit the gradient").clicked() {
        st.open = !st.open;
    }
    let mut changed = false;
    if st.open {
        let mut open = true;
        egui::Window::new("Gradient Editor")
            .id(id.with("window"))
            .open(&mut open)
            .default_size([460.0, 210.0])
            .resizable(true)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                changed = gradient_editor(ui, &mut st, g);
            });
        st.open &= open;
    }
    store(ui, id, st);
    changed
}

fn gradient_editor(ui: &mut egui::Ui, st: &mut GradientState, g: &mut Gradient) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Mode");
        egui::ComboBox::from_id_salt("gradient-mode")
            .selected_text(format!("{:?}", g.mode))
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut g.mode, GradientMode::Blend, "Blend")
                    .changed();
                changed |= ui
                    .selectable_value(&mut g.mode, GradientMode::Fixed, "Fixed")
                    .changed();
            });
    });
    let w = ui.available_width();
    let strip = 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, strip * 2.0 + 40.0), Sense::hover());
    let bar = Rect::from_min_max(
        pos2(rect.left() + 6.0, rect.top() + strip),
        pos2(rect.right() - 6.0, rect.bottom() - strip),
    );
    let alpha_strip = Rect::from_min_max(
        pos2(bar.left() - 6.0, rect.top()),
        pos2(bar.right() + 6.0, bar.top()),
    );
    let color_strip = Rect::from_min_max(
        pos2(bar.left() - 6.0, bar.bottom()),
        pos2(bar.right() + 6.0, rect.bottom()),
    );
    let painter = ui.painter_at(rect.expand(2.0));
    draw_gradient(&painter, bar, g);
    painter.rect_stroke(
        bar,
        0.0,
        Stroke::new(1.0, Color32::from_gray(20)),
        StrokeKind::Outside,
    );

    let x_of = |t: f32| bar.left() + t.clamp(0.0, 1.0) * bar.width();
    let t_of = |x: f32| ((x - bar.left()) / bar.width()).clamp(0.0, 1.0);

    for (alpha, strip_rect) in [(true, alpha_strip), (false, color_strip)] {
        let resp = ui.interact(
            strip_rect,
            ui.id().with(("strip", alpha)),
            Sense::click_and_drag(),
        );
        let times: Vec<f32> = if alpha {
            g.alpha_keys.iter().map(|k| k.time).collect()
        } else {
            g.color_keys.iter().map(|k| k.time).collect()
        };
        // Markers: a little house shape pointing at the bar.
        for (i, t) in times.iter().enumerate() {
            let x = x_of(*t);
            let sel = st.selected == Some((alpha, i));
            let fill = if alpha {
                Color32::from_gray((g.alpha_keys[i].alpha.clamp(0.0, 1.0) * 255.0) as u8)
            } else {
                let c = g.color_keys[i].color;
                color32(Color::new(c.r, c.g, c.b, 1.0))
            };
            let (tip, base) = if alpha {
                (bar.top(), strip_rect.top() + 1.0)
            } else {
                (bar.bottom(), strip_rect.bottom() - 1.0)
            };
            let mid = (tip + base) * 0.5;
            let pts = vec![
                pos2(x, tip),
                pos2(x + 6.0, mid),
                pos2(x + 6.0, base),
                pos2(x - 6.0, base),
                pos2(x - 6.0, mid),
            ];
            let outline = if sel {
                Color32::from_rgb(80, 160, 255)
            } else {
                Color32::from_gray(20)
            };
            painter.add(Shape::convex_polygon(
                pts,
                fill,
                Stroke::new(if sel { 2.0 } else { 1.0 }, outline),
            ));
        }
        let hit = |p: Pos2| -> Option<usize> {
            times
                .iter()
                .enumerate()
                .filter(|(_, t)| (x_of(**t) - p.x).abs() < 7.0)
                .min_by(|a, b| {
                    (x_of(*a.1) - p.x)
                        .abs()
                        .total_cmp(&(x_of(*b.1) - p.x).abs())
                })
                .map(|(i, _)| i)
        };
        let pointer = resp.interact_pointer_pos();
        if resp.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin()).or(pointer)
        {
            st.dragging = hit(p).map(|i| (alpha, i));
            if st.dragging.is_some() {
                st.selected = st.dragging;
            }
        }
        if resp.dragged()
            && let (Some((a, i)), Some(p)) = (st.dragging, pointer)
            && a == alpha
        {
            let t = t_of(p.x);
            if alpha {
                g.alpha_keys[i].time = t;
            } else {
                g.color_keys[i].time = t;
            }
            changed = true;
            // Dragging a key well away from the bar deletes it (Unity), keeping at least one.
            let away = (p.y - strip_rect.center().y).abs() > 40.0;
            let count = if alpha {
                g.alpha_keys.len()
            } else {
                g.color_keys.len()
            };
            if away && count > 1 {
                if alpha {
                    g.alpha_keys.remove(i);
                } else {
                    g.color_keys.remove(i);
                }
                st.dragging = None;
                st.selected = None;
            }
        }
        if resp.drag_stopped() {
            st.dragging = None;
        }
        if resp.clicked()
            && let Some(p) = pointer
        {
            match hit(p) {
                Some(i) => st.selected = Some((alpha, i)),
                None => {
                    // A click on empty strip adds a key with the gradient's current value.
                    let t = t_of(p.x);
                    let now = g.evaluate(t);
                    if alpha && g.alpha_keys.len() < Gradient::MAX_KEYS {
                        g.alpha_keys.push(GradientAlphaKey {
                            alpha: now.a,
                            time: t,
                        });
                        st.selected = Some((true, g.alpha_keys.len() - 1));
                        changed = true;
                    } else if !alpha && g.color_keys.len() < Gradient::MAX_KEYS {
                        g.color_keys.push(GradientColorKey {
                            color: Color::new(now.r, now.g, now.b, 1.0),
                            time: t,
                        });
                        st.selected = Some((false, g.color_keys.len() - 1));
                        changed = true;
                    }
                }
            }
        }
    }

    // Selected key's value, location and Delete.
    ui.add_space(4.0);
    ui.horizontal(|ui| match st.selected {
        Some((true, i)) if i < g.alpha_keys.len() => {
            ui.label("Alpha");
            changed |= ui
                .add(egui::Slider::new(&mut g.alpha_keys[i].alpha, 0.0..=1.0))
                .changed();
            changed |= location(ui, &mut g.alpha_keys[i].time);
            if ui
                .add_enabled(g.alpha_keys.len() > 1, egui::Button::new("Delete"))
                .clicked()
            {
                g.alpha_keys.remove(i);
                st.selected = None;
                changed = true;
            }
        }
        Some((false, i)) if i < g.color_keys.len() => {
            ui.label("Color");
            let c = &mut g.color_keys[i].color;
            let mut rgb = [c.r, c.g, c.b];
            if ui.color_edit_button_rgb(&mut rgb).changed() {
                (c.r, c.g, c.b) = (rgb[0], rgb[1], rgb[2]);
                changed = true;
            }
            changed |= location(ui, &mut g.color_keys[i].time);
            if ui
                .add_enabled(g.color_keys.len() > 1, egui::Button::new("Delete"))
                .clicked()
            {
                g.color_keys.remove(i);
                st.selected = None;
                changed = true;
            }
        }
        _ => {
            ui.weak("Click above or below the bar to add a key; drag a key away to delete it.");
        }
    });
    if changed {
        // Keep the selection on the same key after sorting.
        let sel_time = st.selected.and_then(|(a, i)| {
            if a {
                g.alpha_keys.get(i).map(|k| k.time)
            } else {
                g.color_keys.get(i).map(|k| k.time)
            }
        });
        let drag_time = st.dragging.and_then(|(a, i)| {
            if a {
                g.alpha_keys.get(i).map(|k| k.time)
            } else {
                g.color_keys.get(i).map(|k| k.time)
            }
        });
        g.sort();
        let find = |a: bool, t: Option<f32>| {
            let t = t?;
            if a {
                g.alpha_keys.iter().position(|k| k.time == t)
            } else {
                g.color_keys.iter().position(|k| k.time == t)
            }
        };
        if let Some((a, _)) = st.selected {
            st.selected = find(a, sel_time).map(|i| (a, i));
        }
        if let Some((a, _)) = st.dragging {
            st.dragging = find(a, drag_time).map(|i| (a, i));
        }
    }
    // Presets: the project's gradient libraries, and saving this one.
    ui.horizontal_wrapped(|ui| {
        ui.weak("Presets");
        let list: Vec<(String, Gradient)> = presets().gradients.clone();
        for (name, preset) in list {
            let (r, resp) = ui.allocate_exact_size(vec2(40.0, 18.0), Sense::click());
            draw_gradient(ui.painter(), r, &preset);
            if resp.on_hover_text(&name).clicked() {
                *g = preset;
                st.selected = None;
                changed = true;
            }
        }
        if ui
            .small_button("+")
            .on_hover_text("Save this gradient as a preset (Assets/Editor/Presets.gradients)")
            .clicked()
        {
            let mut p = presets();
            let n = p.gradients.len() + p.save_gradients.len() + 1;
            p.save_gradients.push((format!("Gradient {n}"), g.clone()));
        }
    });
    changed
}

fn location(ui: &mut egui::Ui, t: &mut f32) -> bool {
    ui.label("Location");
    let mut pct = *t * 100.0;
    let r = ui.add(
        egui::DragValue::new(&mut pct)
            .range(0.0..=100.0)
            .suffix("%")
            .max_decimals(1),
    );
    if r.changed() {
        *t = pct / 100.0;
    }
    r.changed()
}

// ---------------------------------------------------------------------------------------
// MinMaxCurve / MinMaxGradient

fn mode_button<T: Copy + PartialEq + std::fmt::Debug>(
    ui: &mut egui::Ui,
    salt: &str,
    m: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(("minmax-mode", salt))
        .selected_text("")
        .width(16.0)
        .show_ui(ui, |ui| {
            for (o, name) in options {
                changed |= ui.selectable_value(m, *o, *name).changed();
            }
        })
        .response
        .on_hover_text("Mode");
    changed
}

pub fn min_max_curve_field(ui: &mut egui::Ui, c: &mut MinMaxCurve, salt: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        // The mode button sits at the right end, as in Unity.
        let full = avail(ui);
        let button = 26.0;
        let body = (full - button - ui.spacing().item_spacing.x).max(40.0);
        ui.allocate_ui(vec2(body, ROW_H), |ui| {
            ui.set_width(body);
            let gap = ui.spacing().item_spacing.x;
            match c.mode {
                MinMaxCurveMode::Constant => {
                    changed |= ui
                        .add_sized(
                            [body, ROW_H],
                            egui::DragValue::new(&mut c.constant)
                                .speed(0.01)
                                .max_decimals(4),
                        )
                        .changed();
                }
                MinMaxCurveMode::RandomBetweenTwoConstants => {
                    let w = (body - gap) / 2.0;
                    changed |= ui
                        .add_sized(
                            [w, ROW_H],
                            egui::DragValue::new(&mut c.constant_min)
                                .speed(0.01)
                                .max_decimals(4),
                        )
                        .changed();
                    changed |= ui
                        .add_sized(
                            [w, ROW_H],
                            egui::DragValue::new(&mut c.constant_max)
                                .speed(0.01)
                                .max_decimals(4),
                        )
                        .changed();
                }
                MinMaxCurveMode::Curve | MinMaxCurveMode::RandomBetweenTwoCurves => {
                    let w = 56.0f32.min(body * 0.35);
                    changed |= ui
                        .add_sized(
                            [w, ROW_H],
                            egui::DragValue::new(&mut c.curve_multiplier)
                                .speed(0.01)
                                .max_decimals(3),
                        )
                        .on_hover_text("Curve multiplier")
                        .changed();
                    let id = state_id(salt);
                    let (rect, r) =
                        ui.allocate_exact_size(vec2(body - w - gap, ROW_H), Sense::click());
                    let two = c.mode == MinMaxCurveMode::RandomBetweenTwoCurves;
                    if two {
                        draw_curve_preview(
                            ui.painter(),
                            rect,
                            &[(&c.curve_min, MIN_COLOR), (&c.curve_max, CURVE_COLOR)],
                            r.hovered(),
                        );
                    } else {
                        draw_curve_preview(
                            ui.painter(),
                            rect,
                            &[(&c.curve, CURVE_COLOR)],
                            r.hovered(),
                        );
                    }
                    let mut st: CurveState = load(ui, id);
                    if r.on_hover_text("Click to edit").clicked() {
                        st.open = !st.open;
                        st.view = None;
                    }
                    if st.open {
                        let mut open = true;
                        egui::Window::new("Curve Editor")
                            .id(id.with("window"))
                            .open(&mut open)
                            .default_size([520.0, 340.0])
                            .collapsible(false)
                            .show(ui.ctx(), |ui| {
                                changed |= if two {
                                    curve_editor(
                                        ui,
                                        &mut st,
                                        &mut [
                                            (&mut c.curve_min, MIN_COLOR),
                                            (&mut c.curve_max, CURVE_COLOR),
                                        ],
                                        Some((0, 1)),
                                    )
                                } else {
                                    curve_editor(
                                        ui,
                                        &mut st,
                                        &mut [(&mut c.curve, CURVE_COLOR)],
                                        None,
                                    )
                                };
                            });
                        st.open &= open;
                    }
                    store(ui, id, st);
                }
            }
        });
        changed |= mode_button(
            ui,
            salt,
            &mut c.mode,
            &[
                (MinMaxCurveMode::Constant, "Constant"),
                (MinMaxCurveMode::Curve, "Curve"),
                (
                    MinMaxCurveMode::RandomBetweenTwoConstants,
                    "Random Between Two Constants",
                ),
                (
                    MinMaxCurveMode::RandomBetweenTwoCurves,
                    "Random Between Two Curves",
                ),
            ],
        );
    });
    changed
}

pub fn min_max_gradient_field(ui: &mut egui::Ui, g: &mut MinMaxGradient, salt: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let full = avail(ui);
        let body = (full - 26.0 - ui.spacing().item_spacing.x).max(40.0);
        ui.allocate_ui(vec2(body, ROW_H), |ui| {
            ui.set_width(body);
            let gap = ui.spacing().item_spacing.x;
            let color = |ui: &mut egui::Ui, c: &mut Color, w: f32| {
                let mut a = [c.r, c.g, c.b, c.a];
                let r = ui
                    .allocate_ui(vec2(w, ROW_H), |ui| {
                        ui.color_edit_button_rgba_unmultiplied(&mut a)
                    })
                    .inner;
                if r.changed() {
                    *c = Color::new(a[0], a[1], a[2], a[3]);
                }
                r.changed()
            };
            match g.mode {
                MinMaxGradientMode::Color => changed |= color(ui, &mut g.color, body),
                MinMaxGradientMode::RandomBetweenTwoColors => {
                    let w = (body - gap) / 2.0;
                    changed |= color(ui, &mut g.color_min, w);
                    changed |= color(ui, &mut g.color_max, w);
                }
                MinMaxGradientMode::Gradient | MinMaxGradientMode::RandomColor => {
                    changed |= gradient_field(ui, &mut g.gradient, salt);
                }
                MinMaxGradientMode::RandomBetweenTwoGradients => {
                    ui.vertical(|ui| {
                        changed |= gradient_field(ui, &mut g.gradient_min, &format!("{salt}.min"));
                        changed |= gradient_field(ui, &mut g.gradient_max, &format!("{salt}.max"));
                    });
                }
            }
        });
        changed |= mode_button(
            ui,
            salt,
            &mut g.mode,
            &[
                (MinMaxGradientMode::Color, "Color"),
                (MinMaxGradientMode::Gradient, "Gradient"),
                (
                    MinMaxGradientMode::RandomBetweenTwoColors,
                    "Random Between Two Colors",
                ),
                (
                    MinMaxGradientMode::RandomBetweenTwoGradients,
                    "Random Between Two Gradients",
                ),
                (MinMaxGradientMode::RandomColor, "Random Color"),
            ],
        );
    });
    changed
}

#[cfg(test)]
mod tests {

    #[test]
    fn saved_presets_become_project_presets() {
        let dir = std::env::temp_dir().join(format!("oxide-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets")).unwrap();
        let mut assets = oxide_assets::AssetServer::new(&dir);
        oxide_engine::builtin_assets::register(&mut assets);
        let torque = AnimationCurve::ease_in_out(0.0, 100.0, 7000.0, 340.0);
        presets().save_curves.push(("Torque".into(), torque.clone()));
        presets().save_gradients.push(("Fire".into(), Gradient::default()));
        sync_presets(&mut assets);
        assert!(dir.join("Assets/Editor/Presets.curves").is_file());
        let p = presets();
        assert!(p.curves.iter().any(|(n, c)| n == "Torque" && *c == torque));
        assert!(p.gradients.iter().any(|(n, _)| n == "Fire"));
        drop(p);
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    #[test]
    fn grid_steps_are_nice() {
        assert_eq!(grid_step(10.0, 5.0), 2.0);
        assert_eq!(grid_step(7000.0, 7.0), 1000.0);
        assert!((grid_step(1.0, 4.0) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn screen_mapping_round_trips() {
        let view = [0.0, -1.0, 10.0, 1.0];
        let rect = Rect::from_min_size(pos2(5.0, 5.0), vec2(200.0, 100.0));
        let p = to_screen(view, rect, 2.5, 0.5);
        let (t, v) = to_curve(view, rect, p);
        assert!((t - 2.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4);
        assert_eq!(to_screen(view, rect, 0.0, -1.0), rect.left_bottom());
    }
}
