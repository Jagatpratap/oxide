//! The Animator window: an Animator Controller's layers and parameters, its state machine
//! as a graph (Entry, Any State, states, transitions), and the selected state's or
//! transition's settings. In Play mode it shows the selected object's live state.
//! Edits save to the `.controller` straight away, as in Unity.
//! Reference: <https://docs.unity3d.com/Manual/AnimatorWindow.html>

use crate::{EditorState, project::AssetPayload};
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use oxide_assets::{
    AnimationClip, AnimatorController, AnimatorControllerLayer, AnimatorLayerBlendingMode,
    AnimatorState, AssetServer, BlendTree2DType, Condition, ConditionMode, Handle, Motion,
    Parameter, ParameterType, Transition,
};
use oxide_engine::Engine;
use oxide_math::Vector2;

const NODE: egui::Vec2 = egui::Vec2::new(150.0, 36.0);
const ENTRY_POS: Pos2 = Pos2::new(40.0, 40.0);
const ANY_POS: Pos2 = Pos2::new(40.0, 140.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sel {
    State(usize),
    Transition(usize),
}

#[derive(Default)]
pub struct AnimatorWindow {
    path: Option<String>,
    layer: usize,
    left_tab: LeftTab,
    selected: Option<Sel>,
    /// Graph scroll offset.
    pan: egui::Vec2,
    dragging: Option<usize>,
    /// "Make Transition": from this state (`None` = Any State) to the next state clicked.
    making: Option<Option<usize>>,
    menu_at: Pos2,
    renaming_param: Option<(usize, String)>,
    error: Option<String>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum LeftTab {
    #[default]
    Layers,
    Parameters,
}

impl AnimatorWindow {
    pub fn open(&mut self, path: &str) {
        if self.path.as_deref() != Some(path) {
            *self = Self {
                path: Some(path.to_owned()),
                ..Default::default()
            };
        }
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

fn unique(name: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == name) {
        return name.to_owned();
    }
    (0..)
        .map(|i| format!("{name} {i}"))
        .find(|n| !taken.contains(n))
        .unwrap_or_else(|| name.to_owned())
}

/// The selected object's Animator, if it plays `path` (for the live view).
fn live_info(
    st: &EditorState,
    engine: &Engine,
    path: &str,
    layer: usize,
) -> Option<oxide_animation::AnimatorStateInfo> {
    if st.play == crate::PlayState::Edit {
        return None;
    }
    let id = *st.selection.last()?;
    let a = engine
        .world
        .get_component::<oxide_animation::Animator>(id)?;
    (a.controller.as_ref()?.path() == path).then(|| a.get_current_animator_state_info(layer))
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    // Follow the selection: an object with an Animator shows its controller (Unity).
    if let Some(&id) = st.selection.last()
        && let Some(a) = engine.world.get_component::<oxide_animation::Animator>(id)
        && let Some(c) = &a.controller
        && st.animator.path() != Some(c.path())
        && !c.path().starts_with("runtime:")
    {
        let p = c.path().to_owned();
        st.animator.open(&p);
    }
    let Some(path) = st.animator.path.clone() else {
        ui.weak("Select an object with an Animator, or double-click an Animator Controller.");
        return;
    };
    let assets = engine.world.resource_mut::<AssetServer>();
    if let Err(e) = assets.ensure_loaded(&path) {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        return;
    }
    let h: Handle<AnimatorController> = Handle::from_path(path.clone());
    let Some(mut c) = assets.controllers.get(&h).cloned() else {
        return;
    };
    let editable = path.ends_with(".controller");
    if c.layers.is_empty() {
        c.base();
    }
    let before = c.clone();
    let w = &mut st.animator;
    w.layer = w.layer.min(c.layers.len() - 1);

    ui.horizontal(|ui| {
        ui.label(RichText::new(path.rsplit('/').next().unwrap_or(&path)).strong());
        if !editable {
            ui.weak("(override controller: edit its base controller)");
        }
        if let Some(e) = &w.error {
            ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        }
    });
    ui.separator();
    let live = live_info(st, engine, &path, st.animator.layer);
    let w = &mut st.animator;
    let h_avail = ui.available_height();
    let total = ui.available_width();
    let left_w = 210.0f32.min(total * 0.25);
    let right_w = 290.0f32.min(total * 0.32);
    let graph_w = (total - left_w - right_w - 16.0).max(200.0);
    let assets = engine.world.resource::<AssetServer>();
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal_top(|ui| {
            ui.allocate_ui(vec2(left_w, h_avail), |ui| {
                ui.set_width(left_w);
                ui.vertical(|ui| left_panel(ui, w, &mut c));
            });
            ui.separator();
            ui.allocate_ui(vec2(graph_w, h_avail), |ui| {
                graph(ui, w, &mut c, live.as_ref(), vec2(graph_w, h_avail), assets);
            });
            ui.separator();
            ui.allocate_ui(vec2(right_w, h_avail), |ui| {
                ui.set_width(right_w);
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("animator-props")
                        .show(ui, |ui| {
                            properties(ui, w, &mut c, assets);
                        });
                });
            });
        });
    });
    if c != before && editable {
        match engine
            .world
            .resource_mut::<AssetServer>()
            .save_animator_controller(&path, &c)
        {
            Ok(_) => st.animator.error = None,
            Err(e) => st.animator.error = Some(e),
        }
    }
}

fn left_panel(ui: &mut egui::Ui, w: &mut AnimatorWindow, c: &mut AnimatorController) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut w.left_tab, LeftTab::Layers, "Layers");
        ui.selectable_value(&mut w.left_tab, LeftTab::Parameters, "Parameters");
    });
    ui.separator();
    match w.left_tab {
        LeftTab::Layers => {
            if ui.small_button("+ Add Layer").clicked() {
                let names: Vec<String> = c.layers.iter().map(|l| l.name.clone()).collect();
                let name = unique("New Layer", &names);
                c.layers.push(AnimatorControllerLayer::new(&name));
                w.layer = c.layers.len() - 1;
                w.selected = None;
            }
            let mut remove = None;
            for i in 0..c.layers.len() {
                let sel = w.layer == i;
                let r = ui.selectable_label(sel, RichText::new(&c.layers[i].name).strong());
                if r.clicked() {
                    w.layer = i;
                    w.selected = None;
                }
                if i > 0 {
                    r.context_menu(|ui| {
                        if ui.button("Delete").clicked() {
                            remove = Some(i);
                            ui.close();
                        }
                    });
                }
                if sel {
                    let l = &mut c.layers[i];
                    ui.indent(("layer", i), |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Name");
                            ui.text_edit_singleline(&mut l.name);
                        });
                        if i > 0 {
                            ui.horizontal(|ui| {
                                ui.label("Weight");
                                ui.add(egui::Slider::new(&mut l.default_weight, 0.0..=1.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Blending");
                                egui::ComboBox::from_id_salt(("blend", i))
                                    .selected_text(format!("{:?}", l.blending_mode))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut l.blending_mode,
                                            AnimatorLayerBlendingMode::Override,
                                            "Override",
                                        );
                                        ui.selectable_value(
                                            &mut l.blending_mode,
                                            AnimatorLayerBlendingMode::Additive,
                                            "Additive",
                                        );
                                    });
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.label("Mask");
                            let current = l.avatar_mask_asset.as_ref().map(|m| m.path().to_owned());
                            egui::ComboBox::from_id_salt(("mask", i))
                                .selected_text(
                                    current
                                        .as_deref()
                                        .map_or("None", |p| p.rsplit('/').next().unwrap_or(p)),
                                )
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(current.is_none(), "None").clicked() {
                                        l.avatar_mask_asset = None;
                                        l.avatar_mask = None;
                                    }
                                    let masks = ui
                                        .ctx()
                                        .data(|d| {
                                            d.get_temp::<Vec<String>>(egui::Id::new(
                                                "animator-masks",
                                            ))
                                        })
                                        .unwrap_or_default();
                                    for m in masks {
                                        if ui
                                            .selectable_label(
                                                current.as_deref() == Some(m.as_str()),
                                                m.rsplit('/').next().unwrap_or(&m),
                                            )
                                            .clicked()
                                        {
                                            l.avatar_mask_asset = Some(Handle::from_path(m));
                                        }
                                    }
                                });
                        });
                        ui.checkbox(&mut l.ik_pass, "IK Pass");
                    });
                }
            }
            if let Some(i) = remove {
                c.layers.remove(i);
                w.layer = 0;
                w.selected = None;
            }
        }
        LeftTab::Parameters => {
            ui.menu_button("+ Add Parameter", |ui| {
                for (label, kind) in [
                    ("Float", ParameterType::Float),
                    ("Int", ParameterType::Int),
                    ("Bool", ParameterType::Bool),
                    ("Trigger", ParameterType::Trigger),
                ] {
                    if ui.button(label).clicked() {
                        let names: Vec<String> =
                            c.parameters.iter().map(|p| p.name.clone()).collect();
                        let name = unique(&format!("New {label}"), &names);
                        c.parameters.push(Parameter {
                            name: name.clone(),
                            kind,
                            default: 0.0,
                        });
                        w.renaming_param = Some((c.parameters.len() - 1, name));
                        ui.close();
                    }
                }
            });
            let mut remove = None;
            for (i, p) in c.parameters.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    if let Some((ri, text)) = &mut w.renaming_param
                        && *ri == i
                    {
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(100.0));
                        if !r.has_focus() && !r.lost_focus() {
                            r.request_focus();
                        }
                        if r.lost_focus() {
                            if !text.trim().is_empty() {
                                p.name = text.trim().to_owned();
                            }
                            w.renaming_param = None;
                        }
                    } else {
                        let r = ui.add(egui::Label::new(&p.name).sense(Sense::click()));
                        if r.double_clicked() {
                            w.renaming_param = Some((i, p.name.clone()));
                        }
                        r.context_menu(|ui| {
                            if ui.button("Rename").clicked() {
                                w.renaming_param = Some((i, p.name.clone()));
                                ui.close();
                            }
                            if ui.button("Delete").clicked() {
                                remove = Some(i);
                                ui.close();
                            }
                        });
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| match p.kind {
                            ParameterType::Float => {
                                ui.add(egui::DragValue::new(&mut p.default).speed(0.01));
                            }
                            ParameterType::Int => {
                                let mut v = p.default as i64;
                                if ui.add(egui::DragValue::new(&mut v)).changed() {
                                    p.default = v as f32;
                                }
                            }
                            ParameterType::Bool | ParameterType::Trigger => {
                                let mut on = p.default != 0.0;
                                if ui.checkbox(&mut on, "").changed() {
                                    p.default = f32::from(u8::from(on));
                                }
                            }
                        },
                    );
                });
            }
            if let Some(i) = remove {
                c.parameters.remove(i);
            }
        }
    }
}

fn node_rect(origin: Pos2, pan: egui::Vec2, p: Vector2) -> Rect {
    Rect::from_min_size(origin + pan + vec2(p.x, p.y), NODE)
}

/// Point on `r`'s border toward `to`.
fn border_point(r: Rect, to: Pos2) -> Pos2 {
    let c = r.center();
    let d = to - c;
    if d.length() < 1e-3 {
        return c;
    }
    let sx = if d.x.abs() > 1e-3 {
        (r.width() / 2.0) / d.x.abs()
    } else {
        f32::INFINITY
    };
    let sy = if d.y.abs() > 1e-3 {
        (r.height() / 2.0) / d.y.abs()
    } else {
        f32::INFINITY
    };
    c + d * sx.min(sy)
}

fn arrow(painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32, width: f32) {
    painter.line_segment([a, b], Stroke::new(width, color));
    let m = a + (b - a) * 0.5;
    let dir = (b - a).normalized();
    let n = vec2(-dir.y, dir.x);
    painter.add(egui::Shape::convex_polygon(
        vec![
            m + dir * 7.0,
            m - dir * 5.0 + n * 5.0,
            m - dir * 5.0 - n * 5.0,
        ],
        color,
        Stroke::NONE,
    ));
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

fn graph(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    live: Option<&oxide_animation::AnimatorStateInfo>,
    size: egui::Vec2,
    assets: &AssetServer,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(36));
    // Grid.
    let step = 20.0;
    let off = vec2(w.pan.x.rem_euclid(step), w.pan.y.rem_euclid(step));
    let mut x = rect.left() + off.x;
    while x < rect.right() {
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(42)),
        );
        x += step;
    }
    let mut y = rect.top() + off.y;
    while y < rect.bottom() {
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.0, Color32::from_gray(42)),
        );
        y += step;
    }
    let origin = rect.min;
    let layer = w.layer;
    // Pass mask paths to the layer panel (it has no AssetServer access).
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            egui::Id::new("animator-masks"),
            assets.paths_of_type("AvatarMask"),
        )
    });
    let sm = &mut c.layers[layer].state_machine;
    let state_rects: Vec<Rect> = sm
        .states
        .iter()
        .map(|s| node_rect(origin, w.pan, s.position))
        .collect();
    let entry = Rect::from_min_size(origin + w.pan + ENTRY_POS.to_vec2(), vec2(110.0, 30.0));
    let any = Rect::from_min_size(origin + w.pan + ANY_POS.to_vec2(), vec2(110.0, 30.0));

    // Transitions (drawn first, under the nodes). Two-way pairs are offset so both show.
    let mut hit_transition = None;
    let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
    for (ti, t) in sm.transitions.iter().enumerate() {
        let Some(to) = sm.state_index(&t.to) else {
            continue;
        };
        let from_rect = match &t.from {
            None => any,
            Some(f) => match sm.state_index(f) {
                Some(i) => state_rects[i],
                None => continue,
            },
        };
        let to_rect = state_rects[to];
        let dir = (to_rect.center() - from_rect.center()).normalized();
        let shift = vec2(-dir.y, dir.x) * 5.0;
        let a = border_point(from_rect, to_rect.center()) + shift;
        let b = border_point(to_rect, from_rect.center()) + shift;
        let sel = w.selected == Some(Sel::Transition(ti));
        let color = if sel {
            Color32::from_rgb(90, 170, 255)
        } else {
            Color32::from_gray(200)
        };
        arrow(&painter, a, b, color, if sel { 2.5 } else { 1.5 });
        if let Some(p) = pointer
            && dist_to_segment(p, a, b) < 6.0
        {
            hit_transition = Some(ti);
        }
    }
    if let Some(d) = sm.states.get(sm.default_state) {
        let to = state_rects[sm.default_state];
        let _ = d;
        arrow(
            &painter,
            border_point(entry, to.center()),
            border_point(to, entry.center()),
            Color32::from_rgb(230, 150, 60),
            1.5,
        );
    }

    // Fixed nodes.
    let fixed = |r: Rect, label: &str, col: Color32| {
        painter.rect_filled(r, 4.0, col);
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(13.0),
            Color32::BLACK,
        );
    };
    fixed(entry, "Entry", Color32::from_rgb(110, 190, 110));
    fixed(any, "Any State", Color32::from_rgb(110, 190, 200));

    // States.
    let mut hit_state = None;
    for (i, s) in sm.states.iter().enumerate() {
        let r = state_rects[i];
        let default = i == sm.default_state;
        let base = if default {
            Color32::from_rgb(200, 120, 50)
        } else {
            Color32::from_gray(90)
        };
        let sel = w.selected == Some(Sel::State(i));
        painter.rect_filled(r, 4.0, base);
        painter.rect_stroke(
            r,
            4.0,
            Stroke::new(
                if sel { 2.5 } else { 1.0 },
                if sel {
                    Color32::from_rgb(90, 170, 255)
                } else {
                    Color32::from_gray(20)
                },
            ),
            StrokeKind::Outside,
        );
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            &s.name,
            FontId::proportional(13.0),
            Color32::WHITE,
        );
        // Live state: Unity's blue progress bar.
        if let Some(info) = live
            && info.name == s.name
        {
            let f = info.normalized_time.fract();
            let bar = Rect::from_min_size(
                pos2(r.left() + 4.0, r.bottom() - 7.0),
                vec2((r.width() - 8.0) * f, 4.0),
            );
            painter.rect_filled(bar, 1.0, Color32::from_rgb(70, 150, 255));
        }
        if pointer.is_some_and(|p| r.contains(p)) {
            hit_state = Some(i);
        }
    }
    let hit_any = pointer.is_some_and(|p| any.contains(p));

    // Making a transition: rubber band to the pointer.
    if let Some(from) = w.making {
        let from_rect = from.map_or(any, |i| state_rects[i]);
        if let Some(p) = pointer {
            arrow(
                &painter,
                from_rect.center(),
                p,
                Color32::from_rgb(240, 200, 90),
                1.5,
            );
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            w.making = None;
        }
    }

    // Interaction.
    if resp.clicked() {
        if let Some(from) = w.making.take() {
            if let Some(to) = hit_state {
                let to_name = sm.states[to].name.clone();
                let from_name = from.map(|i| sm.states[i].name.clone());
                sm.transitions.push(Transition {
                    from: from_name,
                    to: to_name,
                    conditions: vec![],
                    duration: 0.25,
                    // Unity's default for state-to-state transitions.
                    has_exit_time: from.is_some(),
                    exit_time: 0.75,
                });
                w.selected = Some(Sel::Transition(sm.transitions.len() - 1));
            }
        } else {
            w.selected = hit_state
                .map(Sel::State)
                .or(hit_transition.map(Sel::Transition));
        }
    }
    if resp.drag_started() {
        let at = ui
            .input(|i| i.pointer.press_origin())
            .filter(|p| rect.contains(*p));
        w.dragging = at.and_then(|p| state_rects.iter().position(|r| r.contains(p)));
        if let Some(i) = w.dragging {
            w.selected = Some(Sel::State(i));
        }
    }
    if resp.dragged() {
        let d = resp.drag_delta();
        match w.dragging {
            Some(i) => {
                if let Some(s) = sm.states.get_mut(i) {
                    s.position += Vector2::new(d.x, d.y);
                }
            }
            None => w.pan += d,
        }
    }
    if resp.drag_stopped() {
        // Snap to the grid, as Unity does.
        if let Some(i) = w.dragging.take()
            && let Some(s) = sm.states.get_mut(i)
        {
            s.position = Vector2::new(
                (s.position.x / 10.0).round() * 10.0,
                (s.position.y / 10.0).round() * 10.0,
            );
        }
    }
    if resp.secondary_clicked()
        && let Some(p) = pointer
    {
        w.menu_at = p;
        if let Some(i) = hit_state {
            w.selected = Some(Sel::State(i));
        }
    }
    if resp.hovered()
        && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
    {
        delete_selection(w, sm);
    }
    if resp.hovered() {
        crate::shortcuts::claim_keys(ui.ctx());
    }
    // Dropping clips from the Project window creates states (Unity).
    if let Some(payload) = resp.dnd_release_payload::<AssetPayload>()
        && payload.0.ends_with(".anim")
        && let Some(p) = pointer
    {
        let name = payload
            .0
            .rsplit('/')
            .next()
            .unwrap_or("State")
            .trim_end_matches(".anim")
            .to_owned();
        let names: Vec<String> = sm.states.iter().map(|s| s.name.clone()).collect();
        let name = unique(&name, &names);
        let local = p - origin - w.pan - NODE / 2.0;
        let s = sm.add_state(&name, Motion::Clip(Handle::from_path(payload.0.clone())));
        s.position = Vector2::new(local.x, local.y);
        w.selected = Some(Sel::State(sm.states.len() - 1));
    }
    let menu_at = w.menu_at;
    let menu_state = match w.selected {
        Some(Sel::State(i)) if state_rects.get(i).is_some_and(|r| r.contains(menu_at)) => Some(i),
        _ => None,
    };
    let menu_any = any.contains(menu_at);
    resp.context_menu(|ui| {
        if let Some(i) = menu_state {
            if ui.button("Make Transition").clicked() {
                w.making = Some(Some(i));
                ui.close();
            }
            if ui
                .add_enabled(
                    sm.default_state != i,
                    egui::Button::new("Set as Layer Default State"),
                )
                .clicked()
            {
                sm.default_state = i;
                ui.close();
            }
            if ui.button("Delete").clicked() {
                w.selected = Some(Sel::State(i));
                delete_selection(w, sm);
                ui.close();
            }
        } else if menu_any {
            if ui.button("Make Transition").clicked() {
                w.making = Some(None);
                ui.close();
            }
        } else {
            ui.menu_button("Create State", |ui| {
                let local = menu_at - origin - w.pan - NODE / 2.0;
                let mut add = |name: &str, motion: Motion| {
                    let names: Vec<String> = sm.states.iter().map(|s| s.name.clone()).collect();
                    let name = unique(name, &names);
                    let s = sm.add_state(&name, motion);
                    s.position = Vector2::new(local.x, local.y);
                    w.selected = Some(Sel::State(sm.states.len() - 1));
                };
                if ui.button("Empty").clicked() {
                    add("New State", Motion::Empty);
                    ui.close();
                }
                if ui.button("From New Blend Tree (1D)").clicked() {
                    add(
                        "Blend Tree",
                        Motion::BlendTree1D {
                            parameter: String::new(),
                            children: vec![],
                        },
                    );
                    ui.close();
                }
                if ui.button("From New Blend Tree (2D)").clicked() {
                    add(
                        "Blend Tree",
                        Motion::BlendTree2D {
                            parameter_x: String::new(),
                            parameter_y: String::new(),
                            kind: BlendTree2DType::SimpleDirectional,
                            children: vec![],
                        },
                    );
                    ui.close();
                }
            });
        }
    });
    if hit_state.is_some() || hit_transition.is_some() || hit_any {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

fn delete_selection(w: &mut AnimatorWindow, sm: &mut oxide_assets::AnimatorStateMachine) {
    match w.selected.take() {
        Some(Sel::State(i)) if i < sm.states.len() => {
            let name = sm.states.remove(i).name;
            sm.transitions
                .retain(|t| t.to != name && t.from.as_deref() != Some(name.as_str()));
            if sm.default_state == i {
                sm.default_state = 0;
            } else if sm.default_state > i {
                sm.default_state -= 1;
            }
            sm.default_state = sm.default_state.min(sm.states.len().saturating_sub(1));
        }
        Some(Sel::Transition(i)) if i < sm.transitions.len() => {
            sm.transitions.remove(i);
        }
        _ => {}
    }
}

fn clip_picker(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    assets: &AssetServer,
    h: &mut Handle<AnimationClip>,
) -> bool {
    let mut paths: Vec<String> = assets.paths_of_type("AnimationClip");
    paths.extend(assets.animations.iter().map(|(p, _)| p.to_owned()));
    paths.sort();
    paths.dedup();
    let name = |p: &str| {
        let h: Handle<AnimationClip> = Handle::from_path(p);
        assets
            .animations
            .get(&h)
            .map(|c| c.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| p.rsplit('/').next().unwrap_or(p).to_owned())
    };
    let mut changed = false;
    egui::ComboBox::from_id_salt(salt)
        .selected_text(name(h.path()))
        .width(crate::inspector::avail(ui).max(60.0))
        .show_ui(ui, |ui| {
            for p in paths {
                if ui
                    .selectable_label(h.path() == p, name(&p))
                    .on_hover_text(&p)
                    .clicked()
                {
                    *h = Handle::from_path(p);
                    changed = true;
                }
            }
        });
    changed
}

fn param_combo(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    params: &[Parameter],
    value: &mut String,
    filter: impl Fn(&Parameter) -> bool,
) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(if value.is_empty() {
            "(parameter)"
        } else {
            value.as_str()
        })
        .show_ui(ui, |ui| {
            for p in params.iter().filter(|p| filter(p)) {
                ui.selectable_value(value, p.name.clone(), &p.name);
            }
        });
}

fn properties(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    assets: &AssetServer,
) {
    let params = c.parameters.clone();
    let sm = &mut c.layers[w.layer].state_machine;
    match w.selected {
        Some(Sel::State(i)) if i < sm.states.len() => {
            let old_name = sm.states[i].name.clone();
            let s: &mut AnimatorState = &mut sm.states[i];
            ui.label(RichText::new("State").strong());
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Name");
                ui.text_edit_singleline(&mut s.name);
            });
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Tag");
                ui.text_edit_singleline(&mut s.tag);
            });
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Speed");
                ui.add(egui::DragValue::new(&mut s.speed).speed(0.01));
            });
            ui.checkbox(&mut s.looping, "Loop");
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Motion");
                let kind = match &s.motion {
                    Motion::Clip(_) => "Clip",
                    Motion::BlendTree1D { .. } => "Blend Tree 1D",
                    Motion::BlendTree2D { .. } => "Blend Tree 2D",
                    Motion::Empty => "None",
                };
                egui::ComboBox::from_id_salt("motion-kind")
                    .selected_text(kind)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(kind == "None", "None").clicked() {
                            s.motion = Motion::Empty;
                        }
                        if ui.selectable_label(kind == "Clip", "Clip").clicked() && kind != "Clip" {
                            s.motion = Motion::Clip(Handle::from_path(String::new()));
                        }
                        if ui
                            .selectable_label(kind == "Blend Tree 1D", "Blend Tree 1D")
                            .clicked()
                            && kind != "Blend Tree 1D"
                        {
                            s.motion = Motion::BlendTree1D {
                                parameter: String::new(),
                                children: vec![],
                            };
                        }
                        if ui
                            .selectable_label(kind == "Blend Tree 2D", "Blend Tree 2D")
                            .clicked()
                            && kind != "Blend Tree 2D"
                        {
                            s.motion = Motion::BlendTree2D {
                                parameter_x: String::new(),
                                parameter_y: String::new(),
                                kind: BlendTree2DType::SimpleDirectional,
                                children: vec![],
                            };
                        }
                    });
            });
            let floats =
                |p: &Parameter| matches!(p.kind, ParameterType::Float | ParameterType::Int);
            match &mut s.motion {
                Motion::Clip(h) => {
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, "Clip");
                        clip_picker(ui, "state-clip", assets, h);
                    });
                }
                Motion::BlendTree1D {
                    parameter,
                    children,
                } => {
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, "Parameter");
                        param_combo(ui, "bt1-param", &params, parameter, floats);
                    });
                    let mut remove = None;
                    for (k, (thr, h)) in children.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(thr).speed(0.01).max_decimals(2));
                            clip_picker(ui, ("bt1", k), assets, h);
                            if ui.small_button("×").clicked() {
                                remove = Some(k);
                            }
                        });
                    }
                    if let Some(k) = remove {
                        children.remove(k);
                    }
                    if ui.small_button("+ Add Motion").clicked() {
                        let next = children.last().map_or(0.0, |c| c.0 + 1.0);
                        children.push((next, Handle::from_path(String::new())));
                    }
                }
                Motion::BlendTree2D {
                    parameter_x,
                    parameter_y,
                    kind,
                    children,
                } => {
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, "Parameters");
                        param_combo(ui, "bt2-x", &params, parameter_x, floats);
                        param_combo(ui, "bt2-y", &params, parameter_y, floats);
                    });
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, "Blend Type");
                        egui::ComboBox::from_id_salt("bt2-kind")
                            .selected_text(format!("{kind:?}"))
                            .show_ui(ui, |ui| {
                                for k in [
                                    BlendTree2DType::SimpleDirectional,
                                    BlendTree2DType::FreeformDirectional,
                                    BlendTree2DType::FreeformCartesian,
                                ] {
                                    ui.selectable_value(kind, k, format!("{k:?}"));
                                }
                            });
                    });
                    let mut remove = None;
                    for (k, (pos, h)) in children.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut pos.x).speed(0.01).max_decimals(2));
                            ui.add(egui::DragValue::new(&mut pos.y).speed(0.01).max_decimals(2));
                            clip_picker(ui, ("bt2", k), assets, h);
                            if ui.small_button("×").clicked() {
                                remove = Some(k);
                            }
                        });
                    }
                    if let Some(k) = remove {
                        children.remove(k);
                    }
                    if ui.small_button("+ Add Motion").clicked() {
                        children.push((Vector2::ZERO, Handle::from_path(String::new())));
                    }
                }
                Motion::Empty => {}
            }
            // Renaming keeps transitions pointing at the state.
            let new_name = sm.states[i].name.clone();
            if new_name != old_name {
                for t in &mut sm.transitions {
                    if t.to == old_name {
                        t.to = new_name.clone();
                    }
                    if t.from.as_deref() == Some(old_name.as_str()) {
                        t.from = Some(new_name.clone());
                    }
                }
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Transitions").strong());
            let name = sm.states[i].name.clone();
            for (ti, t) in sm.transitions.iter().enumerate() {
                if t.from.as_deref() == Some(name.as_str())
                    && ui
                        .selectable_label(false, format!("{name} -> {}", t.to))
                        .clicked()
                {
                    w.selected = Some(Sel::Transition(ti));
                }
            }
        }
        Some(Sel::Transition(ti)) if ti < sm.transitions.len() => {
            let t = &mut sm.transitions[ti];
            ui.label(
                RichText::new(format!(
                    "{} -> {}",
                    t.from.as_deref().unwrap_or("Any State"),
                    t.to
                ))
                .strong(),
            );
            ui.checkbox(&mut t.has_exit_time, "Has Exit Time");
            ui.add_enabled_ui(t.has_exit_time, |ui| {
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Exit Time");
                    ui.add(
                        egui::DragValue::new(&mut t.exit_time)
                            .speed(0.01)
                            .range(0.0..=10.0),
                    );
                });
            });
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Transition Duration (s)");
                ui.add(
                    egui::DragValue::new(&mut t.duration)
                        .speed(0.01)
                        .range(0.0..=10.0),
                );
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Conditions").strong());
            let mut remove = None;
            for (k, cond) in t.conditions.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    param_combo(ui, ("cond-p", k), &params, &mut cond.parameter, |_| true);
                    let kind = params
                        .iter()
                        .find(|p| p.name == cond.parameter)
                        .map(|p| p.kind);
                    condition_mode(ui, k, kind, &mut cond.mode);
                    if ui.small_button("×").clicked() {
                        remove = Some(k);
                    }
                });
            }
            if let Some(k) = remove {
                t.conditions.remove(k);
            }
            if ui.small_button("+ Add Condition").clicked() {
                let first = params.first();
                t.conditions.push(Condition {
                    parameter: first.map(|p| p.name.clone()).unwrap_or_default(),
                    mode: match first.map(|p| p.kind) {
                        Some(ParameterType::Float | ParameterType::Int) => {
                            ConditionMode::Greater(0.0)
                        }
                        _ => ConditionMode::If,
                    },
                });
            }
            if t.conditions.is_empty() && !t.has_exit_time {
                ui.colored_label(
                    Color32::from_rgb(230, 190, 90),
                    "Without conditions or exit time this transition fires immediately.",
                );
            }
        }
        _ => {
            ui.weak("Select a state or transition. Right-click the graph to create states; right-click a state to make a transition. Drag clips in from the Project window.");
        }
    }
}

fn condition_mode(
    ui: &mut egui::Ui,
    k: usize,
    kind: Option<ParameterType>,
    mode: &mut ConditionMode,
) {
    let value = |m: &ConditionMode| match m {
        ConditionMode::Greater(v)
        | ConditionMode::Less(v)
        | ConditionMode::Equals(v)
        | ConditionMode::NotEqual(v) => Some(*v),
        _ => None,
    };
    let label = |m: &ConditionMode| match m {
        ConditionMode::If => "true",
        ConditionMode::IfNot => "false",
        ConditionMode::Greater(_) => "Greater",
        ConditionMode::Less(_) => "Less",
        ConditionMode::Equals(_) => "Equals",
        ConditionMode::NotEqual(_) => "NotEqual",
    };
    let v = value(mode).unwrap_or(0.0);
    let options: Vec<ConditionMode> = match kind {
        Some(ParameterType::Trigger) => vec![ConditionMode::If],
        Some(ParameterType::Bool) => vec![ConditionMode::If, ConditionMode::IfNot],
        Some(ParameterType::Float) => vec![ConditionMode::Greater(v), ConditionMode::Less(v)],
        _ => vec![
            ConditionMode::Greater(v),
            ConditionMode::Less(v),
            ConditionMode::Equals(v),
            ConditionMode::NotEqual(v),
        ],
    };
    if !options
        .iter()
        .any(|o| std::mem::discriminant(o) == std::mem::discriminant(mode))
    {
        *mode = options[0];
    }
    egui::ComboBox::from_id_salt(("cond-m", k))
        .selected_text(label(mode))
        .width(80.0)
        .show_ui(ui, |ui| {
            for o in &options {
                if ui
                    .selectable_label(
                        std::mem::discriminant(o) == std::mem::discriminant(mode),
                        label(o),
                    )
                    .clicked()
                {
                    *mode = *o;
                }
            }
        });
    if let Some(mut x) = value(mode)
        && ui.add(egui::DragValue::new(&mut x).speed(0.01)).changed()
    {
        *mode = match mode {
            ConditionMode::Greater(_) => ConditionMode::Greater(x),
            ConditionMode::Less(_) => ConditionMode::Less(x),
            ConditionMode::Equals(_) => ConditionMode::Equals(x),
            _ => ConditionMode::NotEqual(x),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_a_state_removes_its_transitions_and_fixes_default() {
        let mut c = AnimatorController::default();
        c.add_state("Idle", Motion::Empty);
        c.add_state("Run", Motion::Empty);
        c.add_state("Jump", Motion::Empty);
        c.add_transition(Some("Idle"), "Run", vec![], 0.1);
        c.add_transition(Some("Run"), "Jump", vec![], 0.1);
        c.add_transition(None, "Idle", vec![], 0.1);
        c.base().default_state = 2;
        let mut w = AnimatorWindow {
            selected: Some(Sel::State(1)),
            ..Default::default()
        };
        delete_selection(&mut w, c.base());
        let sm = c.base();
        assert_eq!(sm.states.len(), 2);
        assert_eq!(
            sm.transitions.len(),
            1,
            "transitions to and from Run are gone"
        );
        assert_eq!(sm.default_state, 1, "Jump moved down one");
    }

    #[test]
    fn border_points_sit_on_the_rect() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 40.0));
        let p = border_point(r, pos2(500.0, 20.0));
        assert!((p.x - 100.0).abs() < 1e-3 && (p.y - 20.0).abs() < 1e-3);
    }
}
