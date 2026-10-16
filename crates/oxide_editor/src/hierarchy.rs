//! Hierarchy window: the scene's GameObject tree with selection, rename, drag-to-reparent,
//! search and the create/context menus.
//! Reference: <https://docs.unity3d.com/Manual/Hierarchy.html>

use crate::{Action, EditorState, menus};
use egui::{Color32, Sense, Stroke, pos2, vec2};
use oxide_engine::Engine;
use oxide_scene::{GameObjectId, World};
use std::collections::HashSet;

const ROW_H: f32 = 18.0;
const INDENT: f32 = 14.0;
/// Left column with the visibility (eye) and pickability (hand) toggles.
const GUTTER: f32 = 32.0;

/// GameObject icon: a small cube outline (hexagon with the three front edges).
fn cube_icon(p: &egui::Painter, c: egui::Pos2, col: Color32) {
    let (rx, ry) = (4.5, 5.0);
    let hex: Vec<egui::Pos2> = (0..6)
        .map(|k| {
            let a = std::f32::consts::FRAC_PI_3 * k as f32 + std::f32::consts::FRAC_PI_6;
            c + vec2(rx * a.cos(), ry * a.sin())
        })
        .collect();
    let stroke = Stroke::new(1.1, col);
    p.add(egui::Shape::closed_line(hex.clone(), stroke));
    // Front corner to the three visible outer corners (top-left, top-right, bottom).
    let front = c + vec2(0.0, 0.5);
    p.line_segment([front, hex[1]], stroke);
    p.line_segment([front, hex[3]], stroke);
    p.line_segment([front, hex[5]], stroke);
}

/// Eye icon; slashed when hidden.
fn eye_icon(p: &egui::Painter, c: egui::Pos2, off: bool, col: Color32) {
    let pts: Vec<_> = (0..=12)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / 12.0;
            c + vec2(-5.0 * a.cos(), -3.2 * a.sin())
        })
        .collect();
    let low: Vec<_> = pts.iter().map(|q| pos2(q.x, 2.0 * c.y - q.y)).collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.1, col)));
    p.add(egui::Shape::line(low, Stroke::new(1.1, col)));
    p.circle_filled(c, 1.6, col);
    if off {
        p.line_segment(
            [c + vec2(-5.0, 4.0), c + vec2(5.0, -4.0)],
            Stroke::new(1.3, col),
        );
    }
}

/// Pointer icon; slashed when picking is off.
fn pick_icon(p: &egui::Painter, c: egui::Pos2, off: bool, col: Color32) {
    let o = c + vec2(-3.0, -5.0);
    p.add(egui::Shape::convex_polygon(
        vec![
            o,
            o + vec2(0.0, 9.0),
            o + vec2(2.5, 6.8),
            o + vec2(6.5, 6.5),
        ],
        Color32::TRANSPARENT,
        Stroke::new(1.1, col),
    ));
    if off {
        p.line_segment(
            [c + vec2(-5.0, 4.0), c + vec2(5.0, -4.0)],
            Stroke::new(1.3, col),
        );
    }
}

/// Drag payload: objects being dragged in the Hierarchy.
#[derive(Clone, Debug)]
pub struct HierarchyDrag(pub Vec<GameObjectId>);

#[derive(Default)]
pub struct HierarchyState {
    pub search: String,
    expanded: HashSet<GameObjectId>,
    /// Object being renamed and the edited text.
    pub renaming: Option<(GameObjectId, String)>,
    /// Scroll this row into view next frame.
    scroll_to: Option<GameObjectId>,
}

impl HierarchyState {
    /// Expands the parents of `id` so it is visible, and scrolls to it.
    pub fn reveal(&mut self, world: &World, id: GameObjectId) {
        let mut p = world.parent(id);
        while let Some(x) = p {
            self.expanded.insert(x);
            p = world.parent(x);
        }
        self.scroll_to = Some(id);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DropAt {
    Before,
    Into,
    After,
}

struct Row {
    id: GameObjectId,
    depth: usize,
}

fn collect(world: &World, st: &HierarchyState, id: GameObjectId, depth: usize, out: &mut Vec<Row>) {
    out.push(Row { id, depth });
    if st.expanded.contains(&id) {
        for &c in world.children(id) {
            collect(world, st, c, depth + 1, out);
        }
    }
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.menu_button("+", |ui| menus::create_menu(st, ui));
        ui.add(
            egui::TextEdit::singleline(&mut st.hierarchy.search)
                .hint_text("Search")
                .desired_width(f32::INFINITY),
        );
    });
    ui.separator();

    let world = &engine.world;
    let query = st.hierarchy.search.trim().to_lowercase();
    let mut rows = Vec::new();
    let scenes = world.loaded_scenes();
    if query.is_empty() {
        // Grouped by scene, like Unity's multi-scene Hierarchy.
        for sc in &scenes {
            for r in world.scene_roots(sc.handle) {
                collect(world, &st.hierarchy, r, 0, &mut rows);
            }
        }
    } else {
        for id in world.hierarchy_order() {
            if world
                .get(id)
                .is_some_and(|o| o.name.to_lowercase().contains(&query))
            {
                rows.push(Row { id, depth: 0 });
            }
        }
    }

    let mut drop: Option<(Vec<GameObjectId>, Option<GameObjectId>, DropAt)> = None;
    let mut clicked: Option<(GameObjectId, bool, bool)> = None;
    let mut toggle: Option<GameObjectId> = None;
    let mut vis_toggle: Option<(GameObjectId, bool)> = None;
    let mut commit_rename: Option<(GameObjectId, String)> = None;
    let mut actions = Vec::new();
    let scroll_to = st.hierarchy.scroll_to.take();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Scene header rows.
            let header = |ui: &mut egui::Ui, text: String| {
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                ui.painter().rect_filled(r, 0.0, Color32::from_gray(48));
                ui.painter().text(
                    r.left_center() + vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    text,
                    egui::FontId::proportional(12.5),
                    Color32::from_gray(220),
                );
            };
            header(
                ui,
                format!("{}{}", world.scene_name, if st.dirty { "*" } else { "" }),
            );
            let mut current_scene = oxide_scene::SceneHandle::MAIN;

            for row in &rows {
                let id = row.id;
                let Some(o) = world.get(id) else { continue };
                if query.is_empty() && o.scene() != current_scene {
                    current_scene = o.scene();
                    if let Some(sc) = scenes.iter().find(|s| s.handle == current_scene) {
                        header(ui, sc.name.clone());
                    }
                }
                let (rect, resp) = ui.allocate_exact_size(
                    vec2(ui.available_width(), ROW_H),
                    Sense::click_and_drag(),
                );
                if scroll_to == Some(id) {
                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                }
                let selected = st.is_selected(id);
                let p = ui.painter();
                if selected {
                    p.rect_filled(rect, 0.0, crate::theme::SELECTION_BG);
                } else if resp.hovered() {
                    p.rect_filled(rect, 0.0, Color32::from_gray(69));
                }
                // The toggle column is a darker strip, as in Unity.
                if !selected {
                    p.rect_filled(
                        egui::Rect::from_min_size(rect.min, vec2(GUTTER, ROW_H)),
                        0.0,
                        Color32::from_gray(46),
                    );
                }
                // Visibility and pickability toggles (shown on hover or when set).
                let eye_c = pos2(rect.left() + 9.0, rect.center().y);
                let hand_c = pos2(rect.left() + 24.0, rect.center().y);
                let vis = &st.visibility;
                let hidden_self = vis.hidden.contains(&id);
                let hidden = vis.hidden_in_chain(world, id);
                let unpick_self = vis.unpickable.contains(&id);
                let unpick = vis.is_unpickable(world, id);
                let dim = Color32::from_gray(150);
                let strong = Color32::from_gray(225);
                if hidden || resp.hovered() {
                    eye_icon(
                        p,
                        eye_c,
                        hidden,
                        if hidden_self || !hidden { strong } else { dim },
                    );
                }
                if unpick || resp.hovered() {
                    pick_icon(
                        p,
                        hand_c,
                        unpick,
                        if unpick_self || !unpick { strong } else { dim },
                    );
                }
                let x0 = rect.left() + GUTTER + 4.0 + row.depth as f32 * INDENT;
                let arrow = egui::Rect::from_min_size(pos2(x0, rect.top()), vec2(12.0, ROW_H));
                let has_children = !o.children().is_empty();
                if has_children && query.is_empty() {
                    let open = st.hierarchy.expanded.contains(&id);
                    let c = arrow.center();
                    let pts = if open {
                        vec![
                            c + vec2(-4.0, -2.0),
                            c + vec2(4.0, -2.0),
                            c + vec2(0.0, 3.0),
                        ]
                    } else {
                        vec![
                            c + vec2(-2.0, -4.0),
                            c + vec2(3.0, 0.0),
                            c + vec2(-2.0, 4.0),
                        ]
                    };
                    p.add(egui::Shape::convex_polygon(
                        pts,
                        Color32::from_gray(180),
                        Stroke::NONE,
                    ));
                }
                let active = o.active_in_hierarchy();
                let prefab = o.prefab_source.is_some() || o.prefab_link.is_some();
                let text_col = match (active, prefab) {
                    (true, true) => Color32::from_rgb(140, 180, 255),
                    (true, false) => Color32::from_gray(215),
                    (false, true) => Color32::from_rgb(90, 110, 150),
                    (false, false) => Color32::from_gray(120),
                };
                cube_icon(p, pos2(x0 + 19.0, rect.center().y), text_col);
                let text_pos = pos2(x0 + 28.0, rect.center().y);

                let renaming = st
                    .hierarchy
                    .renaming
                    .as_ref()
                    .is_some_and(|(r, _)| *r == id);
                if renaming {
                    let text_rect =
                        egui::Rect::from_min_max(pos2(text_pos.x, rect.top()), rect.right_bottom());
                    let (_, text) = st.hierarchy.renaming.as_mut().expect("renaming");
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect));
                    let out = egui::TextEdit::singleline(text)
                        .desired_width(text_rect.width())
                        .show(&mut child);
                    let te = out.response;
                    if !te.has_focus() && !te.lost_focus() {
                        // The whole name starts selected, so typing replaces it (Unity).
                        te.request_focus();
                        let mut state = out.state;
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(text.chars().count()),
                            )));
                        state.store(ui.ctx(), te.id);
                    }
                    if te.lost_focus() {
                        let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if !cancel {
                            commit_rename = Some((id, text.clone()));
                        }
                        st.hierarchy.renaming = None;
                    }
                } else {
                    ui.painter().text(
                        text_pos,
                        egui::Align2::LEFT_CENTER,
                        &o.name,
                        egui::FontId::proportional(12.5),
                        text_col,
                    );
                }

                // Interaction.
                let click_x = resp.interact_pointer_pos().map(|p| p.x - rect.left());
                if resp.clicked() && click_x.is_some_and(|x| x < GUTTER) {
                    vis_toggle = Some((id, click_x.is_some_and(|x| x < GUTTER * 0.5)));
                } else if resp.clicked() {
                    let on_arrow = resp
                        .interact_pointer_pos()
                        .is_some_and(|p| arrow.contains(p));
                    if on_arrow && has_children {
                        toggle = Some(id);
                    } else {
                        let m = ui.input(|i| i.modifiers);
                        clicked = Some((id, m.command, m.shift));
                    }
                }
                if resp.double_clicked() {
                    actions.push(Action::FrameSelected);
                }
                if resp.secondary_clicked() && !selected {
                    clicked = Some((id, false, false));
                }
                resp.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        actions.push(Action::Rename);
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() {
                        actions.push(Action::Duplicate);
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        actions.push(Action::Delete);
                        ui.close();
                    }
                    ui.separator();
                    menus::create_menu(st, ui);
                });
                if resp.drag_started() {
                    let dragged = if selected {
                        st.selection.clone()
                    } else {
                        vec![id]
                    };
                    resp.dnd_set_drag_payload(HierarchyDrag(dragged));
                }
                if let Some(payload) = resp.dnd_hover_payload::<HierarchyDrag>() {
                    let y = ui
                        .input(|i| i.pointer.hover_pos())
                        .map_or(rect.center().y, |p| p.y);
                    let at = if y < rect.top() + ROW_H * 0.25 {
                        DropAt::Before
                    } else if y > rect.bottom() - ROW_H * 0.25 {
                        DropAt::After
                    } else {
                        DropAt::Into
                    };
                    let line_col = Color32::from_rgb(90, 160, 255);
                    match at {
                        DropAt::Before => ui.painter().hline(
                            rect.x_range(),
                            rect.top(),
                            Stroke::new(2.0, line_col),
                        ),
                        DropAt::After => ui.painter().hline(
                            rect.x_range(),
                            rect.bottom(),
                            Stroke::new(2.0, line_col),
                        ),
                        DropAt::Into => ui.painter().rect_stroke(
                            rect,
                            0.0,
                            Stroke::new(1.5, line_col),
                            egui::StrokeKind::Inside,
                        ),
                    };
                    if resp.dnd_release_payload::<HierarchyDrag>().is_some() {
                        drop = Some((payload.0.clone(), Some(id), at));
                    }
                }
            }

            // Empty space: drop to make root; click to deselect.
            let rest = ui.available_rect_before_wrap();
            let rest =
                egui::Rect::from_min_size(rest.min, vec2(rest.width(), rest.height().max(40.0)));
            let resp = ui.allocate_rect(rest, Sense::click());
            if let Some(payload) = resp.dnd_release_payload::<HierarchyDrag>() {
                drop = Some((payload.0.clone(), None, DropAt::Into));
            }
            if let Some(payload) = resp.dnd_release_payload::<crate::project::AssetPayload>() {
                let p = payload.0.to_ascii_lowercase();
                if p.ends_with(".prefab") {
                    actions.push(Action::InstantiatePrefab(payload.0.clone()));
                } else if p.ends_with(".glb") || p.ends_with(".gltf") {
                    actions.push(Action::InstantiateModel(payload.0.clone()));
                }
            }
            if resp.clicked() {
                st.selection.clear();
            }
            resp.context_menu(|ui| menus::create_menu(st, ui));
        });

    let now = oxide_core::time::realtime_since_startup();
    if let Some((id, eye)) = vis_toggle {
        let w = &engine.world;
        if eye {
            st.visibility.toggle_hidden(w, id);
        } else {
            st.visibility.toggle_pickable(w, id);
        }
    }
    if let Some(id) = toggle
        && !st.hierarchy.expanded.remove(&id)
    {
        st.hierarchy.expanded.insert(id);
    }
    if let Some((id, command, shift)) = clicked {
        if command {
            st.toggle_select(id);
        } else if shift {
            // Range select between the active object and this one, in row order.
            let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
            match st
                .active()
                .and_then(|a| ids.iter().position(|&x| x == a))
                .zip(ids.iter().position(|&x| x == id))
            {
                Some((a, b)) => {
                    let (lo, hi) = (a.min(b), a.max(b));
                    st.selection = ids[lo..=hi].to_vec();
                    if a > b {
                        st.selection.reverse();
                    }
                }
                None => st.select(id),
            }
        } else {
            st.select(id);
        }
        st.game_focused = false;
    }
    if let Some((id, name)) = commit_rename
        && engine.world.get(id).is_some_and(|o| o.name != name)
    {
        st.record(&engine.world, "Rename", None, now);
        if let Some(o) = engine.world.get_mut(id) {
            o.name = name;
        }
    }
    if let Some((dragged, target, at)) = drop {
        reparent(st, &mut engine.world, &dragged, target, at, now);
    }
    st.actions.extend(actions);
}

fn is_ancestor(world: &World, ancestor: GameObjectId, mut id: GameObjectId) -> bool {
    loop {
        if id == ancestor {
            return true;
        }
        match world.parent(id) {
            Some(p) => id = p,
            None => return false,
        }
    }
}

fn reparent(
    st: &mut EditorState,
    w: &mut World,
    dragged: &[GameObjectId],
    target: Option<GameObjectId>,
    at: DropAt,
    now: f64,
) {
    // Can't drop an object into itself or its own children.
    let moving: Vec<_> = dragged
        .iter()
        .copied()
        .filter(|&d| w.is_alive(d) && !target.is_some_and(|t| is_ancestor(w, d, t)))
        .collect();
    if moving.is_empty() {
        return;
    }
    st.record(w, "Reparent", None, now);
    for d in moving {
        match (target, at) {
            (None, _) => {
                let _ = w.set_parent(d, None, true);
            }
            (Some(t), DropAt::Into) => {
                let _ = w.set_parent(d, Some(t), true);
                st.hierarchy.expanded.insert(t);
            }
            (Some(t), before_after) => {
                let parent = w.parent(t);
                if w.parent(d) != parent {
                    let _ = w.set_parent(d, parent, true);
                }
                let mut index = w.sibling_index(t);
                if before_after == DropAt::After {
                    index += 1;
                }
                if w.sibling_index(d) < index {
                    index -= 1;
                }
                w.set_sibling_index(d, index);
            }
        }
    }
}
