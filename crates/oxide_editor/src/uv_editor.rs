//! ProBuilder's UV Editor (Tools > ProBuilder > Editors > UV Editor): the selected object's
//! faces laid out in UV space over the 0–1 square. Click faces to select them (Shift adds),
//! drag to move their UVs, and use Unity's actions: rotate, flip, scale, Fit, Planar
//! Project and Reset to Auto UVs. Moving UVs switches faces to manual UVs.
//! <https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/uv-editor.html>

use crate::EditorState;
use crate::probuilder_window::{Mode, edit, target};
use egui::{Color32, Pos2, Stroke, Vec2};
use oxide_engine::Engine;
use oxide_math::Vector2;
use oxide_probuilder::ProBuilderMesh;

/// Pan and zoom of the canvas.
#[derive(Clone, Copy)]
pub struct UvView {
    zoom: f32,
    pan: Vec2,
    dragging: bool,
}

impl Default for UvView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
            dragging: false,
        }
    }
}

fn point_in_poly(p: Pos2, poly: &[Pos2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let Some((id, _)) = target(st, &engine.world) else {
        ui.label(
            "Select a ProBuilder object, switch to Face mode and pick faces to edit their UVs.",
        );
        return;
    };
    // A selection made for another object doesn't carry over.
    if st.probuilder.object.is_some_and(|o| o != id) {
        st.probuilder.faces.clear();
    }
    st.probuilder.object = Some(id);
    let sel = st.probuilder.faces.clone();
    // ---- Actions.
    ui.horizontal_wrapped(|ui| {
        let mut act =
            |ui: &mut egui::Ui, label: &str, f: &dyn Fn(&mut ProBuilderMesh, &[usize])| {
                if ui
                    .add_enabled(!sel.is_empty(), egui::Button::new(label))
                    .clicked()
                {
                    edit(st, engine, &format!("UV {label}"), |pb, t| f(pb, &t.faces));
                }
            };
        let center = |pb: &ProBuilderMesh, s: &[usize]| pb.mesh.uv_center(s);
        act(ui, "Rotate 90°", &|pb, s| {
            let c = center(pb, s);
            pb.mesh
                .uv_transform(s, Vector2::ZERO, 90.0, Vector2::ONE, c)
        });
        act(ui, "Rotate −90°", &|pb, s| {
            let c = center(pb, s);
            pb.mesh
                .uv_transform(s, Vector2::ZERO, -90.0, Vector2::ONE, c)
        });
        act(ui, "Flip U", &|pb, s| {
            let c = center(pb, s);
            pb.mesh
                .uv_transform(s, Vector2::ZERO, 0.0, Vector2::new(-1.0, 1.0), c)
        });
        act(ui, "Flip V", &|pb, s| {
            let c = center(pb, s);
            pb.mesh
                .uv_transform(s, Vector2::ZERO, 0.0, Vector2::new(1.0, -1.0), c)
        });
        act(ui, "Scale ×2", &|pb, s| {
            let c = center(pb, s);
            pb.mesh
                .uv_transform(s, Vector2::ZERO, 0.0, Vector2::splat(2.0), c)
        });
        act(ui, "Scale ½", &|pb, s| {
            let c = center(pb, s);
            pb.mesh
                .uv_transform(s, Vector2::ZERO, 0.0, Vector2::splat(0.5), c)
        });
        act(ui, "Fit", &|pb, s| pb.mesh.uv_fit(s));
        act(ui, "Planar Project", &|pb, s| pb.mesh.uv_planar_project(s));
        act(ui, "Reset to Auto", &|pb, s| pb.mesh.uv_reset_auto(s));
    });
    let mesh = engine
        .world
        .get_component::<ProBuilderMesh>(id)
        .expect("target")
        .mesh
        .clone();
    let manual = mesh.faces.iter().filter(|f| f.has_manual_uv()).count();
    ui.horizontal(|ui| {
        ui.weak(format!(
            "{} of {} faces selected · {manual} manual, {} auto",
            sel.len(),
            mesh.faces.len(),
            mesh.faces.len() - manual
        ));
        if ui.small_button("Select All").clicked() {
            st.probuilder.mode = Mode::Face;
            st.probuilder.faces = (0..mesh.faces.len()).collect();
        }
        if ui.small_button("Frame 0–1").clicked() {
            st.uv_view = UvView::default();
        }
    });

    // ---- Canvas.
    let (resp, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let rect = resp.rect;
    painter.rect_filled(rect, 0.0, Color32::from_gray(30));
    let view = &mut st.uv_view;
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            view.zoom = (view.zoom * (1.0 + scroll * 0.002)).clamp(0.1, 20.0);
        }
    }
    if resp.dragged_by(egui::PointerButton::Middle)
        || resp.dragged_by(egui::PointerButton::Secondary)
    {
        view.pan += resp.drag_delta();
    }
    let side = rect.width().min(rect.height()) * 0.8 * view.zoom;
    let origin = rect.center() + view.pan + egui::vec2(-side * 0.5, side * 0.5);
    let to_screen = |uv: Vector2| Pos2::new(origin.x + uv.x * side, origin.y - uv.y * side);
    let px_to_uv = |d: Vec2| Vector2::new(d.x / side, -d.y / side);
    // Grid and the 0–1 square.
    for k in -10..=20 {
        let t = k as f32 * 0.1;
        let col = if k % 10 == 0 {
            Color32::from_gray(90)
        } else {
            Color32::from_gray(45)
        };
        painter.line_segment(
            [
                to_screen(Vector2::new(t, -1.0)),
                to_screen(Vector2::new(t, 2.0)),
            ],
            Stroke::new(1.0, col),
        );
        painter.line_segment(
            [
                to_screen(Vector2::new(-1.0, t)),
                to_screen(Vector2::new(2.0, t)),
            ],
            Stroke::new(1.0, col),
        );
    }
    painter.rect_stroke(
        egui::Rect::from_two_pos(to_screen(Vector2::ZERO), to_screen(Vector2::ONE)),
        0.0,
        Stroke::new(1.5, Color32::from_gray(160)),
        egui::StrokeKind::Middle,
    );
    // Faces.
    let polys: Vec<Vec<Pos2>> = (0..mesh.faces.len())
        .map(|f| mesh.face_uvs(f).into_iter().map(to_screen).collect())
        .collect();
    for (f, poly) in polys.iter().enumerate() {
        let selected = sel.contains(&f);
        if selected && poly.len() >= 3 {
            for [a, b, c] in oxide_probuilder::triangulate(
                &poly
                    .iter()
                    .map(|p| oxide_math::Vector3::new(p.x, p.y, 0.0))
                    .collect::<Vec<_>>(),
                oxide_math::Vector3::Z,
            ) {
                painter.add(egui::Shape::convex_polygon(
                    vec![poly[a], poly[b], poly[c]],
                    Color32::from_rgba_unmultiplied(255, 170, 0, 70),
                    Stroke::NONE,
                ));
            }
        }
        let stroke = if selected {
            Stroke::new(2.0, Color32::from_rgb(255, 200, 40))
        } else {
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 200, 255, 160))
        };
        let mut closed = poly.clone();
        if let Some(first) = poly.first() {
            closed.push(*first);
        }
        painter.add(egui::Shape::line(closed, stroke));
    }
    // Click selects faces (Shift adds); dragging a selected face moves the selection's UVs.
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let hit = pointer.and_then(|p| {
        polys
            .iter()
            .rposition(|poly| poly.len() >= 3 && point_in_poly(p, poly))
    });
    if resp.clicked() {
        let shift = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        st.probuilder.mode = Mode::Face;
        match hit {
            Some(f) if shift => {
                if let Some(k) = st.probuilder.faces.iter().position(|x| *x == f) {
                    st.probuilder.faces.remove(k);
                } else {
                    st.probuilder.faces.push(f);
                }
            }
            Some(f) => st.probuilder.faces = vec![f],
            None if !shift => st.probuilder.faces.clear(),
            None => {}
        }
    }
    if resp.drag_started_by(egui::PointerButton::Primary) && hit.is_some_and(|f| sel.contains(&f)) {
        let now = oxide_core::time::realtime_since_startup();
        st.record(&engine.world, "Move UVs", None, now);
        st.uv_view.dragging = true;
    }
    if st.uv_view.dragging {
        if resp.dragged_by(egui::PointerButton::Primary) {
            let d = px_to_uv(resp.drag_delta());
            if let Some(pb) = engine.world.get_component_mut::<ProBuilderMesh>(id) {
                pb.mesh
                    .uv_transform(&sel, d, 0.0, Vector2::ONE, Vector2::ZERO);
                pb.edited();
            }
            oxide_probuilder::update(&mut engine.world);
        }
        if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            st.uv_view.dragging = false;
        }
    }
}
