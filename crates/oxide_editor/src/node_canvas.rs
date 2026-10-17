//! The node graph canvas Shader Graph and VFX Graph are edited on (Unity's GraphView):
//! nodes with typed ports, wires, pan and zoom, box selection, the Create Node searcher,
//! groups, sticky notes, copy/paste and framing.
//!
//! The canvas edits a [`GraphData`] through a [`CanvasModel`], which says what ports each
//! node has, how it is titled and previewed, and which nodes can be created.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/Shader-Graph-Window.html>

use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use oxide_math::Vector2;
use oxide_shadergraph::nodes::Opt;
use oxide_shadergraph::{GraphData, GraphEdge, GraphGroup, GraphNode, Slot, StickyNote, Ty};
use std::collections::HashMap;

/// A node the searcher can create.
#[derive(Clone, Debug)]
pub struct Template {
    /// Searcher path (`Math/Basic/Add`).
    pub path: String,
    pub kind: String,
    pub options: Vec<(String, String)>,
    pub preview: bool,
}

impl Template {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// What the canvas needs to know about the graph it edits.
pub trait CanvasModel {
    fn data(&self) -> &GraphData;
    fn data_mut(&mut self) -> &mut GraphData;
    /// Input and output ports (empty when the node is broken).
    fn ports(&self, node: &GraphNode) -> (Vec<Slot>, Vec<Slot>);
    fn title(&self, node: &GraphNode) -> String;
    /// Dropdowns shown on the node.
    fn options(&self, node: &GraphNode) -> Vec<Opt>;
    /// Can't be deleted or copied (master stack, contexts).
    fn fixed(&self, node: &GraphNode) -> bool;
    fn width(&self, node: &GraphNode) -> f32;
    /// Header tint.
    fn accent(&self, node: &GraphNode) -> Color32;
    /// The node's preview image and its UV rect, if it has one now.
    fn preview(&self, node: &GraphNode) -> Option<(egui::TextureId, Rect)>;
    fn has_preview(&self, node: &GraphNode) -> bool;
    fn error(&self, node: u32) -> Option<String>;
    fn templates(&self) -> Vec<Template>;
    fn can_connect(&self, from: Ty, to: Ty) -> bool;
    /// The inline value of an unconnected input (VFX Graph keeps block values on blocks).
    fn value(&self, node: &GraphNode, slot: &str) -> Option<oxide_math::Vector4> {
        node.value(slot)
    }
    fn set_value(&mut self, node: u32, slot: &str, v: oxide_math::Vector4) {
        if let Some(n) = self.data_mut().node_mut(node) {
            n.set_value(slot, v);
        }
    }
}

/// Unity's port colours by type.
pub fn port_color(ty: Ty) -> Color32 {
    match ty {
        Ty::F1 | Ty::Dyn => Color32::from_rgb(132, 228, 231),
        Ty::F2 => Color32::from_rgb(154, 239, 146),
        Ty::F3 => Color32::from_rgb(246, 255, 154),
        Ty::F4 => Color32::from_rgb(251, 203, 244),
        Ty::Bool => Color32::from_rgb(148, 129, 230),
        Ty::Tex => Color32::from_rgb(255, 139, 139),
        Ty::Flow => Color32::from_rgb(200, 200, 200),
        Ty::M2 | Ty::M3 | Ty::M4 => Color32::from_rgb(143, 196, 255),
        Ty::Gradient => Color32::from_rgb(130, 130, 130),
        Ty::Sampler => Color32::from_rgb(158, 158, 158),
    }
}

const HEADER: f32 = 26.0;
const ROW: f32 = 22.0;
const SELECTED: Color32 = Color32::from_rgb(68, 192, 255);

#[derive(Clone, Debug, Default)]
enum Drag {
    #[default]
    None,
    Pan,
    Nodes,
    Box(Pos2),
    /// A wire from an output being dragged to an input.
    Wire {
        node: u32,
        port: String,
        ty: Ty,
    },
    /// A wire from an unconnected input being dragged to an output.
    WireBack {
        node: u32,
        port: String,
        ty: Ty,
    },
    Note {
        index: usize,
        resize: bool,
    },
    Group(usize),
}

/// Where a new node from the searcher connects.
#[derive(Clone, Debug)]
enum Pending {
    FromOutput(u32, String, Ty),
    ToInput(u32, String, Ty),
}

#[derive(Clone, Debug)]
struct Searcher {
    /// Graph position of the new node.
    at: Vector2,
    screen: Pos2,
    query: String,
    pending: Option<Pending>,
    focused: bool,
}

/// What the user did this frame.
pub struct CanvasOutput {
    /// The graph changed.
    pub edited: bool,
    /// A gesture finished: record an undo step.
    pub commit: bool,
    /// A node was double-clicked (open its Sub Graph).
    pub open: Option<u32>,
    pub undo: bool,
    pub redo: bool,
    pub save: bool,
    /// The canvas response (drag-and-drop targets) and its rect.
    pub response: Option<egui::Response>,
    pub rect: Rect,
}

impl Default for CanvasOutput {
    fn default() -> Self {
        Self {
            edited: false,
            commit: false,
            open: None,
            undo: false,
            redo: false,
            save: false,
            response: None,
            rect: Rect::NOTHING,
        }
    }
}

/// The canvas's view and selection.
#[derive(Default)]
pub struct CanvasView {
    pub pan: Vec2,
    pub zoom: f32,
    pub selected: Vec<u32>,
    pub selected_edge: Option<usize>,
    pub selected_note: Option<usize>,
    pub selected_group: Option<usize>,
    drag: Drag,
    searcher: Option<Searcher>,
    clipboard: Option<GraphData>,
    /// Port centres on screen this frame: (node, input?, port) → position.
    ports: HashMap<(u32, bool, String), (Pos2, Ty)>,
    rects: HashMap<u32, Rect>,
    context_at: Pos2,
    /// Frame everything on the next draw (a graph was just opened).
    pub frame_all: bool,
}

impl CanvasView {
    pub fn new() -> Self {
        Self {
            zoom: 1.0,
            frame_all: true,
            ..Default::default()
        }
    }

    fn to_screen(&self, rect: Rect, p: Vector2) -> Pos2 {
        rect.min + (vec2(p.x, p.y) + self.pan) * self.zoom
    }

    pub fn to_graph(&self, rect: Rect, p: Pos2) -> Vector2 {
        let g = (p - rect.min) / self.zoom - self.pan;
        Vector2::new(g.x, g.y)
    }

    /// Clears the selection (another graph was opened).
    pub fn reset(&mut self) {
        *self = Self {
            clipboard: self.clipboard.take(),
            ..Self::new()
        };
    }

    /// Opens the Create Node searcher at a screen position.
    pub fn open_searcher(&mut self, rect: Rect, at: Pos2) {
        self.searcher = Some(Searcher {
            at: self.to_graph(rect, at),
            screen: at,
            query: String::new(),
            pending: None,
            focused: false,
        });
    }
}

/// A node's size in graph units.
fn node_size(m: &dyn CanvasModel, n: &GraphNode) -> Vec2 {
    let (ins, outs) = m.ports(n);
    let visible = |s: &[Slot], input: bool| {
        s.iter()
            .filter(|p| {
                !p.value_only
                    && (!n.collapsed
                        || if input {
                            m.data().input_edge(n.id, &p.id).is_some()
                        } else {
                            m.data()
                                .edges
                                .iter()
                                .any(|e| e.from == n.id && e.output == p.id)
                        })
            })
            .count()
    };
    let value_rows = ins.iter().filter(|p| p.value_only).count();
    let opts = m
        .options(n)
        .iter()
        .filter(|o| !o.choices.is_empty())
        .count();
    let rows = visible(&ins, true).max(visible(&outs, false)) + value_rows + opts;
    let w = m.width(n);
    let preview = if n.preview && m.has_preview(n) && !n.collapsed {
        w - 16.0
    } else {
        0.0
    };
    vec2(w, HEADER + rows as f32 * ROW + 8.0 + preview)
}

fn bezier(a: Pos2, b: Pos2, zoom: f32) -> [Pos2; 4] {
    let dx = ((b.x - a.x).abs() * 0.5).max(40.0 * zoom);
    [a, a + vec2(dx, 0.0), b - vec2(dx, 0.0), b]
}

fn near_bezier(p: Pos2, c: [Pos2; 4], tol: f32) -> bool {
    (0..=24).any(|k| {
        let t = k as f32 / 24.0;
        let u = 1.0 - t;
        let q = c[0].to_vec2() * (u * u * u)
            + c[1].to_vec2() * (3.0 * u * u * t)
            + c[2].to_vec2() * (3.0 * u * t * t)
            + c[3].to_vec2() * (t * t * t);
        (q.to_pos2() - p).length() < tol
    })
}

/// Group bounds in graph units (around its nodes, with a title bar).
fn group_rect(m: &dyn CanvasModel, g: &GraphGroup) -> Option<Rect> {
    let mut r: Option<Rect> = None;
    for id in &g.nodes {
        if let Some(n) = m.data().node(*id) {
            let nr = Rect::from_min_size(pos2(n.position.x, n.position.y), node_size(m, n));
            r = Some(r.map_or(nr, |r| r.union(nr)));
        }
    }
    r.map(|r| Rect::from_min_max(r.min - vec2(20.0, 40.0), r.max + vec2(20.0, 20.0)))
}

fn value_ui(ui: &mut egui::Ui, r: Rect, slot: &Slot, v: &mut [f32; 4]) -> egui::Response {
    match slot.ty {
        Ty::Bool => {
            let mut b = v[0] != 0.0;
            let resp = ui.put(r, egui::Checkbox::without_text(&mut b));
            v[0] = b as u32 as f32;
            resp
        }
        _ if slot.color => {
            let mut c = egui::Rgba::from_rgba_unmultiplied(v[0], v[1], v[2], v[3]);
            let mut srgba = egui::Color32::from(c);
            let resp = ui.put(r, |ui: &mut egui::Ui| {
                egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut srgba,
                    egui::color_picker::Alpha::OnlyBlend,
                )
            });
            if resp.changed() {
                c = egui::Rgba::from(srgba);
                // The picker edits sRGB bytes; values are stored as sRGB floats.
                *v = [
                    srgba.r() as f32 / 255.0,
                    srgba.g() as f32 / 255.0,
                    srgba.b() as f32 / 255.0,
                    c.a(),
                ];
            }
            resp
        }
        ty => {
            let n = ty.width().max(1) as usize;
            let w = r.width() / n as f32;
            let mut resp: Option<egui::Response> = None;
            for (k, x) in v.iter_mut().enumerate().take(n) {
                let cell =
                    Rect::from_min_size(r.min + vec2(w * k as f32, 0.0), vec2(w - 2.0, r.height()));
                let x_resp = ui.put(cell, egui::DragValue::new(x).speed(0.01).max_decimals(3));
                resp = Some(match resp {
                    Some(p) => p | x_resp,
                    None => x_resp,
                });
            }
            resp.expect("one field")
        }
    }
}

/// Draws and edits the graph. `id` keeps widget ids apart between windows.
pub fn show(
    ui: &mut egui::Ui,
    view: &mut CanvasView,
    m: &mut dyn CanvasModel,
    id: &str,
) -> CanvasOutput {
    let mut out = CanvasOutput::default();
    let rect = ui.available_rect_before_wrap();
    out.rect = rect;
    let resp = ui.interact(
        rect,
        egui::Id::new(("node-canvas", id)),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    if view.zoom == 0.0 {
        view.zoom = 1.0;
    }
    if view.frame_all {
        view.frame_all = false;
        frame(view, m, rect, false);
    }

    // ---- Background grid (Unity's dark graph view).
    painter.rect_filled(rect, 0.0, Color32::from_rgb(32, 32, 32));
    for (step, col) in [(20.0, 38), (100.0, 44)] {
        let s = step * view.zoom;
        if s < 6.0 {
            continue;
        }
        let off = vec2(view.pan.x * view.zoom % s, view.pan.y * view.zoom % s);
        let mut x = rect.left() + off.x;
        while x < rect.right() {
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(1.0, Color32::from_gray(col)),
            );
            x += s;
        }
        let mut y = rect.top() + off.y;
        while y < rect.bottom() {
            painter.line_segment(
                [pos2(rect.left(), y), pos2(rect.right(), y)],
                Stroke::new(1.0, Color32::from_gray(col)),
            );
            y += s;
        }
    }

    // ---- Pan and zoom (wheel zooms about the cursor; middle drag or Alt+drag pans).
    if resp.hovered() {
        let (scroll, pointer, zoom_delta) = ui.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.pointer.hover_pos(),
                i.zoom_delta(),
            )
        });
        let factor = if zoom_delta != 1.0 {
            zoom_delta
        } else if scroll != 0.0 {
            (scroll * 0.0015).exp()
        } else {
            1.0
        };
        if factor != 1.0
            && let Some(p) = pointer
        {
            let g = (p - rect.min) / view.zoom - view.pan;
            view.zoom = (view.zoom * factor).clamp(0.2, 2.0);
            view.pan = (p - rect.min) / view.zoom - g;
        }
    }
    let z = view.zoom;
    let font = |size: f32| FontId::proportional((size * z).max(6.0));

    // ---- Layout this frame.
    view.ports.clear();
    view.rects.clear();
    let nodes: Vec<GraphNode> = m.data().nodes.clone();
    for n in &nodes {
        let size = node_size(m, n);
        let r = Rect::from_min_size(view.to_screen(rect, n.position), size * z);
        view.rects.insert(n.id, r);
        let (ins, outs) = m.ports(n);
        let opts = m
            .options(n)
            .iter()
            .filter(|o| !o.choices.is_empty())
            .count();
        let value_rows = ins.iter().filter(|p| p.value_only).count();
        let mut row_in = 0;
        for p in ins.iter().filter(|p| !p.value_only) {
            let connected = m.data().input_edge(n.id, &p.id).is_some();
            if n.collapsed && !connected {
                continue;
            }
            let y = r.top()
                + (HEADER + (opts + value_rows + row_in) as f32 * ROW + ROW * 0.5 + 4.0) * z;
            view.ports
                .insert((n.id, true, p.id.clone()), (pos2(r.left(), y), p.ty));
            row_in += 1;
        }
        let mut row_out = 0;
        for p in &outs {
            let connected = m
                .data()
                .edges
                .iter()
                .any(|e| e.from == n.id && e.output == p.id);
            if n.collapsed && !connected {
                continue;
            }
            let y = r.top()
                + (HEADER + (opts + value_rows + row_out) as f32 * ROW + ROW * 0.5 + 4.0) * z;
            view.ports
                .insert((n.id, false, p.id.clone()), (pos2(r.right(), y), p.ty));
            row_out += 1;
        }
    }
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let port_at = |view: &CanvasView, p: Pos2| -> Option<(u32, bool, String, Ty)> {
        view.ports
            .iter()
            .filter(|(_, (c, _))| (*c - p).length() < 9.0 * z.max(0.6))
            .min_by(|a, b| ((a.1).0 - p).length().total_cmp(&((b.1).0 - p).length()))
            .map(|((n, i, s), (_, t))| (*n, *i, s.clone(), *t))
    };
    let node_at = |view: &CanvasView, p: Pos2| -> Option<u32> {
        // Topmost (last drawn) first.
        nodes
            .iter()
            .rev()
            .find(|n| view.rects.get(&n.id).is_some_and(|r| r.contains(p)))
            .map(|n| n.id)
    };
    let note_at = |view: &CanvasView, m: &dyn CanvasModel, p: Pos2| -> Option<(usize, bool)> {
        m.data().notes.iter().enumerate().rev().find_map(|(k, s)| {
            let r = Rect::from_min_size(
                view.to_screen(rect, s.position),
                vec2(s.size.x, s.size.y) * z,
            );
            r.contains(p).then(|| (k, (r.max - p).length() < 14.0))
        })
    };
    let group_title_at = |view: &CanvasView, m: &dyn CanvasModel, p: Pos2| -> Option<usize> {
        m.data().groups.iter().enumerate().rev().find_map(|(k, g)| {
            let gr = group_rect(m, g)?;
            let r = Rect::from_min_max(
                view.to_screen(rect, Vector2::new(gr.min.x, gr.min.y)),
                view.to_screen(rect, Vector2::new(gr.max.x, gr.min.y + 32.0)),
            );
            r.contains(p).then_some(k)
        })
    };
    let edge_at = |view: &CanvasView, m: &dyn CanvasModel, p: Pos2| -> Option<usize> {
        m.data().edges.iter().position(|e| {
            let a = view.ports.get(&(e.from, false, e.output.clone()));
            let b = view.ports.get(&(e.to, true, e.input.clone()));
            matches!((a, b), (Some(a), Some(b)) if near_bezier(p, bezier(a.0, b.0, z), 6.0))
        })
    };

    // ---- Groups and sticky notes (under the nodes).
    for (k, g) in m.data().groups.iter().enumerate() {
        let Some(gr) = group_rect(m, g) else { continue };
        let r = Rect::from_min_max(
            view.to_screen(rect, Vector2::new(gr.min.x, gr.min.y)),
            view.to_screen(rect, Vector2::new(gr.max.x, gr.max.y)),
        );
        let sel = view.selected_group == Some(k);
        painter.rect(
            r,
            6.0 * z,
            Color32::from_rgba_unmultiplied(70, 70, 70, 60),
            Stroke::new(
                if sel { 2.0 } else { 1.0 },
                if sel {
                    SELECTED
                } else {
                    Color32::from_gray(80)
                },
            ),
            egui::StrokeKind::Inside,
        );
        painter.text(
            r.min + vec2(10.0, 8.0) * z,
            egui::Align2::LEFT_TOP,
            &g.title,
            font(15.0),
            Color32::from_gray(220),
        );
    }
    for (k, s) in m.data().notes.iter().enumerate() {
        let r = Rect::from_min_size(
            view.to_screen(rect, s.position),
            vec2(s.size.x, s.size.y) * z,
        );
        let sel = view.selected_note == Some(k);
        painter.rect(
            r,
            2.0,
            Color32::from_rgb(232, 210, 120),
            Stroke::new(if sel { 2.0 } else { 0.0 }, SELECTED),
            egui::StrokeKind::Outside,
        );
        painter.text(
            r.min + vec2(8.0, 6.0) * z,
            egui::Align2::LEFT_TOP,
            &s.title,
            font(15.0),
            Color32::from_gray(30),
        );
        let galley = painter.layout(
            s.text.clone(),
            font(12.0),
            Color32::from_gray(40),
            (r.width() - 16.0 * z).max(10.0),
        );
        painter.galley(r.min + vec2(8.0, 30.0) * z, galley, Color32::from_gray(40));
        // Resize handle.
        painter.line_segment(
            [r.max - vec2(10.0, 3.0), r.max - vec2(3.0, 10.0)],
            Stroke::new(1.0, Color32::from_gray(90)),
        );
    }

    // ---- Wires.
    for (k, e) in m.data().edges.iter().enumerate() {
        let (Some(a), Some(b)) = (
            view.ports.get(&(e.from, false, e.output.clone())),
            view.ports.get(&(e.to, true, e.input.clone())),
        ) else {
            continue;
        };
        let sel = view.selected_edge == Some(k)
            || view.selected.contains(&e.from)
            || view.selected.contains(&e.to);
        let c = port_color(a.1);
        let stroke = Stroke::new(
            if sel { 3.0 } else { 2.0 } * z.max(0.5),
            if sel { SELECTED } else { c },
        );
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            bezier(a.0, b.0, z),
            false,
            Color32::TRANSPARENT,
            stroke,
        ));
    }

    // ---- Nodes.
    let show_fields = z >= 0.55;
    for n in &nodes {
        let r = view.rects[&n.id];
        if !r.intersects(rect.expand(200.0)) {
            continue;
        }
        let sel = view.selected.contains(&n.id);
        let err = m.error(n.id);
        let accent = m.accent(n);
        painter.rect_filled(r, 5.0 * z, Color32::from_rgb(46, 46, 46));
        let header = Rect::from_min_size(r.min, vec2(r.width(), HEADER * z));
        painter.rect_filled(
            header,
            egui::CornerRadius {
                nw: (5.0 * z) as u8,
                ne: (5.0 * z) as u8,
                sw: 0,
                se: 0,
            },
            Color32::from_rgb(36, 36, 36),
        );
        painter.line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(2.0 * z, accent),
        );
        painter.text(
            header.left_center() + vec2(8.0 * z, 0.0),
            egui::Align2::LEFT_CENTER,
            m.title(n),
            font(13.0),
            Color32::from_gray(225),
        );
        let outline = if err.is_some() {
            Stroke::new(2.0, Color32::from_rgb(230, 70, 60))
        } else if sel {
            Stroke::new(2.0, SELECTED)
        } else {
            Stroke::new(1.0, Color32::from_gray(20))
        };
        painter.rect_stroke(r, 5.0 * z, outline, egui::StrokeKind::Outside);
        if let Some(e) = &err {
            // Unity shows the error above the node.
            painter.text(
                header.left_top() - vec2(0.0, 4.0),
                egui::Align2::LEFT_BOTTOM,
                e.lines().next().unwrap_or(""),
                font(11.0),
                Color32::from_rgb(240, 110, 100),
            );
        }
        // Collapse arrow (hides unconnected ports, like Unity's).
        let c = header.right_center() - vec2(12.0 * z, 0.0);
        let s = 4.0 * z;
        let tri = if n.collapsed {
            vec![c + vec2(-s, -s), c + vec2(s, -s), c + vec2(0.0, s)]
        } else {
            vec![c + vec2(-s, s), c + vec2(s, s), c + vec2(0.0, -s)]
        };
        painter.add(egui::Shape::convex_polygon(
            tri,
            Color32::from_gray(150),
            Stroke::NONE,
        ));

        let (ins, outs) = m.ports(n);
        let mut y = header.bottom() + 4.0 * z;
        // Option dropdowns.
        for o in m.options(n).into_iter().filter(|o| !o.choices.is_empty()) {
            let row = Rect::from_min_size(
                pos2(r.left() + 8.0 * z, y),
                vec2(r.width() - 16.0 * z, ROW * z - 2.0),
            );
            if show_fields {
                let cur = n.option(o.name).unwrap_or(o.default).to_owned();
                let mut pick = None;
                ui.put(row, |ui: &mut egui::Ui| {
                    egui::ComboBox::from_id_salt((id, n.id, o.name))
                        .width(row.width())
                        .selected_text(format!("{}: {cur}", o.name))
                        .show_ui(ui, |ui| {
                            for c in o.choices {
                                if ui.selectable_label(*c == cur, *c).clicked() {
                                    pick = Some((*c).to_owned());
                                }
                            }
                        })
                        .response
                });
                if let Some(c) = pick
                    && let Some(node) = m.data_mut().node_mut(n.id)
                {
                    node.set_option(o.name, &c);
                    out.edited = true;
                    out.commit = true;
                }
            }
            y += ROW * z;
        }
        // Value-only fields (the Color node's colour).
        for p in ins.iter().filter(|p| p.value_only) {
            let row = Rect::from_min_size(
                pos2(r.left() + 8.0 * z, y + 1.0),
                vec2(r.width() - 16.0 * z, ROW * z - 4.0),
            );
            if show_fields {
                let mut v = m.value(n, &p.id).map_or(p.default, |v| v.to_array());
                let resp = value_ui(ui, row, p, &mut v);
                if resp.changed() {
                    m.set_value(n.id, &p.id, oxide_math::Vector4::from_array(v));
                    out.edited = true;
                }
                if resp.drag_stopped() || resp.lost_focus() || (resp.changed() && !resp.dragged()) {
                    out.commit = true;
                }
            }
            y += ROW * z;
        }
        // Ports.
        for p in ins.iter().filter(|p| !p.value_only) {
            let Some((c, ty)) = view.ports.get(&(n.id, true, p.id.clone())).cloned() else {
                continue;
            };
            let connected = m.data().input_edge(n.id, &p.id).is_some();
            let col = port_color(ty);
            if connected {
                painter.circle_filled(c, 4.5 * z, col);
            } else {
                painter.circle_stroke(c, 4.0 * z, Stroke::new(1.5, col));
            }
            painter.text(
                c + vec2(10.0 * z, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{}({})", p.label, short(ty)),
                font(11.0),
                Color32::from_gray(200),
            );
            // Unconnected inputs show their value to the left of the node (Unity).
            if !connected && show_fields && !ty.is_opaque() && !ty.is_matrix() {
                let w = match (p.color, ty) {
                    (true, _) => 40.0,
                    (_, Ty::Bool) => 20.0,
                    (_, t) => 46.0 * t.width().max(1) as f32,
                };
                let field = Rect::from_min_size(
                    c - vec2(w * z.min(1.0) + 14.0, 9.0),
                    vec2(w * z.min(1.0), 18.0),
                );
                painter.rect_filled(field.expand(3.0), 3.0, Color32::from_rgb(60, 60, 60));
                painter.line_segment(
                    [
                        field.right_center() + vec2(3.0, 0.0),
                        c - vec2(4.0 * z, 0.0),
                    ],
                    Stroke::new(1.5, col),
                );
                if let (Some(label), None) = (p.bind.label(), m.value(n, &p.id)) {
                    painter.text(
                        field.center(),
                        egui::Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(10.0),
                        Color32::from_gray(200),
                    );
                } else {
                    let mut v = m.value(n, &p.id).map_or(p.default, |v| v.to_array());
                    let resp = value_ui(ui, field, p, &mut v);
                    if resp.changed() {
                        m.set_value(n.id, &p.id, oxide_math::Vector4::from_array(v));
                        out.edited = true;
                    }
                    if resp.drag_stopped()
                        || resp.lost_focus()
                        || (resp.changed() && !resp.dragged())
                    {
                        out.commit = true;
                    }
                }
            } else if !connected && ty == Ty::Tex && show_fields {
                let field = Rect::from_min_size(c - vec2(74.0, 9.0), vec2(60.0, 18.0));
                painter.rect_filled(field.expand(3.0), 3.0, Color32::from_rgb(60, 60, 60));
                let name = n
                    .option("Texture")
                    .filter(|t| !t.is_empty())
                    .map(|t| t.rsplit('/').next().unwrap_or(t).to_owned())
                    .unwrap_or_else(|| "None".into());
                painter.text(
                    field.center(),
                    egui::Align2::CENTER_CENTER,
                    name,
                    FontId::proportional(10.0),
                    Color32::from_gray(200),
                );
            }
        }
        for p in &outs {
            let Some((c, ty)) = view.ports.get(&(n.id, false, p.id.clone())).cloned() else {
                continue;
            };
            let connected = m
                .data()
                .edges
                .iter()
                .any(|e| e.from == n.id && e.output == p.id);
            let col = port_color(ty);
            if connected {
                painter.circle_filled(c, 4.5 * z, col);
            } else {
                painter.circle_stroke(c, 4.0 * z, Stroke::new(1.5, col));
            }
            painter.text(
                c - vec2(10.0 * z, 0.0),
                egui::Align2::RIGHT_CENTER,
                format!("{}({})", p.label, short(ty)),
                font(11.0),
                Color32::from_gray(200),
            );
        }
        // Preview.
        if n.preview && m.has_preview(n) && !n.collapsed {
            let side = (m.width(n) - 16.0) * z;
            let pr = Rect::from_min_size(
                pos2(r.left() + 8.0 * z, r.bottom() - side - 4.0 * z),
                vec2(side, side),
            );
            match m.preview(n) {
                Some((t, uv)) => {
                    painter.image(t, pr, uv, Color32::WHITE);
                }
                None => {
                    painter.rect_filled(pr, 0.0, Color32::from_gray(24));
                }
            }
        }
    }

    // ---- Interaction.
    let (press, cmd, shift, alt, middle) = ui.input(|i| {
        (
            i.pointer.press_origin(),
            i.modifiers.command,
            i.modifiers.shift,
            i.modifiers.alt,
            i.pointer.middle_down(),
        )
    });
    if resp.secondary_clicked() {
        view.context_at = pointer.unwrap_or(rect.center());
        if let Some(n) = pointer.and_then(|p| node_at(view, p))
            && !view.selected.contains(&n)
        {
            view.selected = vec![n];
        }
    }
    if resp.clicked()
        && let Some(p) = pointer
    {
        let hit_node = node_at(view, p);
        // Collapse arrow.
        if let Some(n) = hit_node
            && let Some(r) = view.rects.get(&n)
            && (p - (pos2(r.right(), r.top() + HEADER * 0.5 * z) - vec2(12.0 * z, 0.0))).length()
                < 9.0 * z
            && let Some(node) = m.data_mut().node_mut(n)
        {
            node.collapsed = !node.collapsed;
            out.edited = true;
            out.commit = true;
        }
        view.selected_edge = None;
        view.selected_note = None;
        view.selected_group = None;
        match hit_node {
            Some(n) if shift || cmd => {
                if let Some(i) = view.selected.iter().position(|x| *x == n) {
                    view.selected.remove(i);
                } else {
                    view.selected.push(n);
                }
            }
            Some(n) => view.selected = vec![n],
            None => {
                if !(shift || cmd) {
                    view.selected.clear();
                }
                if let Some((k, _)) = note_at(view, m, p) {
                    view.selected_note = Some(k);
                } else if let Some(k) = group_title_at(view, m, p) {
                    view.selected_group = Some(k);
                } else {
                    view.selected_edge = edge_at(view, m, p);
                }
            }
        }
    }
    if resp.double_clicked()
        && let Some(p) = pointer
        && let Some(n) = node_at(view, p)
    {
        out.open = Some(n);
    }
    if resp.drag_started()
        && let Some(o) = press
    {
        view.drag = if middle || alt {
            Drag::Pan
        } else if let Some((n, input, port, ty)) = port_at(view, o) {
            if input {
                // Picking up a connected wire drags it from its output end.
                match m.data().input_edge(n, &port).cloned() {
                    Some(e) => {
                        m.data_mut().disconnect(n, &port);
                        out.edited = true;
                        let ty = view
                            .ports
                            .get(&(e.from, false, e.output.clone()))
                            .map_or(ty, |x| x.1);
                        Drag::Wire {
                            node: e.from,
                            port: e.output,
                            ty,
                        }
                    }
                    None => Drag::WireBack { node: n, port, ty },
                }
            } else {
                Drag::Wire { node: n, port, ty }
            }
        } else if let Some(n) = node_at(view, o) {
            if !view.selected.contains(&n) {
                if !(shift || cmd) {
                    view.selected.clear();
                }
                view.selected.push(n);
            }
            Drag::Nodes
        } else if let Some((k, resize)) = note_at(view, m, o) {
            view.selected_note = Some(k);
            Drag::Note { index: k, resize }
        } else if let Some(k) = group_title_at(view, m, o) {
            view.selected_group = Some(k);
            Drag::Group(k)
        } else {
            Drag::Box(o)
        };
    }
    if resp.dragged() {
        let d = resp.drag_delta() / z;
        let d = Vector2::new(d.x, d.y);
        match view.drag.clone() {
            Drag::Pan => view.pan += resp.drag_delta() / z,
            Drag::Nodes => {
                for id in view.selected.clone() {
                    if let Some(n) = m.data_mut().node_mut(id) {
                        n.position += d;
                        out.edited = true;
                    }
                }
            }
            Drag::Group(k) => {
                let ids = m
                    .data()
                    .groups
                    .get(k)
                    .map(|g| g.nodes.clone())
                    .unwrap_or_default();
                for id in ids {
                    if let Some(n) = m.data_mut().node_mut(id) {
                        n.position += d;
                        out.edited = true;
                    }
                }
            }
            Drag::Note { index, resize } => {
                if let Some(s) = m.data_mut().notes.get_mut(index) {
                    if resize {
                        s.size = (s.size + d).max(Vector2::new(80.0, 60.0));
                    } else {
                        s.position += d;
                    }
                    out.edited = true;
                }
            }
            _ => {}
        }
    }
    // Wire being dragged, and the selection box.
    if let Some(p) = pointer {
        match &view.drag {
            Drag::Wire { node, port, ty } => {
                if let Some(a) = view.ports.get(&(*node, false, port.clone())) {
                    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                        bezier(a.0, p, z),
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(2.0, port_color(*ty)),
                    ));
                }
            }
            Drag::WireBack { node, port, ty } => {
                if let Some(b) = view.ports.get(&(*node, true, port.clone())) {
                    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                        bezier(p, b.0, z),
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(2.0, port_color(*ty)),
                    ));
                }
            }
            Drag::Box(o) => {
                let r = Rect::from_two_pos(*o, p);
                painter.rect(
                    r,
                    0.0,
                    Color32::from_rgba_unmultiplied(68, 192, 255, 24),
                    Stroke::new(1.0, SELECTED),
                    egui::StrokeKind::Inside,
                );
            }
            _ => {}
        }
    }
    if resp.drag_stopped() {
        let drag = std::mem::take(&mut view.drag);
        let p = pointer
            .or(ui.input(|i| i.pointer.latest_pos()))
            .unwrap_or(rect.center());
        match drag {
            Drag::Wire { node, port, ty } => {
                match port_at(view, p) {
                    Some((to, true, input, to_ty)) if to != node => {
                        if m.can_connect(ty, to_ty) && !m.data().creates_cycle(node, to) {
                            m.data_mut().connect(node, &port, to, &input);
                        }
                    }
                    _ if node_at(view, p).is_none() => {
                        view.searcher = Some(Searcher {
                            at: view.to_graph(rect, p),
                            screen: p,
                            query: String::new(),
                            pending: Some(Pending::FromOutput(node, port, ty)),
                            focused: false,
                        });
                    }
                    _ => {}
                }
                out.edited = true;
                out.commit = true;
            }
            Drag::WireBack { node, port, ty } => match port_at(view, p) {
                Some((from, false, output, from_ty)) if from != node => {
                    if m.can_connect(from_ty, ty) && !m.data().creates_cycle(from, node) {
                        m.data_mut().connect(from, &output, node, &port);
                        out.edited = true;
                        out.commit = true;
                    }
                }
                _ if node_at(view, p).is_none() => {
                    view.searcher = Some(Searcher {
                        at: view.to_graph(rect, p),
                        screen: p,
                        query: String::new(),
                        pending: Some(Pending::ToInput(node, port, ty)),
                        focused: false,
                    });
                }
                _ => {}
            },
            Drag::Box(o) => {
                let r = Rect::from_two_pos(o, p);
                if !(shift || cmd) {
                    view.selected.clear();
                }
                for (id, nr) in &view.rects {
                    if nr.intersects(r) && !view.selected.contains(id) {
                        view.selected.push(*id);
                    }
                }
            }
            Drag::Nodes | Drag::Group(_) | Drag::Note { .. } => out.commit = true,
            _ => {}
        }
    }

    // ---- Context menu.
    let at = view.context_at;
    resp.context_menu(|ui| {
        if ui.button("Create Node").clicked() {
            view.open_searcher(rect, at);
            ui.close();
        }
        if ui.button("Create Sticky Note").clicked() {
            let g = view.to_graph(rect, at);
            m.data_mut().notes.push(StickyNote {
                position: g,
                ..Default::default()
            });
            out.edited = true;
            out.commit = true;
            ui.close();
        }
        ui.separator();
        let has_sel = !view.selected.is_empty();
        if ui
            .add_enabled(has_sel, egui::Button::new("Group Selection"))
            .clicked()
        {
            group_selection(view, m);
            out.edited = true;
            out.commit = true;
            ui.close();
        }
        if ui
            .add_enabled(view.selected_group.is_some(), egui::Button::new("Ungroup"))
            .clicked()
        {
            if let Some(k) = view.selected_group.take() {
                m.data_mut().groups.remove(k);
            }
            out.edited = true;
            out.commit = true;
            ui.close();
        }
        ui.separator();
        if ui.add_enabled(has_sel, egui::Button::new("Copy")).clicked() {
            copy(view, m);
            ui.close();
        }
        if ui.add_enabled(has_sel, egui::Button::new("Cut")).clicked() {
            copy(view, m);
            delete(view, m);
            out.edited = true;
            out.commit = true;
            ui.close();
        }
        if ui
            .add_enabled(view.clipboard.is_some(), egui::Button::new("Paste"))
            .clicked()
        {
            paste(view, m, rect, Some(at));
            out.edited = true;
            out.commit = true;
            ui.close();
        }
        if ui
            .add_enabled(has_sel, egui::Button::new("Duplicate"))
            .clicked()
        {
            copy(view, m);
            paste(view, m, rect, None);
            out.edited = true;
            out.commit = true;
            ui.close();
        }
        if ui.button("Delete").clicked() {
            delete(view, m);
            out.edited = true;
            out.commit = true;
            ui.close();
        }
        ui.separator();
        if ui.button("Frame All").clicked() {
            frame(view, m, rect, false);
            ui.close();
        }
    });

    // ---- Shortcuts (while the pointer is over the canvas and no text field has focus).
    let keys = resp.hovered() && !ui.ctx().egui_wants_keyboard_input();
    if resp.hovered() {
        // The graph's Delete, ⌘Z, ⌘D... act on the graph, not the scene.
        crate::shortcuts::claim_keys(ui.ctx());
    }
    if keys {
        // Each key press with the modifiers it was pressed with.
        let presses: Vec<(egui::Key, egui::Modifiers)> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        for (key, mods) in presses {
            use egui::Key as K;
            match (key, mods.command) {
                (K::Delete | K::Backspace, false) => {
                    delete(view, m);
                    out.edited = true;
                    out.commit = true;
                }
                (K::C, true) => copy(view, m),
                (K::X, true) => {
                    copy(view, m);
                    delete(view, m);
                    out.edited = true;
                    out.commit = true;
                }
                (K::V, true) => {
                    paste(view, m, rect, pointer);
                    out.edited = true;
                    out.commit = true;
                }
                (K::D, true) => {
                    copy(view, m);
                    paste(view, m, rect, None);
                    out.edited = true;
                    out.commit = true;
                }
                (K::G, true) => {
                    group_selection(view, m);
                    out.edited = true;
                    out.commit = true;
                }
                (K::Z, true) if mods.shift => out.redo = true,
                (K::Z, true) => out.undo = true,
                (K::Y, true) => out.redo = true,
                (K::S, true) => out.save = true,
                (K::F, false) => frame(view, m, rect, true),
                (K::A, false) => frame(view, m, rect, false),
                (K::Space, false) => view.open_searcher(rect, pointer.unwrap_or(rect.center())),
                _ => {}
            }
        }
    }

    searcher_ui(ui, view, m, &mut out, id);
    out.response = Some(resp);
    out
}

fn short(ty: Ty) -> &'static str {
    match ty {
        Ty::F1 => "1",
        Ty::F2 => "2",
        Ty::F3 => "3",
        Ty::F4 => "4",
        Ty::Bool => "B",
        Ty::Tex => "T2",
        Ty::Flow => "",
        Ty::Dyn => "n",
        Ty::M2 => "2x2",
        Ty::M3 => "3x3",
        Ty::M4 => "4x4",
        Ty::Gradient => "G",
        Ty::Sampler => "SS",
    }
}

fn group_selection(view: &mut CanvasView, m: &mut dyn CanvasModel) {
    if view.selected.is_empty() {
        return;
    }
    let sel = view.selected.clone();
    for g in &mut m.data_mut().groups {
        g.nodes.retain(|n| !sel.contains(n));
    }
    m.data_mut().groups.retain(|g| !g.nodes.is_empty());
    m.data_mut().groups.push(GraphGroup {
        title: "New Group".into(),
        nodes: sel,
    });
    view.selected_group = Some(m.data().groups.len() - 1);
}

fn delete(view: &mut CanvasView, m: &mut dyn CanvasModel) {
    if let Some(k) = view.selected_edge.take()
        && k < m.data().edges.len()
    {
        m.data_mut().edges.remove(k);
    }
    if let Some(k) = view.selected_note.take()
        && k < m.data().notes.len()
    {
        m.data_mut().notes.remove(k);
    }
    if let Some(k) = view.selected_group.take()
        && k < m.data().groups.len()
    {
        m.data_mut().groups.remove(k);
    }
    let ids: Vec<u32> = view
        .selected
        .iter()
        .copied()
        .filter(|id| m.data().node(*id).is_some_and(|n| !m.fixed(n)))
        .collect();
    m.data_mut().remove(&ids);
    view.selected.clear();
}

fn copy(view: &mut CanvasView, m: &dyn CanvasModel) {
    let ids: Vec<u32> = view
        .selected
        .iter()
        .copied()
        .filter(|id| m.data().node(*id).is_some_and(|n| !m.fixed(n)))
        .collect();
    if ids.is_empty() {
        return;
    }
    let data = m.data();
    view.clipboard = Some(GraphData {
        nodes: data
            .nodes
            .iter()
            .filter(|n| ids.contains(&n.id))
            .cloned()
            .collect(),
        edges: data
            .edges
            .iter()
            .filter(|e| ids.contains(&e.from) && ids.contains(&e.to))
            .cloned()
            .collect(),
        groups: Vec::new(),
        notes: Vec::new(),
    });
}

/// Pastes the clipboard (at `at`, or offset from the originals) and selects the copies.
fn paste(view: &mut CanvasView, m: &mut dyn CanvasModel, rect: Rect, at: Option<Pos2>) {
    let Some(clip) = view.clipboard.clone() else {
        return;
    };
    let min = clip
        .nodes
        .iter()
        .map(|n| n.position)
        .fold(Vector2::splat(f32::MAX), Vector2::min);
    let offset = match at {
        Some(p) => view.to_graph(rect, p) - min,
        None => Vector2::splat(30.0),
    };
    let mut map = HashMap::new();
    view.selected.clear();
    for n in &clip.nodes {
        let mut c = n.clone();
        c.position += offset;
        let id = m.data_mut().add(c);
        map.insert(n.id, id);
        view.selected.push(id);
    }
    for e in &clip.edges {
        if let (Some(a), Some(b)) = (map.get(&e.from), map.get(&e.to)) {
            m.data_mut().edges.push(GraphEdge {
                from: *a,
                output: e.output.clone(),
                to: *b,
                input: e.input.clone(),
            });
        }
    }
}

/// Fits all nodes (or the selection) in view (Unity's A and F).
fn frame(view: &mut CanvasView, m: &dyn CanvasModel, rect: Rect, selection: bool) {
    let mut b: Option<Rect> = None;
    for n in &m.data().nodes {
        if selection && !view.selected.is_empty() && !view.selected.contains(&n.id) {
            continue;
        }
        let r = Rect::from_min_size(pos2(n.position.x, n.position.y), node_size(m, n));
        b = Some(b.map_or(r, |b| b.union(r)));
    }
    let Some(b) = b else {
        view.pan = rect.size() * 0.5 / view.zoom.max(0.1);
        return;
    };
    let b = b.expand(40.0);
    view.zoom = (rect.width() / b.width())
        .min(rect.height() / b.height())
        .clamp(0.2, 1.0);
    view.pan = rect.size() * 0.5 / view.zoom - b.center().to_vec2();
}

fn compatible(m: &dyn CanvasModel, t: &Template, pending: &Pending) -> Option<String> {
    let node = GraphNode {
        kind: t.kind.clone(),
        options: t
            .options
            .iter()
            .map(|(k, v)| oxide_shadergraph::NodeOption {
                name: k.clone(),
                value: v.clone(),
            })
            .collect(),
        ..Default::default()
    };
    let (ins, outs) = m.ports(&node);
    match pending {
        Pending::FromOutput(_, _, ty) => ins
            .iter()
            .find(|p| !p.value_only && m.can_connect(*ty, p.ty))
            .map(|p| p.id.clone()),
        Pending::ToInput(_, _, ty) => outs
            .iter()
            .find(|p| m.can_connect(p.ty, *ty))
            .map(|p| p.id.clone()),
    }
}

/// The Create Node searcher: a category tree, or matches while typing.
fn searcher_ui(
    ui: &mut egui::Ui,
    view: &mut CanvasView,
    m: &mut dyn CanvasModel,
    out: &mut CanvasOutput,
    id: &str,
) {
    let Some(mut s) = view.searcher.take() else {
        return;
    };
    let mut close = false;
    let mut pick: Option<(Template, Option<String>)> = None;
    let templates = m.templates();
    let area = egui::Area::new(egui::Id::new(("node-searcher", id)))
        .fixed_pos(s.screen)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(270.0);
                ui.label(egui::RichText::new("Create Node").strong());
                let r = ui.add(egui::TextEdit::singleline(&mut s.query).hint_text("Search"));
                if !s.focused {
                    r.request_focus();
                    s.focused = true;
                }
                let q = s.query.to_ascii_lowercase();
                let words: Vec<&str> = q.split_whitespace().collect();
                let matches: Vec<(Template, Option<String>)> = templates
                    .iter()
                    .filter(|t| {
                        let hay = t.path.to_ascii_lowercase();
                        words.iter().all(|w| hay.contains(w))
                    })
                    .filter_map(|t| match &s.pending {
                        Some(p) => compatible(m, t, p).map(|port| (t.clone(), Some(port))),
                        None => Some((t.clone(), None)),
                    })
                    .collect();
                egui::ScrollArea::vertical()
                    .max_height(340.0)
                    .show(ui, |ui| {
                        if words.is_empty() && s.pending.is_none() {
                            // Category tree.
                            let mut cats: Vec<&str> = matches
                                .iter()
                                .map(|(t, _)| t.path.rsplit_once('/').map_or("", |(c, _)| c))
                                .collect();
                            cats.dedup();
                            cats.sort();
                            cats.dedup();
                            for cat in cats {
                                let title = if cat.is_empty() { "General" } else { cat };
                                egui::CollapsingHeader::new(title)
                                    .id_salt(("cat", id, cat))
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        for (t, port) in matches.iter().filter(|(t, _)| {
                                            t.path.rsplit_once('/').map_or("", |(c, _)| c) == cat
                                        }) {
                                            if ui.selectable_label(false, t.name()).clicked() {
                                                pick = Some((t.clone(), port.clone()));
                                            }
                                        }
                                    });
                            }
                        } else {
                            for (t, port) in matches.iter().take(60) {
                                let label = format!("{}   ", t.name());
                                let cat = t.path.rsplit_once('/').map_or("", |(c, _)| c);
                                if ui
                                    .selectable_label(false, egui::RichText::new(label))
                                    .on_hover_text(cat)
                                    .clicked()
                                {
                                    pick = Some((t.clone(), port.clone()));
                                }
                            }
                        }
                    });
                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Some(first) = matches.first()
                {
                    pick = Some(first.clone());
                }
            });
        });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    let clicked_outside = ui.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !area.response.rect.contains(p))
    });
    if clicked_outside && pick.is_none() {
        close = true;
    }
    if let Some((t, port)) = pick {
        let node = GraphNode {
            kind: t.kind.clone(),
            position: s.at,
            preview: t.preview,
            options: t
                .options
                .iter()
                .map(|(k, v)| oxide_shadergraph::NodeOption {
                    name: k.clone(),
                    value: v.clone(),
                })
                .collect(),
            ..Default::default()
        };
        let new = m.data_mut().add(node);
        match (&s.pending, port) {
            (Some(Pending::FromOutput(from, output, _)), Some(input)) => {
                m.data_mut().connect(*from, output, new, &input);
            }
            (Some(Pending::ToInput(to, input, _)), Some(output)) => {
                m.data_mut().connect(new, &output, *to, input);
            }
            _ => {}
        }
        view.selected = vec![new];
        out.edited = true;
        out.commit = true;
        close = true;
    }
    if !close {
        view.searcher = Some(s);
    }
}
