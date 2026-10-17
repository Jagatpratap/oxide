//! ProBuilder in the editor: the ProBuilder window (New Shape, selection modes, the actions
//! for faces, edges, vertices and objects, face materials, smoothing, colors and UVs) and
//! element editing in the Scene view (pick vertices, edges or faces of the selected
//! ProBuilder object, move them with an axis handle).
//! Reference: <https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/overview-ui.html>

use crate::EditorState;
use crate::view::Projector;
use egui::{Color32, RichText, Stroke};
use oxide_assets::{AssetServer, Handle};
use oxide_engine::Engine;
use oxide_math::{Matrix4x4, Vector3};
use oxide_probuilder::{PbMesh, ProBuilderMesh, ShapeParams, ShapeType, edge_key, triangulate};
use oxide_scene::{GameObjectId, World};
use serde_json::{Value as Json, json};

/// Unity ProBuilder's selection modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Object,
    Vertex,
    Edge,
    Face,
}

#[derive(Clone, Debug)]
struct Drag {
    axis: Vector3,
    last: egui::Pos2,
}

pub struct ProBuilderTool {
    pub mode: Mode,
    /// The object the element selection belongs to.
    pub object: Option<GameObjectId>,
    pub faces: Vec<usize>,
    pub edges: Vec<(u32, u32)>,
    pub vertices: Vec<u32>,
    pub shape: ShapeParams,
    extrude: f32,
    inset: f32,
    weld: f32,
    smoothing: u32,
    color: [f32; 4],
    material: Option<String>,
    drag: Option<Drag>,
}

impl Default for ProBuilderTool {
    fn default() -> Self {
        Self {
            mode: Mode::Object,
            object: None,
            faces: Vec::new(),
            edges: Vec::new(),
            vertices: Vec::new(),
            shape: ShapeParams::new(ShapeType::Cube),
            extrude: 0.5,
            inset: 0.25,
            weld: 0.01,
            smoothing: 1,
            color: [1.0; 4],
            material: None,
            drag: None,
        }
    }
}

impl ProBuilderTool {
    fn clear(&mut self) {
        self.faces.clear();
        self.edges.clear();
        self.vertices.clear();
    }

    /// The vertices the selection moves.
    fn selected_vertices(&self, m: &PbMesh) -> Vec<u32> {
        let mut v: Vec<u32> = match self.mode {
            Mode::Vertex => self.vertices.clone(),
            Mode::Edge => self.edges.iter().flat_map(|(a, b)| [*a, *b]).collect(),
            Mode::Face => m.face_vertices(&self.faces),
            Mode::Object => Vec::new(),
        };
        v.sort();
        v.dedup();
        v.retain(|i| (*i as usize) < m.positions.len());
        v
    }
}

/// The selected object's ProBuilder mesh and transform, if any.
fn target(st: &EditorState, w: &World) -> Option<(GameObjectId, Matrix4x4)> {
    let id = st.active()?;
    w.get_component::<ProBuilderMesh>(id)?;
    Some((id, w.local_to_world(id)))
}

/// Runs an edit on the selected object's mesh as one undo step.
fn edit(
    st: &mut EditorState,
    engine: &mut Engine,
    label: &str,
    f: impl FnOnce(&mut ProBuilderMesh, &mut ProBuilderTool),
) {
    let Some((id, _)) = target(st, &engine.world) else {
        st.status = "Select a ProBuilder object".into();
        return;
    };
    let now = oxide_core::time::realtime_since_startup();
    st.record(&engine.world, label, None, now);
    let mut tool = std::mem::take(&mut st.probuilder);
    if let Some(pb) = engine.world.get_component_mut::<ProBuilderMesh>(id) {
        f(pb, &mut tool);
        pb.edited();
    }
    st.probuilder = tool;
    oxide_probuilder::update(&mut engine.world);
    st.status = label.to_owned();
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // ---- Selection mode.
            ui.horizontal(|ui| {
                for (m, label) in [
                    (Mode::Object, "Object"),
                    (Mode::Vertex, "Vertex"),
                    (Mode::Edge, "Edge"),
                    (Mode::Face, "Face"),
                ] {
                    if ui
                        .selectable_label(st.probuilder.mode == m, label)
                        .clicked()
                    {
                        st.probuilder.mode = m;
                        st.probuilder.clear();
                    }
                }
            });
            ui.separator();
            // ---- New Shape.
            egui::CollapsingHeader::new("New Shape")
                .default_open(true)
                .show(ui, |ui| {
                    let mut shape = st.probuilder.shape.clone();
                    ui.horizontal(|ui| {
                        ui.label("Shape");
                        egui::ComboBox::from_id_salt("pb-shape")
                            .selected_text(shape.shape.name())
                            .show_ui(ui, |ui| {
                                for s in ShapeType::ALL {
                                    if ui.selectable_label(shape.shape == s, s.name()).clicked() {
                                        shape = ShapeParams::new(s);
                                    }
                                }
                            });
                    });
                    crate::inspector::reflect_grid_where(
                        ui,
                        &engine.world,
                        &mut shape,
                        "pb-new-shape",
                        &|f| f != "shape",
                    );
                    st.probuilder.shape = shape;
                    if ui.button("Create Shape").clicked() {
                        let now = oxide_core::time::realtime_since_startup();
                        st.record(&engine.world, "New Shape", None, now);
                        let id = oxide_probuilder::create_shape(
                            &mut engine.world,
                            st.probuilder.shape.clone(),
                        );
                        st.selection = vec![id];
                        st.status = format!("Created {}", st.probuilder.shape.shape.name());
                    }
                });
            ui.separator();
            let Some((id, _)) = target(st, &engine.world) else {
                ui.weak("Select a ProBuilder object to edit it (or ProBuilderize a mesh object).");
                if let Some(a) = st.active()
                    && engine
                        .world
                        .get_component::<oxide_scene::MeshFilter>(a)
                        .is_some()
                    && ui.button("ProBuilderize").clicked()
                {
                    let now = oxide_core::time::realtime_since_startup();
                    st.record(&engine.world, "ProBuilderize", None, now);
                    st.status = match oxide_probuilder::probuilderize(&mut engine.world, a) {
                        Ok(()) => "ProBuilderized".into(),
                        Err(e) => e,
                    };
                }
                return;
            };
            if st.probuilder.object != Some(id) {
                st.probuilder.object = Some(id);
                st.probuilder.clear();
            }
            let (nf, nv, ne) = {
                let pb = engine
                    .world
                    .get_component::<ProBuilderMesh>(id)
                    .expect("target");
                (
                    pb.mesh.faces.len(),
                    pb.mesh.positions.len(),
                    pb.mesh.edges().len(),
                )
            };
            let t = &st.probuilder;
            ui.weak(format!(
                "{nf} faces, {ne} edges, {nv} vertices · selected: {} faces, {} edges, {} vertices",
                t.faces.len(),
                t.edges.len(),
                t.vertices.len()
            ));
            match st.probuilder.mode {
                Mode::Object => {
                    if ui.button("Center Pivot").clicked() {
                        let mut offset = Vector3::ZERO;
                        edit(st, engine, "Center Pivot", |pb, _| {
                            offset = pb.mesh.center_pivot()
                        });
                        let p = engine.world.position(id);
                        let r = engine.world.rotation(id);
                        let s = engine
                            .world
                            .get(id)
                            .map_or(Vector3::ONE, |o| o.transform.local_scale);
                        engine.world.set_position(id, p + r * (offset * s));
                    }
                    if ui.button("Subdivide Object").clicked() {
                        edit(st, engine, "Subdivide Object", |pb, _| {
                            let all: Vec<usize> = (0..pb.mesh.faces.len()).collect();
                            pb.mesh.subdivide_faces(&all);
                        });
                    }
                    if ui.button("Triangulate Object").clicked() {
                        edit(st, engine, "Triangulate Object", |pb, _| {
                            let all: Vec<usize> = (0..pb.mesh.faces.len()).collect();
                            pb.mesh.triangulate_faces(&all);
                        });
                    }
                    if ui.button("Export OBJ").clicked() {
                        st.status = export_obj(engine, id).unwrap_or_else(|e| e);
                    }
                }
                Mode::Face => face_actions(st, engine, ui, id),
                Mode::Edge => {
                    if ui
                        .button("Bridge Edges")
                        .on_hover_text("Select two open edges")
                        .clicked()
                    {
                        edit(st, engine, "Bridge Edges", |pb, t| {
                            if let [a, b] = t.edges[..] {
                                pb.mesh.bridge_edges(a, b);
                            }
                        });
                    }
                    if ui.button("Select All").clicked() {
                        let pb = engine
                            .world
                            .get_component::<ProBuilderMesh>(id)
                            .expect("target");
                        st.probuilder.edges = pb.mesh.edges();
                    }
                }
                Mode::Vertex => {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut st.probuilder.weld)
                                .speed(0.001)
                                .prefix("distance "),
                        );
                        if ui.button("Weld Vertices").clicked() {
                            edit(st, engine, "Weld Vertices", |pb, t| {
                                pb.mesh.weld_vertices(&t.vertices, t.weld);
                                t.vertices.clear();
                            });
                        }
                    });
                    if ui.button("Collapse Vertices").clicked() {
                        edit(st, engine, "Collapse Vertices", |pb, t| {
                            pb.mesh.collapse_vertices(&t.vertices);
                            t.vertices.clear();
                        });
                    }
                    if ui.button("Select All").clicked() {
                        st.probuilder.vertices = (0..nv as u32).collect();
                    }
                }
            }
        });
}

fn face_actions(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, id: GameObjectId) {
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut st.probuilder.extrude)
                .speed(0.01)
                .prefix("distance "),
        );
        if ui.button("Extrude Faces").clicked() {
            edit(st, engine, "Extrude Faces", |pb, t| {
                pb.mesh.extrude_faces(&t.faces, t.extrude)
            });
        }
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut st.probuilder.inset)
                .speed(0.01)
                .range(0.0..=0.95)
                .prefix("amount "),
        );
        if ui.button("Inset Faces").clicked() {
            edit(st, engine, "Inset Faces", |pb, t| {
                pb.mesh.inset_faces(&t.faces, t.inset)
            });
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Subdivide").clicked() {
            edit(st, engine, "Subdivide Faces", |pb, t| {
                pb.mesh.subdivide_faces(&t.faces);
                t.faces.clear();
            });
        }
        if ui.button("Merge").clicked() {
            edit(st, engine, "Merge Faces", |pb, t| {
                if pb.mesh.merge_faces(&t.faces) {
                    t.faces = vec![pb.mesh.faces.len() - 1];
                }
            });
        }
        if ui.button("Triangulate").clicked() {
            edit(st, engine, "Triangulate Faces", |pb, t| {
                pb.mesh.triangulate_faces(&t.faces);
                t.faces.clear();
            });
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Flip Normals").clicked() {
            edit(st, engine, "Flip Face Normals", |pb, t| {
                pb.mesh.flip_faces(&t.faces)
            });
        }
        if ui.button("Delete").clicked() {
            edit(st, engine, "Delete Faces", |pb, t| {
                pb.mesh.delete_faces(&t.faces);
                t.faces.clear();
            });
        }
        if ui
            .button("Detach")
            .on_hover_text("To a new object")
            .clicked()
        {
            let mut part = None;
            edit(st, engine, "Detach Faces", |pb, t| {
                part = Some(pb.mesh.detach_faces(&t.faces));
                t.faces.clear();
            });
            if let Some(m) = part.filter(|m| !m.faces.is_empty()) {
                let (p, r) = (engine.world.position(id), engine.world.rotation(id));
                let new = engine.world.create("Detached");
                engine.world.set_position(new, p);
                engine.world.set_rotation(new, r);
                engine
                    .world
                    .add_component(new, ProBuilderMesh::from_mesh(m));
                oxide_probuilder::update(&mut engine.world);
            }
        }
    });
    if ui.button("Select All").clicked() {
        let pb = engine
            .world
            .get_component::<ProBuilderMesh>(id)
            .expect("target");
        st.probuilder.faces = (0..pb.mesh.faces.len()).collect();
    }
    ui.separator();
    ui.label(RichText::new("Material").strong());
    ui.horizontal(|ui| {
        let mut v = st.probuilder.material.clone().map_or(
            oxide_core::reflect::Value::Null,
            oxide_core::reflect::Value::Asset,
        );
        if crate::inspector::asset_field(ui, &engine.world, Some("Material"), &mut v, "pb-material")
        {
            st.probuilder.material = match v {
                oxide_core::reflect::Value::Asset(p) => Some(p),
                _ => None,
            };
        }
        if ui.button("Apply").clicked()
            && let Some(m) = st.probuilder.material.clone()
        {
            if let Err(e) = engine.world.resource_mut::<AssetServer>().ensure_loaded(&m) {
                st.status = e;
            }
            edit(st, engine, "Apply Material", |pb, t| {
                pb.set_face_material(&t.faces, Handle::from_path(m.clone()))
            });
        }
    });
    ui.horizontal(|ui| {
        ui.label("Smoothing Group");
        ui.add(egui::DragValue::new(&mut st.probuilder.smoothing).range(0..=24));
        if ui.button("Apply").clicked() {
            edit(st, engine, "Smoothing Group", |pb, t| {
                for f in &t.faces {
                    if let Some(face) = pb.mesh.faces.get_mut(*f) {
                        face.smoothing_group = t.smoothing;
                    }
                }
            });
        }
    });
    ui.horizontal(|ui| {
        ui.label("Vertex Color");
        ui.color_edit_button_rgba_unmultiplied(&mut st.probuilder.color);
        if ui.button("Apply").clicked() {
            edit(st, engine, "Face Color", |pb, t| {
                let c = t.color;
                for f in &t.faces {
                    if let Some(face) = pb.mesh.faces.get_mut(*f) {
                        face.color = oxide_math::Color::new(c[0], c[1], c[2], c[3]);
                    }
                }
            });
        }
    });
    // Auto UVs of the first selected face, applied to all selected (Unity's UV Editor).
    let first = {
        let pb = engine
            .world
            .get_component::<ProBuilderMesh>(id)
            .expect("target");
        st.probuilder
            .faces
            .first()
            .and_then(|f| pb.mesh.faces.get(*f))
            .map(|f| f.uv.clone())
    };
    if let Some(mut uv) = first {
        ui.separator();
        ui.label(RichText::new("UV (Auto)").strong());
        if crate::inspector::reflect_grid(ui, &engine.world, &mut uv, "pb-uv") {
            edit(st, engine, "Face UVs", |pb, t| {
                for f in &t.faces {
                    if let Some(face) = pb.mesh.faces.get_mut(*f) {
                        face.uv = uv.clone();
                    }
                }
            });
        }
    }
}

fn export_obj(engine: &Engine, id: GameObjectId) -> Result<String, String> {
    let pb = engine
        .world
        .get_component::<ProBuilderMesh>(id)
        .ok_or("no ProBuilder mesh")?;
    let name = engine
        .world
        .get(id)
        .map_or("Mesh", |o| o.name.as_str())
        .to_owned();
    let path = format!("Assets/{name}.obj");
    let full = engine.world.resource::<AssetServer>().full_path(&path);
    std::fs::write(&full, pb.mesh.to_obj(&name)).map_err(|e| e.to_string())?;
    Ok(format!("Exported {path}"))
}

fn dist_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

/// Ray-polygon hit distance (world space).
fn ray_poly(ray: &oxide_math::Ray, poly: &[Vector3], n: Vector3) -> Option<f32> {
    for [a, b, c] in triangulate(poly, n) {
        let (p0, p1, p2) = (poly[a], poly[b], poly[c]);
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let h = ray.direction.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-8 {
            continue;
        }
        let inv = 1.0 / det;
        let s = ray.origin - p0;
        let u = s.dot(h) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = ray.direction.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let t = e2.dot(q) * inv;
        if t > 0.0 {
            return Some(t);
        }
    }
    None
}

/// Element editing in the Scene view. Returns true when it handled the input (the normal
/// object selection and handles are skipped).
pub fn scene_edit(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &egui::Ui,
    resp: &egui::Response,
    proj: &Projector,
    painter: &egui::Painter,
) -> bool {
    if st.probuilder.mode == Mode::Object {
        return false;
    }
    let Some((id, model)) = target(st, &engine.world) else {
        return false;
    };
    if st.probuilder.object != Some(id) {
        st.probuilder.object = Some(id);
        st.probuilder.clear();
    }
    let mesh = engine
        .world
        .get_component::<ProBuilderMesh>(id)
        .expect("target")
        .mesh
        .clone();
    let world: Vec<Vector3> = mesh
        .positions
        .iter()
        .map(|p| model.transform_point3(*p))
        .collect();
    let screen: Vec<Option<egui::Pos2>> = world.iter().map(|p| proj.project(*p)).collect();
    let tool = &st.probuilder;
    let accent = Color32::from_rgb(255, 200, 40);
    // Selected faces.
    for &f in &tool.faces {
        let Some(face) = mesh.faces.get(f) else {
            continue;
        };
        let poly: Vec<Vector3> = face.indices.iter().map(|i| world[*i as usize]).collect();
        let n = (model.transform_vector3(mesh.face_normal(f))).normalize_or(Vector3::Y);
        for [a, b, c] in triangulate(&poly, n) {
            if let (Some(pa), Some(pb), Some(pc)) = (
                proj.project(poly[a]),
                proj.project(poly[b]),
                proj.project(poly[c]),
            ) {
                painter.add(egui::Shape::convex_polygon(
                    vec![pa, pb, pc],
                    Color32::from_rgba_unmultiplied(255, 170, 0, 120),
                    Stroke::NONE,
                ));
            }
        }
    }
    // Wireframe.
    for (a, b) in mesh.edges() {
        if let Some(seg) = proj.segment(world[a as usize], world[b as usize]) {
            let sel = tool.mode == Mode::Edge && tool.edges.contains(&(a, b));
            painter.line_segment(
                seg,
                Stroke::new(
                    if sel { 3.0 } else { 1.0 },
                    if sel {
                        accent
                    } else {
                        Color32::from_rgba_unmultiplied(120, 200, 255, 200)
                    },
                ),
            );
        }
    }
    if tool.mode == Mode::Vertex {
        for (k, p) in screen.iter().enumerate() {
            if let Some(p) = p {
                let sel = tool.vertices.contains(&(k as u32));
                painter.circle_filled(
                    *p,
                    if sel { 5.0 } else { 3.0 },
                    if sel {
                        accent
                    } else {
                        Color32::from_rgb(120, 200, 255)
                    },
                );
            }
        }
    }
    // The move handle at the selection's center.
    let verts = tool.selected_vertices(&mesh);
    let mut over_handle = None;
    let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
    if !verts.is_empty() {
        let c = verts.iter().map(|v| world[*v as usize]).sum::<Vector3>() / verts.len() as f32;
        if let Some(cs) = proj.project(c) {
            let len = proj.world_per_pixels(c, 70.0);
            for (axis, col) in [
                (Vector3::X, Color32::from_rgb(230, 60, 60)),
                (Vector3::Y, Color32::from_rgb(90, 220, 90)),
                (Vector3::Z, Color32::from_rgb(70, 130, 240)),
            ] {
                if let Some(tip) = proj.project(c + axis * len) {
                    let hot = pointer.is_some_and(|p| dist_to_segment(p, cs, tip) < 7.0);
                    if hot {
                        over_handle = Some(axis);
                    }
                    let active = st.probuilder.drag.as_ref().is_some_and(|d| d.axis == axis);
                    painter.line_segment(
                        [cs, tip],
                        Stroke::new(
                            if hot || active { 4.0 } else { 2.5 },
                            if hot || active { accent } else { col },
                        ),
                    );
                    painter.circle_filled(tip, 5.0, col);
                }
            }
            painter.circle_filled(cs, 4.0, Color32::WHITE);
        }
    }
    // Dragging the handle moves the elements along the axis.
    if resp.drag_started()
        && let Some(axis) = over_handle
        && let Some(p) = pointer
    {
        let now = oxide_core::time::realtime_since_startup();
        st.record(&engine.world, "Move Elements", None, now);
        st.probuilder.drag = Some(Drag { axis, last: p });
    }
    if let Some(d) = st.probuilder.drag.clone() {
        if resp.dragged()
            && let Some(p) = pointer
            && !verts.is_empty()
        {
            let c = verts.iter().map(|v| world[*v as usize]).sum::<Vector3>() / verts.len() as f32;
            let (Some(a), Some(b)) = (proj.project(c), proj.project(c + d.axis)) else {
                return true;
            };
            let dir = (b - a).normalized();
            let px = (p - d.last).dot(dir);
            let amount = proj.world_per_pixels(c, px);
            let local = model.inverse().transform_vector3(d.axis * amount);
            if let Some(pb) = engine.world.get_component_mut::<ProBuilderMesh>(id) {
                pb.mesh.translate(&verts, local);
                pb.edited();
            }
            oxide_probuilder::update(&mut engine.world);
            st.probuilder.drag = Some(Drag { last: p, ..d });
        }
        if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            st.probuilder.drag = None;
        }
        return true;
    }
    // Click to pick elements (Shift toggles).
    if resp.clicked()
        && over_handle.is_none()
        && let Some(p) = pointer
    {
        let shift = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        let tool = &mut st.probuilder;
        match tool.mode {
            Mode::Vertex => {
                let hit = screen
                    .iter()
                    .enumerate()
                    .filter_map(|(k, s)| s.map(|s| (k as u32, s.distance(p))))
                    .filter(|(_, d)| *d < 10.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(k, _)| k);
                toggle(&mut tool.vertices, hit, shift);
            }
            Mode::Edge => {
                let hit = mesh
                    .edges()
                    .into_iter()
                    .filter_map(|(a, b)| {
                        let seg = proj.segment(world[a as usize], world[b as usize])?;
                        Some(((a, b), dist_to_segment(p, seg[0], seg[1])))
                    })
                    .filter(|(_, d)| *d < 8.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(e, _)| edge_key(e.0, e.1));
                toggle(&mut tool.edges, hit, shift);
            }
            Mode::Face => {
                let ray = proj.ray(p);
                let hit = (0..mesh.faces.len())
                    .filter_map(|f| {
                        let poly: Vec<Vector3> = mesh.faces[f]
                            .indices
                            .iter()
                            .map(|i| world[*i as usize])
                            .collect();
                        let n = model
                            .transform_vector3(mesh.face_normal(f))
                            .normalize_or(Vector3::Y);
                        ray_poly(&ray, &poly, n).map(|t| (f, t))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(f, _)| f);
                toggle(&mut tool.faces, hit, shift);
            }
            Mode::Object => {}
        }
        return true;
    }
    // Element modes replace the object handles and picking (camera navigation stays).
    true
}

fn toggle<T: PartialEq + Copy>(list: &mut Vec<T>, hit: Option<T>, add: bool) {
    match hit {
        Some(h) if add => {
            if let Some(i) = list.iter().position(|x| *x == h) {
                list.remove(i);
            } else {
                list.push(h);
            }
        }
        Some(h) => *list = vec![h],
        None if !add => list.clear(),
        None => {}
    }
}

// ---- MCP: `probuilder`.

pub fn mcp(
    st: &mut EditorState,
    engine: &mut Engine,
    args: &serde_json::Map<String, Json>,
) -> Result<Json, String> {
    let now = oxide_core::time::realtime_since_startup();
    if let Some(c) = args.get("create").and_then(Json::as_object) {
        let kind = c.get("shape").and_then(Json::as_str).unwrap_or("Cube");
        let shape = ShapeType::ALL
            .into_iter()
            .find(|s| s.name().eq_ignore_ascii_case(kind))
            .ok_or_else(|| format!("unknown shape `{kind}`"))?;
        let mut p = ShapeParams::new(shape);
        if let Some(s) = c.get("size").and_then(Json::as_array) {
            p.size = Vector3::new(
                s.first().and_then(Json::as_f64).unwrap_or(1.0) as f32,
                s.get(1).and_then(Json::as_f64).unwrap_or(1.0) as f32,
                s.get(2).and_then(Json::as_f64).unwrap_or(1.0) as f32,
            );
        }
        for (k, v) in c {
            match k.as_str() {
                "sides" => p.sides = v.as_u64().unwrap_or(16) as u32,
                "segments" | "steps" => p.segments = v.as_u64().unwrap_or(1) as u32,
                "thickness" => p.thickness = v.as_f64().unwrap_or(0.25) as f32,
                "angle" => p.angle = v.as_f64().unwrap_or(180.0) as f32,
                "subdivisions" => p.subdivisions = v.as_u64().unwrap_or(2) as u32,
                "smooth" => p.smooth = v.as_bool().unwrap_or(true),
                _ => {}
            }
        }
        st.record(&engine.world, "New Shape", None, now);
        let id = oxide_probuilder::create_shape(&mut engine.world, p);
        if let Some(pos) = c.get("position").and_then(Json::as_array) {
            engine.world.set_position(
                id,
                Vector3::new(
                    pos.first().and_then(Json::as_f64).unwrap_or(0.0) as f32,
                    pos.get(1).and_then(Json::as_f64).unwrap_or(0.0) as f32,
                    pos.get(2).and_then(Json::as_f64).unwrap_or(0.0) as f32,
                ),
            );
        }
        st.selection = vec![id];
    }
    if let Some(o) = args.get("object") {
        let id =
            oxide_engine::json_values::resolve_object(&engine.world, o).ok_or("no such object")?;
        st.selection = vec![id];
    }
    if args.get("probuilderize").and_then(Json::as_bool) == Some(true) {
        let id = st.active().ok_or("select an object")?;
        st.record(&engine.world, "ProBuilderize", None, now);
        oxide_probuilder::probuilderize(&mut engine.world, id)?;
    }
    let faces: Vec<usize> = args
        .get("faces")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Json::as_u64)
                .map(|x| x as usize)
                .collect()
        })
        .unwrap_or_default();
    let vertices: Vec<u32> = args
        .get("vertices")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Json::as_u64)
                .map(|x| x as u32)
                .collect()
        })
        .unwrap_or_default();
    if let Some(op) = args.get("operation").and_then(Json::as_str) {
        let amount = args.get("amount").and_then(Json::as_f64).map(|v| v as f32);
        let (id, _) = target(st, &engine.world).ok_or("select a ProBuilder object")?;
        st.record(&engine.world, &format!("ProBuilder {op}"), None, now);
        let pb = engine
            .world
            .get_component_mut::<ProBuilderMesh>(id)
            .expect("target");
        let m = &mut pb.mesh;
        let all: Vec<usize> = (0..m.faces.len()).collect();
        let faces = if faces.is_empty() { all } else { faces.clone() };
        match op {
            "extrude" => m.extrude_faces(&faces, amount.unwrap_or(0.5)),
            "inset" => m.inset_faces(&faces, amount.unwrap_or(0.25)),
            "subdivide" => m.subdivide_faces(&faces),
            "merge" => {
                if !m.merge_faces(&faces) {
                    return Err("the faces don't form one connected region".into());
                }
            }
            "delete" => m.delete_faces(&faces),
            "flip" => m.flip_faces(&faces),
            "triangulate" => m.triangulate_faces(&faces),
            "weld" => m.weld_vertices(&vertices, amount.unwrap_or(0.01)),
            "collapse" => m.collapse_vertices(&vertices),
            "translate" => {
                let d = args
                    .get("delta")
                    .and_then(Json::as_array)
                    .ok_or("translate needs `delta` [x,y,z]")?;
                let d = Vector3::new(
                    d.first().and_then(Json::as_f64).unwrap_or(0.0) as f32,
                    d.get(1).and_then(Json::as_f64).unwrap_or(0.0) as f32,
                    d.get(2).and_then(Json::as_f64).unwrap_or(0.0) as f32,
                );
                let v = if vertices.is_empty() {
                    m.face_vertices(&faces)
                } else {
                    vertices.clone()
                };
                m.translate(&v, d);
            }
            "smoothing" => {
                for f in &faces {
                    if let Some(face) = m.faces.get_mut(*f) {
                        face.smoothing_group = amount.unwrap_or(1.0) as u32;
                    }
                }
            }
            "material" => {
                let mat = args
                    .get("material")
                    .and_then(Json::as_str)
                    .ok_or("material needs `material`")?
                    .to_owned();
                pb.set_face_material(&faces, Handle::from_path(mat.clone()));
                engine
                    .world
                    .resource_mut::<AssetServer>()
                    .ensure_loaded(&mat)?;
            }
            other => return Err(format!("unknown operation `{other}`")),
        }
        if let Some(pb) = engine.world.get_component_mut::<ProBuilderMesh>(id) {
            pb.edited();
        }
        oxide_probuilder::update(&mut engine.world);
    }
    if let Some(m) = args.get("mode").and_then(Json::as_str) {
        st.probuilder.mode = match m.to_ascii_lowercase().as_str() {
            "vertex" => Mode::Vertex,
            "edge" => Mode::Edge,
            "face" => Mode::Face,
            _ => Mode::Object,
        };
        st.probuilder.faces = faces.clone();
        st.probuilder.vertices = vertices.clone();
    }
    let Some((id, _)) = target(st, &engine.world) else {
        return Ok(json!({ "selected": null }));
    };
    let pb = engine
        .world
        .get_component::<ProBuilderMesh>(id)
        .expect("target");
    Ok(json!({
        "object": id.to_bits(),
        "name": engine.world.get(id).map(|o| o.name.clone()),
        "shape": pb.shape.as_ref().map(|s| s.shape.name()),
        "faces": pb.mesh.faces.iter().enumerate().map(|(k, f)| json!({
            "id": k, "vertices": f.indices, "normal": pb.mesh.face_normal(k).to_array(),
            "center": pb.mesh.face_center(k).to_array(), "material": f.material, "smoothing_group": f.smoothing_group,
        })).collect::<Vec<_>>(),
        "vertex_count": pb.mesh.positions.len(),
        "edge_count": pb.mesh.edges().len(),
        "materials": pb.materials.iter().map(|m| m.path().to_owned()).collect::<Vec<_>>(),
        "mode": format!("{:?}", st.probuilder.mode),
    }))
}

/// Whether a ProBuilder object is selected (the ProBuilder overlay shows then).
pub fn has_target(st: &EditorState, world: &World) -> bool {
    target(st, world).is_some()
}

/// The ProBuilder overlay's edit mode buttons.
pub fn mode_buttons(st: &mut EditorState, ui: &mut egui::Ui) {
    for (m, label) in [
        (Mode::Object, "Object"),
        (Mode::Vertex, "Vertex"),
        (Mode::Edge, "Edge"),
        (Mode::Face, "Face"),
    ] {
        if ui
            .selectable_label(st.probuilder.mode == m, label)
            .clicked()
        {
            st.probuilder.mode = m;
            st.probuilder.clear();
        }
    }
}
