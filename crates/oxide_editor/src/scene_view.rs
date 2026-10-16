//! Scene view: editor camera navigation, grid, click picking, gizmo icons, selection
//! outlines and the Move/Rotate/Scale handles.
//!
//! References: <https://docs.unity3d.com/Manual/SceneViewNavigation.html>,
//! <https://docs.unity3d.com/Manual/PositioningGameObjects.html>,
//! <https://docs.unity3d.com/Manual/ScenePicking.html>

use crate::project::AssetPayload;
use crate::view::{Projector, VIEW_FORMAT};
use crate::{EditorState, Tool};
use egui::{Color32, Pos2, Sense, Stroke, Vec2, pos2, vec2};
use oxide_assets::{AssetServer, Handle, Mesh};
use oxide_engine::{Engine, extract};
use oxide_math::{Color, Matrix4x4, Quaternion, QuaternionExt, Ray, Vector3};
use oxide_scene::{Camera, GameObjectId, Light, LightType, MeshFilter, SkinnedMeshRenderer, World};

pub const AXIS_COLORS: [Color32; 3] = [
    Color32::from_rgb(219, 62, 29),
    Color32::from_rgb(154, 243, 72),
    Color32::from_rgb(58, 122, 248),
];
const HOVER: Color32 = Color32::from_rgb(255, 255, 0);
const SELECTION: Color32 = Color32::from_rgb(255, 140, 0);
const COLLIDER: Color32 = Color32::from_rgb(145, 244, 139);
/// Handle length in pixels.
const HANDLE_PX: f32 = 90.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Nav {
    #[default]
    None,
    Fly,
    Orbit,
    Pan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle3 {
    Axis(usize),
    Plane(usize),
    Ring(usize),
    Uniform,
    /// Rect tool: an edge or corner (`(su, sv)` signs, one may be 0), or `(0, 0)` = inside.
    Rect(i8, i8),
}

struct Drag {
    handle: Handle3,
    tool: Tool,
    start_mouse: Pos2,
    center: Vector3,
    axes: [Vector3; 3],
    size: f32,
    /// Axis parameter (move/scale) or plane hit (move) at the start.
    start_s: f32,
    start_hit: Vector3,
    /// Rotate: screen direction of the ring tangent at the grab point, and ring radius in px.
    tangent: Vec2,
    radius_px: f32,
    /// (object, world position, world rotation, local scale) at the start.
    objects: Vec<(GameObjectId, Vector3, Quaternion, Vector3)>,
    /// Current scale factor (drawn on the scale handle).
    factor: f32,
    /// Rect tool: the rect at the start of the drag.
    rect: Option<RectGeom>,
}

/// Scene view effect toggles (Unity's Effects dropdown).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneEffects {
    pub enabled: bool,
    pub skybox: bool,
    pub fog: bool,
    pub particles: bool,
    pub post_processing: bool,
}

impl Default for SceneEffects {
    fn default() -> Self {
        Self {
            enabled: true,
            skybox: true,
            fog: true,
            particles: true,
            post_processing: true,
        }
    }
}

impl SceneEffects {
    fn on(&self, flag: bool) -> bool {
        self.enabled && flag
    }
}

pub struct SceneViewState {
    /// Draw mode (Shaded, Wireframe, Albedo, ...): Scene view only.
    pub draw_mode: oxide_render::DebugView,
    /// Scene lighting; off lights the view with a light from the camera (Unity's bulb toggle).
    pub lighting: bool,
    pub effects: SceneEffects,
    /// 2D mode: orthographic, looking down +Z, no rotation.
    pub mode_2d: bool,
    /// The 3D camera to restore when 2D mode is turned off.
    saved_3d: Option<crate::SceneCamera>,
    pub grid: bool,
    pub gizmos: bool,
    pub frame_request: bool,
    /// Right-mouse fly mode is active (WASD moves the camera, so tool keys are off).
    pub flying: bool,
    nav: Nav,
    drag: Option<Drag>,
}

impl Default for SceneViewState {
    fn default() -> Self {
        Self {
            draw_mode: oxide_render::DebugView::Shaded,
            lighting: true,
            effects: SceneEffects::default(),
            mode_2d: false,
            saved_3d: None,
            grid: true,
            gizmos: true,
            frame_request: false,
            flying: false,
            nav: Nav::None,
            drag: None,
        }
    }
}

/// Turns 2D mode on (front orthographic view) or off (back to the previous 3D view).
pub fn set_2d(st: &mut EditorState, on: bool) {
    if st.scene_ui.mode_2d == on {
        return;
    }
    st.scene_ui.mode_2d = on;
    let cam = &mut st.scene_cam;
    if on {
        st.scene_ui.saved_3d = Some(cam.clone());
        cam.yaw = 0.0;
        cam.pitch = 0.0;
        cam.orthographic = true;
    } else if let Some(prev) = st.scene_ui.saved_3d.take() {
        cam.yaw = prev.yaw;
        cam.pitch = prev.pitch;
        cam.orthographic = prev.orthographic;
    } else {
        cam.orthographic = false;
    }
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    overlay_bar(st, ui);
    let rect = ui.available_rect_before_wrap();
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let resp = ui.allocate_rect(rect, Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();
    let tex = st.scene_view.texture_id(
        engine,
        (rect.width() * ppp).round() as u32,
        (rect.height() * ppp).round() as u32,
    );
    st.scene_view.rect = Some(rect);
    let painter = ui.painter_at(rect);
    painter.image(
        tex,
        rect,
        egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    if resp.clicked() || resp.drag_started() {
        st.game_focused = false;
    }

    if std::mem::take(&mut st.scene_ui.frame_request) {
        frame_selection(st, &engine.world);
    }
    navigate(st, ui, &resp);

    let proj = Projector::new(&st.scene_cam.camera_view(), rect);
    let world = &engine.world;
    st.visibility.prune(world);
    let hidden = st.visibility.hidden_bits(world);
    let icons = if st.scene_ui.gizmos {
        draw_icons(world, &proj, &painter, &hidden)
    } else {
        Vec::new()
    };
    draw_selection(st, world, &proj, &painter);
    // A Tile Palette tool paints instead of selecting.
    let palette_open = st.tile_palette_open;
    if crate::tile_palette::scene_paint(st, engine, ui, &resp, &proj, &painter, palette_open) {
        return;
    }
    let over_handle = handles(st, engine, ui, &resp, &proj, &painter);

    // Click to select (Shift/Cmd adds or removes).
    if resp.clicked()
        && !over_handle
        && st.scene_ui.nav == Nav::None
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let world = &engine.world;
        let vis = &st.visibility;
        let icon_hit = icons
            .iter()
            .filter(|(id, p)| p.distance(pos) < 12.0 && vis.can_pick(world, *id))
            .min_by(|a, b| a.1.distance(pos).total_cmp(&b.1.distance(pos)))
            .map(|(id, _)| *id);
        let hit = icon_hit.or_else(|| {
            pick_where(world, &proj.ray(pos), |id| vis.can_pick(world, id)).map(|(id, _)| id)
        });
        let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        match (hit, additive) {
            (Some(id), true) => st.toggle_select(id),
            (Some(id), false) => {
                st.select(id);
                st.hierarchy.reveal(world, id);
            }
            (None, false) => st.selection.clear(),
            (None, true) => {}
        }
    }

    // Drop a model from the Project window.
    if let Some(payload) = resp.dnd_release_payload::<AssetPayload>()
        && let Some(pos) = resp
            .interact_pointer_pos()
            .or(ui.input(|i| i.pointer.hover_pos()))
    {
        drop_model(st, engine, &payload.0, &proj.ray(pos));
    }

    if !st.scene_ui.mode_2d {
        orientation_gizmo(st, ui, rect);
    }
}

fn overlay_bar(st: &mut EditorState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("scene_draw_mode")
            .selected_text(st.scene_ui.draw_mode.name())
            .show_ui(ui, |ui| {
                for mode in oxide_render::DebugView::ALL {
                    ui.selectable_value(&mut st.scene_ui.draw_mode, mode, mode.name());
                }
            })
            .response
            .on_hover_text("Draw Mode");
        let mut two_d = st.scene_ui.mode_2d;
        if ui
            .toggle_value(&mut two_d, "2D")
            .on_hover_text("2D mode")
            .changed()
        {
            set_2d(st, two_d);
        }
        ui.toggle_value(&mut st.scene_ui.lighting, "Light")
            .on_hover_text("Scene lighting (off: lit from the camera)");
        let fx = &mut st.scene_ui.effects;
        let fx_label = if fx.enabled {
            "Effects"
        } else {
            "Effects (off)"
        };
        ui.menu_button(fx_label, |ui| {
            ui.checkbox(&mut fx.enabled, "Enabled");
            ui.separator();
            ui.checkbox(&mut fx.skybox, "Skybox");
            ui.checkbox(&mut fx.fog, "Fog");
            ui.checkbox(&mut fx.particles, "Particle Systems");
            ui.checkbox(&mut fx.post_processing, "Post Processing");
        })
        .response
        .on_hover_text("Effects");
        let mut vis_on = !st.visibility.show_all;
        if ui
            .toggle_value(&mut vis_on, "Vis")
            .on_hover_text("Scene visibility: off shows objects hidden in the Hierarchy")
            .changed()
        {
            st.visibility.show_all = !vis_on;
        }
        ui.toggle_value(&mut st.scene_ui.grid, "#")
            .on_hover_text("Grid");
        ui.toggle_value(&mut st.scene_ui.gizmos, "Gizmos");
        let label = if st.scene_cam.orthographic {
            "Iso"
        } else {
            "Persp"
        };
        if ui
            .add_enabled(!st.scene_ui.mode_2d, egui::Button::new(label))
            .on_hover_text("Perspective / Isometric")
            .clicked()
        {
            st.scene_cam.orthographic = !st.scene_cam.orthographic;
        }
        ui.separator();
        ui.label("Speed");
        ui.add(
            egui::DragValue::new(&mut st.scene_cam.speed)
                .range(0.1..=1000.0)
                .speed(0.1),
        );
        ui.label("FOV");
        ui.add(
            egui::DragValue::new(&mut st.scene_cam.fov)
                .range(4.0..=120.0)
                .speed(0.5),
        );
    });
}

fn navigate(st: &mut EditorState, ui: &egui::Ui, resp: &egui::Response) {
    use egui::{Key, PointerButton};
    let (delta, scroll, alt, shift, dt, press, down) = ui.input(|i| {
        (
            i.pointer.delta(),
            i.smooth_scroll_delta.y,
            i.modifiers.alt,
            i.modifiers.shift,
            i.stable_dt.min(0.1),
            [
                i.pointer.button_pressed(PointerButton::Primary),
                i.pointer.button_pressed(PointerButton::Secondary),
                i.pointer.button_pressed(PointerButton::Middle),
            ],
            [
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
                i.pointer.middle_down(),
            ],
        )
    });
    let hovered = resp.hovered();
    let mode_2d = st.scene_ui.mode_2d;
    let nav = &mut st.scene_ui.nav;
    if *nav == Nav::None && hovered {
        if press[1] {
            // 2D mode has no rotation: right drag pans.
            *nav = if mode_2d { Nav::Pan } else { Nav::Fly };
        } else if press[0] && alt {
            *nav = if mode_2d { Nav::Pan } else { Nav::Orbit };
        } else if press[2] || (press[0] && st.tool == Tool::Hand) {
            *nav = Nav::Pan;
        }
    }
    let still = match *nav {
        Nav::Fly => down[1],
        Nav::Orbit => down[0],
        Nav::Pan => down[0] || down[1] || down[2],
        Nav::None => false,
    };
    if !still {
        *nav = Nav::None;
    }
    st.scene_ui.flying = *nav == Nav::Fly;
    let cam = &mut st.scene_cam;
    match *nav {
        Nav::Fly => {
            cam.rotate_in_place(delta.x * 0.25, delta.y * 0.25);
            let keys = ui.input(|i| {
                [
                    i.key_down(Key::W),
                    i.key_down(Key::S),
                    i.key_down(Key::D),
                    i.key_down(Key::A),
                    i.key_down(Key::E),
                    i.key_down(Key::Q),
                ]
            });
            let rot = cam.rotation();
            let axis = |pos: bool, neg: bool| pos as i32 as f32 - neg as i32 as f32;
            let dir = rot * Vector3::Z * axis(keys[0], keys[1])
                + rot * Vector3::X * axis(keys[2], keys[3])
                + Vector3::Y * axis(keys[4], keys[5]);
            let speed = cam.speed * if shift { 4.0 } else { 1.0 };
            cam.pivot += dir * speed * dt;
            if scroll != 0.0 {
                cam.speed = (cam.speed * (scroll * 0.004).exp()).clamp(0.05, 2000.0);
            }
        }
        Nav::Orbit => {
            cam.yaw += delta.x * 0.3;
            cam.pitch = (cam.pitch + delta.y * 0.3).clamp(-89.9, 89.9);
        }
        Nav::Pan => {
            let half = (cam.fov * 0.5).to_radians().tan();
            let per_px = cam.distance * half * 2.0 / resp.rect.height().max(1.0);
            let rot = cam.rotation();
            cam.pivot += (rot * Vector3::X * -delta.x + rot * Vector3::Y * delta.y) * per_px;
        }
        Nav::None => {
            if hovered && scroll != 0.0 {
                cam.distance = (cam.distance * (-scroll * 0.003).exp()).clamp(0.05, 100_000.0);
            }
        }
    }
}

// ---- picking ------------------------------------------------------------------------------

fn mesh_of(world: &World, id: GameObjectId) -> Option<(Handle<Mesh>, bool)> {
    if let Some(m) = world
        .get_component::<MeshFilter>(id)
        .and_then(|f| f.mesh.clone())
    {
        return Some((m, false));
    }
    world
        .get_component::<SkinnedMeshRenderer>(id)
        .and_then(|s| s.mesh.clone())
        .map(|m| (m, true))
}

/// Slab test with an unnormalized direction; returns the entry parameter.
fn ray_box(min: Vector3, max: Vector3, o: Vector3, d: Vector3) -> Option<f32> {
    let inv = d.recip();
    let t1 = (min - o) * inv;
    let t2 = (max - o) * inv;
    let near = t1.min(t2).max_element();
    let far = t1.max(t2).min_element();
    (far >= 0.0 && near <= far).then_some(near.max(0.0))
}

/// Möller–Trumbore, both faces.
fn ray_triangle(o: Vector3, d: Vector3, a: Vector3, b: Vector3, c: Vector3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

fn ray_terrain(world: &World, id: GameObjectId, ray: &Ray) -> Option<f32> {
    let t = world.get_component::<oxide_terrain::Terrain>(id)?;
    let assets = world.resource::<AssetServer>();
    let data = assets.terrains.get(t.data.as_ref()?)?;
    let origin = world.position(id);
    let size = data.size;
    let t0 = ray_box(origin, origin + size, ray.origin, ray.direction)?;
    let step = (size.x.max(size.z) / data.resolution.max(2) as f32).max(0.05);
    let above = |p: Vector3| {
        let u = (p.x - origin.x) / size.x;
        let v = (p.z - origin.z) / size.z;
        p.y - origin.y - data.interpolated_height(u, v)
    };
    let mut prev = t0;
    let mut t = t0;
    let end = t0 + (size.length() * 1.5);
    while t < end {
        let p = ray.get_point(t);
        if above(p) <= 0.0 {
            // Refine between prev and t.
            let (mut a, mut b) = (prev, t);
            for _ in 0..16 {
                let m = (a + b) * 0.5;
                if above(ray.get_point(m)) > 0.0 {
                    a = m
                } else {
                    b = m
                }
            }
            return Some(b);
        }
        prev = t;
        t += step;
    }
    None
}

/// Closest object under a world ray (meshes by triangle, skinned meshes by bounds, terrain
/// by heightmap). Returns the object and hit distance.
pub fn pick(world: &World, ray: &Ray) -> Option<(GameObjectId, f32)> {
    pick_where(world, ray, |_| true)
}

/// [`pick`] limited to objects `keep` accepts (Scene view pickability).
pub fn pick_where(
    world: &World,
    ray: &Ray,
    keep: impl Fn(GameObjectId) -> bool,
) -> Option<(GameObjectId, f32)> {
    let assets = world.resource::<AssetServer>();
    let mut best: Option<(GameObjectId, f32)> = None;
    for (id, o) in world.objects() {
        if !o.active_in_hierarchy() || !keep(id) {
            continue;
        }
        if let Some(t) = ray_terrain(world, id, ray)
            && best.is_none_or(|b| t < b.1)
        {
            best = Some((id, t));
        }
        let Some((h, skinned)) = mesh_of(world, id) else {
            continue;
        };
        let Some(mesh) = assets.meshes.get(&h) else {
            continue;
        };
        let inv = world.local_to_world(id).inverse();
        let o_l = inv.transform_point3(ray.origin);
        let d_l = inv.transform_vector3(ray.direction);
        let Some(tb) = ray_box(mesh.bounds.min(), mesh.bounds.max(), o_l, d_l) else {
            continue;
        };
        if best.is_some_and(|b| tb > b.1) {
            continue;
        }
        let t = if skinned || mesh.indices.is_empty() {
            Some(tb)
        } else {
            mesh.triangles()
                .filter_map(|[a, b, c]| {
                    let v = &mesh.vertices;
                    ray_triangle(o_l, d_l, v[a as usize], v[b as usize], v[c as usize])
                })
                .min_by(f32::total_cmp)
        };
        if let Some(t) = t
            && best.is_none_or(|b| t < b.1)
        {
            best = Some((id, t));
        }
    }
    best
}

fn drop_model(st: &mut EditorState, engine: &mut Engine, path: &str, ray: &Ray) {
    let lower = path.to_ascii_lowercase();
    let prefab = lower.ends_with(".prefab");
    if !(prefab || lower.ends_with(".glb") || lower.ends_with(".gltf")) {
        st.status = format!("{path}: only prefabs and models can be dropped into the scene");
        return;
    }
    let now = oxide_core::time::realtime_since_startup();
    let w = &mut engine.world;
    let point = pick(w, ray)
        .map(|(_, t)| ray.get_point(t))
        .or_else(|| {
            oxide_math::Plane::new(Vector3::Y, Vector3::ZERO)
                .raycast(ray)
                .map(|t| ray.get_point(t))
        })
        .unwrap_or(st.scene_cam.pivot);
    if prefab {
        st.record(w, "Instantiate Prefab", None, now);
        match w.instantiate_prefab(path, None) {
            Ok(id) => {
                w.set_position(id, point);
                st.select(id);
                st.status = format!("Instantiated {path}");
            }
            Err(e) => st.status = e,
        }
        return;
    }
    let h = match w.resource_mut::<AssetServer>().load_model(path) {
        Ok(h) => h,
        Err(e) => {
            st.status = format!("cannot import {path}: {e}");
            return;
        }
    };
    st.record(w, "Instantiate Model", None, now);
    if let Some(id) = oxide_engine::objects::instantiate_model(w, &h) {
        w.set_position(id, point);
        st.select(id);
        st.status = format!("Instantiated {path}");
    }
}

// ---- bounds & framing ---------------------------------------------------------------------

/// World bounding sphere of objects and their children.
pub fn bounds_of(world: &World, ids: &[GameObjectId]) -> Option<(Vector3, f32)> {
    let assets = world.resource::<AssetServer>();
    let mut min = Vector3::splat(f32::MAX);
    let mut max = Vector3::splat(f32::MIN);
    let mut any = false;
    for &root in ids {
        for id in world.descendants(root) {
            let m = world.local_to_world(id);
            if let Some((h, _)) = mesh_of(world, id)
                && let Some(mesh) = assets.meshes.get(&h)
            {
                for c in corners(mesh.bounds.min(), mesh.bounds.max()) {
                    let p = m.transform_point3(c);
                    min = min.min(p);
                    max = max.max(p);
                }
                any = true;
            } else if let Some(t) = world.get_component::<oxide_terrain::Terrain>(id)
                && let Some(d) = t.data.as_ref().and_then(|h| assets.terrains.get(h))
            {
                let o = world.position(id);
                min = min.min(o);
                max = max.max(o + d.size);
                any = true;
            } else {
                let p = world.position(id);
                min = min.min(p - Vector3::splat(0.5));
                max = max.max(p + Vector3::splat(0.5));
                any = true;
            }
        }
    }
    any.then(|| ((min + max) * 0.5, ((max - min) * 0.5).length()))
}

fn frame_selection(st: &mut EditorState, world: &World) {
    if let Some((c, r)) = bounds_of(world, &st.selection) {
        st.scene_cam.frame(c, r);
    }
}

fn corners(min: Vector3, max: Vector3) -> [Vector3; 8] {
    [
        Vector3::new(min.x, min.y, min.z),
        Vector3::new(max.x, min.y, min.z),
        Vector3::new(max.x, max.y, min.z),
        Vector3::new(min.x, max.y, min.z),
        Vector3::new(min.x, min.y, max.z),
        Vector3::new(max.x, min.y, max.z),
        Vector3::new(max.x, max.y, max.z),
        Vector3::new(min.x, max.y, max.z),
    ]
}

const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (1, 2),
    (2, 3),
    (3, 0),
    (4, 5),
    (5, 6),
    (6, 7),
    (7, 4),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

// ---- drawing helpers ----------------------------------------------------------------------

fn line(p: &egui::Painter, proj: &Projector, a: Vector3, b: Vector3, stroke: Stroke) {
    if let Some(s) = proj.segment(a, b) {
        p.line_segment(s, stroke);
    }
}

fn wire_box(
    p: &egui::Painter,
    proj: &Projector,
    m: &Matrix4x4,
    min: Vector3,
    max: Vector3,
    s: Stroke,
) {
    let c = corners(min, max).map(|c| m.transform_point3(c));
    for (a, b) in BOX_EDGES {
        line(p, proj, c[a], c[b], s);
    }
}

fn circle(
    p: &egui::Painter,
    proj: &Projector,
    center: Vector3,
    normal: Vector3,
    r: f32,
    s: Stroke,
) {
    let (u, v) = basis(normal);
    let mut prev = center + u * r;
    for i in 1..=48 {
        let a = i as f32 / 48.0 * std::f32::consts::TAU;
        let q = center + (u * a.cos() + v * a.sin()) * r;
        line(p, proj, prev, q, s);
        prev = q;
    }
}

fn basis(n: Vector3) -> (Vector3, Vector3) {
    let n = n.normalize_or(Vector3::Y);
    let helper = if n.y.abs() < 0.99 {
        Vector3::Y
    } else {
        Vector3::X
    };
    let u = n.cross(helper).normalize();
    (u, n.cross(u))
}

/// Light and camera icons. Returns their screen positions for click picking.
fn draw_icons(
    world: &World,
    proj: &Projector,
    p: &egui::Painter,
    hidden: &std::collections::HashSet<u64>,
) -> Vec<(GameObjectId, Pos2)> {
    let mut out = Vec::new();
    let shown = |id: GameObjectId| {
        !hidden.contains(&id.to_bits()) && world.get(id).is_some_and(|o| o.active_in_hierarchy())
    };
    for (id, light) in world.query::<Light>() {
        if !shown(id) {
            continue;
        }
        let Some(s) = proj.project(world.position(id)) else {
            continue;
        };
        let c = light.color;
        let col = Color32::from_rgb(
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
        );
        p.circle_filled(s, 5.0, col);
        p.circle_stroke(s, 5.0, Stroke::new(1.0, Color32::BLACK));
        for i in 0..8 {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            let d = vec2(a.cos(), a.sin());
            p.line_segment([s + d * 7.0, s + d * 10.0], Stroke::new(1.5, col));
        }
        out.push((id, s));
    }
    for (id, _) in world.query::<Camera>() {
        if !shown(id) {
            continue;
        }
        let Some(s) = proj.project(world.position(id)) else {
            continue;
        };
        let body = egui::Rect::from_center_size(s - vec2(2.0, 0.0), vec2(12.0, 9.0));
        p.rect_filled(body, 1.5, Color32::from_gray(220));
        p.add(egui::Shape::convex_polygon(
            vec![s + vec2(4.0, 0.0), s + vec2(9.0, -4.5), s + vec2(9.0, 4.5)],
            Color32::from_gray(220),
            Stroke::NONE,
        ));
        p.rect_stroke(
            body,
            1.5,
            Stroke::new(1.0, Color32::BLACK),
            egui::StrokeKind::Outside,
        );
        out.push((id, s));
    }
    out
}

/// Selection outlines plus camera, light and collider gizmos of selected objects.
fn draw_selection(st: &EditorState, world: &World, proj: &Projector, p: &egui::Painter) {
    let assets = world.resource::<AssetServer>();
    let outline = Stroke::new(1.2, SELECTION);
    let mut drawn = 0;
    for &root in &st.selection {
        if !world.is_alive(root) {
            continue;
        }
        for id in world.descendants(root) {
            if drawn > 400 {
                break;
            }
            if let Some((h, _)) = mesh_of(world, id)
                && let Some(mesh) = assets.meshes.get(&h)
            {
                let m = world.local_to_world(id);
                wire_box(p, proj, &m, mesh.bounds.min(), mesh.bounds.max(), outline);
                drawn += 1;
            }
        }
        if !st.scene_ui.gizmos {
            continue;
        }
        let pos = world.position(root);
        let rot = world.rotation(root);
        if let Some(cam) = world.get_component::<Camera>(root) {
            let aspect = st
                .game_view
                .rect
                .map_or(16.0 / 9.0, |r| r.width() / r.height().max(1.0));
            let far = cam.far_clip_plane.min(200.0);
            let s = Stroke::new(1.0, Color32::from_gray(230));
            let corners_at = |d: f32| -> [Vector3; 4] {
                let (hw, hh) = if cam.projection == oxide_scene::Projection::Orthographic {
                    (cam.orthographic_size * aspect, cam.orthographic_size)
                } else {
                    let h = d * (cam.field_of_view * 0.5).to_radians().tan();
                    (h * aspect, h)
                };
                [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    .map(|(x, y)| pos + rot * Vector3::new(x * hw, y * hh, d))
            };
            let n = corners_at(cam.near_clip_plane);
            let f = corners_at(far);
            for i in 0..4 {
                line(p, proj, n[i], n[(i + 1) % 4], s);
                line(p, proj, f[i], f[(i + 1) % 4], s);
                line(p, proj, n[i], f[i], s);
            }
        }
        if let Some(l) = world.get_component::<Light>(root) {
            let s = Stroke::new(1.0, Color32::from_rgb(254, 253, 136));
            match l.light_type {
                LightType::Point => {
                    for n in [Vector3::X, Vector3::Y, Vector3::Z] {
                        circle(p, proj, pos, n, l.range, s);
                    }
                }
                LightType::Spot => {
                    let fwd = rot * Vector3::Z;
                    let r = l.range * (l.spot_angle * 0.5).to_radians().tan();
                    let end = pos + fwd * l.range;
                    circle(p, proj, end, fwd, r, s);
                    let (u, v) = basis(fwd);
                    for d in [u, -u, v, -v] {
                        line(p, proj, pos, end + d * r, s);
                    }
                }
                _ => {
                    let fwd = rot * Vector3::Z;
                    let (u, v) = basis(fwd);
                    let r = proj.world_per_pixels(pos, 14.0);
                    let len = proj.world_per_pixels(pos, 60.0);
                    for d in [u, -u, v, -v, Vector3::ZERO] {
                        line(p, proj, pos + d * r, pos + d * r + fwd * len, s);
                    }
                }
            }
        }
        draw_colliders(world, root, proj, p);
        if let Some(g) = world.get_component::<oxide_scene::LODGroup>(root) {
            let cam = st.scene_cam.camera_view();
            let bias = oxide_scene::QualitySettings::of(world).lod_bias;
            let (c, r) = oxide_engine::extract::lod_group_sphere(world, root, g);
            let level = oxide_engine::extract::lod_level_for(world, root, g, &cam, bias);
            let pct = oxide_engine::extract::screen_relative_height(&cam, c, r) * bias * 100.0;
            if let Some(at) = proj.project(c) {
                let text = match level {
                    Some(l) => format!("LOD {l}  ({pct:.0}%)"),
                    None => format!("Culled  ({pct:.1}%)"),
                };
                p.text(
                    at + vec2(0.0, -18.0),
                    egui::Align2::CENTER_BOTTOM,
                    text,
                    egui::FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
        }
    }
}

/// Green wireframes for BoxCollider/SphereCollider/CapsuleCollider, read via reflection.
fn draw_colliders(world: &World, id: GameObjectId, proj: &Projector, p: &egui::Painter) {
    use oxide_core::reflect::Value;
    let Some(o) = world.get(id) else { return };
    let m = world.local_to_world(id);
    let s = Stroke::new(1.0, COLLIDER);
    for (cid, name, enabled) in o.component_list() {
        if !enabled || !name.ends_with("Collider") {
            continue;
        }
        let Some(c) = world.component_dyn(id, cid) else {
            continue;
        };
        let v3 = |f: &str| match c.get_field(f) {
            Some(Value::Vector3(v)) => Some(Vector3::from_array(v)),
            _ => None,
        };
        let f32_ = |f: &str| match c.get_field(f) {
            Some(Value::Float(v)) => Some(v as f32),
            _ => None,
        };
        let center = v3("center").unwrap_or(Vector3::ZERO);
        if let Some(size) = v3("size") {
            wire_box(p, proj, &m, center - size * 0.5, center + size * 0.5, s);
        } else if let Some(r) = f32_("radius") {
            let scale = world.lossy_scale(id).abs().max_element();
            let wc = m.transform_point3(center);
            let r = r * scale;
            match f32_("height") {
                Some(h) => {
                    let up = world.rotation(id) * Vector3::Y;
                    let half = (h * world.lossy_scale(id).y.abs() * 0.5 - r).max(0.0);
                    for e in [wc + up * half, wc - up * half] {
                        circle(p, proj, e, up, r, s);
                        circle(p, proj, e, world.rotation(id) * Vector3::X, r, s);
                    }
                    let (u, v) = basis(up);
                    for d in [u, -u, v, -v] {
                        line(p, proj, wc + up * half + d * r, wc - up * half + d * r, s);
                    }
                }
                None => {
                    for n in [Vector3::X, Vector3::Y, Vector3::Z] {
                        circle(p, proj, wc, n, r, s);
                    }
                }
            }
        }
    }
}

// ---- transform handles --------------------------------------------------------------------

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn point_in_quad(p: Pos2, q: &[Pos2; 4]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let c = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if c.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

/// Parameter `s` of the point on line `origin + axis * s` closest to the ray.
fn closest_on_axis(ray: &Ray, origin: Vector3, axis: Vector3) -> Option<f32> {
    let w0 = origin - ray.origin;
    let b = axis.dot(ray.direction);
    let a = axis.dot(axis);
    let d = axis.dot(w0);
    let e = ray.direction.dot(w0);
    let denom = a - b * b;
    (denom.abs() > 1e-5).then(|| (b * e - d) / denom)
}

fn ray_plane(ray: &Ray, point: Vector3, normal: Vector3) -> Option<Vector3> {
    let denom = normal.dot(ray.direction);
    if denom.abs() < 1e-5 {
        return None;
    }
    let t = normal.dot(point - ray.origin) / denom;
    (t > 0.0).then(|| ray.get_point(t))
}

struct HandleGeom {
    center: Vector3,
    axes: [Vector3; 3],
    size: f32,
    /// Rotation ring radius relative to `size` (larger for the Transform tool).
    ring: f32,
}

impl HandleGeom {
    /// Plane handle `i` (normal = axis i), offset toward the camera.
    fn plane_quad(&self, proj: &Projector, i: usize) -> Option<[Pos2; 4]> {
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        let to_cam = proj.to_camera(self.center);
        let sj = if self.axes[j].dot(to_cam) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let sk = if self.axes[k].dot(to_cam) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let aj = self.axes[j] * sj * self.size;
        let ak = self.axes[k] * sk * self.size;
        let (a, b) = (0.12, 0.36);
        let q = [
            self.center + aj * a + ak * a,
            self.center + aj * b + ak * a,
            self.center + aj * b + ak * b,
            self.center + aj * a + ak * b,
        ];
        Some([
            proj.project(q[0])?,
            proj.project(q[1])?,
            proj.project(q[2])?,
            proj.project(q[3])?,
        ])
    }

    /// Front-facing points of rotation ring `i`.
    fn ring(&self, proj: &Projector, i: usize) -> Vec<(Vector3, Option<Pos2>)> {
        let (u, v) = basis(self.axes[i]);
        let to_cam = proj.to_camera(self.center);
        (0..=64)
            .map(|n| {
                let a = n as f32 / 64.0 * std::f32::consts::TAU;
                let d = u * a.cos() + v * a.sin();
                let wp = self.center + d * self.size * self.ring;
                let front = d.dot(to_cam) >= -0.05;
                (wp, if front { proj.project(wp) } else { None })
            })
            .collect()
    }

    fn hit(&self, tool: Tool, proj: &Projector, m: Pos2) -> Option<Handle3> {
        let c = proj.project(self.center)?;
        match tool {
            Tool::Move => {
                for i in 0..3 {
                    if self
                        .plane_quad(proj, i)
                        .is_some_and(|q| point_in_quad(m, &q))
                    {
                        return Some(Handle3::Plane(i));
                    }
                }
                self.nearest_axis(proj, c, m, 1.15)
            }
            Tool::Scale => {
                if c.distance(m) < 9.0 {
                    return Some(Handle3::Uniform);
                }
                self.nearest_axis(proj, c, m, 1.05)
            }
            Tool::Rotate => {
                let mut best: Option<(usize, f32)> = None;
                for i in 0..3 {
                    let pts = self.ring(proj, i);
                    for w in pts.windows(2) {
                        if let (Some(a), Some(b)) = (w[0].1, w[1].1) {
                            let d = dist_to_segment(m, a, b);
                            if d < 8.0 && best.is_none_or(|x| d < x.1) {
                                best = Some((i, d));
                            }
                        }
                    }
                }
                best.map(|(i, _)| Handle3::Ring(i))
            }
            Tool::Transform => {
                if c.distance(m) < 9.0 {
                    return Some(Handle3::Uniform);
                }
                self.hit(Tool::Move, proj, m)
                    .or_else(|| self.hit(Tool::Rotate, proj, m))
            }
            Tool::Hand | Tool::Rect => None,
        }
    }

    fn nearest_axis(&self, proj: &Projector, c: Pos2, m: Pos2, ext: f32) -> Option<Handle3> {
        let mut best: Option<(usize, f32)> = None;
        for i in 0..3 {
            let Some(e) = proj.project(self.center + self.axes[i] * self.size * ext) else {
                continue;
            };
            if c.distance(e) < 8.0 {
                continue;
            }
            let d = dist_to_segment(m, c, e);
            if d < 8.0 && best.is_none_or(|b| d < b.1) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| Handle3::Axis(i))
    }
}

/// Draws and runs the active tool's handles. Returns true if the pointer is on a handle (so
/// a click doesn't pick through it).
fn handles(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &egui::Ui,
    resp: &egui::Response,
    proj: &Projector,
    painter: &egui::Painter,
) -> bool {
    let tool = st.tool;
    let Some(active) = st.active().filter(|&a| engine.world.is_alive(a)) else {
        st.scene_ui.drag = None;
        return false;
    };
    if tool == Tool::Hand {
        return false;
    }
    if tool == Tool::Rect {
        return rect_tool(st, engine, ui, resp, proj, painter, active);
    }
    let w = &engine.world;
    let center = if st.center_handles {
        bounds_of(w, &st.top_level_selection(w)).map_or(w.position(active), |(c, _)| c)
    } else {
        w.position(active)
    };
    let rot = w.rotation(active);
    let local = st.local_handles || tool == Tool::Scale;
    let axes = if local {
        [rot * Vector3::X, rot * Vector3::Y, rot * Vector3::Z]
    } else {
        [Vector3::X, Vector3::Y, Vector3::Z]
    };
    let size = proj.world_per_pixels(center, HANDLE_PX);
    let ring = if tool == Tool::Transform { 1.35 } else { 1.0 };
    let geom = HandleGeom {
        center,
        axes,
        size,
        ring,
    };
    let around_center = st.center_handles;
    let (mouse, press_origin, primary_down, snap) = ui.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.press_origin(),
            i.pointer.primary_down(),
            i.modifiers.command,
        )
    });
    let busy = st.scene_ui.nav != Nav::None;

    // Start a drag.
    if st.scene_ui.drag.is_none()
        && !busy
        && resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(origin) = press_origin
        && let Some(h) = geom.hit(tool, proj, origin)
    {
        let now = oxide_core::time::realtime_since_startup();
        // The Transform tool acts as whichever tool owns the grabbed handle.
        let tool = match (tool, h) {
            (Tool::Transform, Handle3::Axis(_) | Handle3::Plane(_)) => Tool::Move,
            (Tool::Transform, Handle3::Ring(_)) => Tool::Rotate,
            (Tool::Transform, _) => Tool::Scale,
            (t, _) => t,
        };
        let label = match tool {
            Tool::Move => "Move",
            Tool::Rotate => "Rotate",
            _ => "Scale",
        };
        st.record(w, label, None, now);
        let w = &engine.world;
        let ray = proj.ray(origin);
        let objects = st
            .top_level_selection(w)
            .into_iter()
            .map(|id| {
                let ls = w.transform(id).map_or(Vector3::ONE, |t| t.local_scale);
                (id, w.position(id), w.rotation(id), ls)
            })
            .collect();
        let mut d = Drag {
            handle: h,
            tool,
            start_mouse: origin,
            center,
            axes,
            size,
            start_s: 0.0,
            start_hit: center,
            tangent: Vec2::X,
            radius_px: 60.0,
            objects,
            factor: 1.0,
            rect: None,
        };
        match h {
            Handle3::Axis(i) => d.start_s = closest_on_axis(&ray, center, axes[i]).unwrap_or(0.0),
            Handle3::Plane(i) => d.start_hit = ray_plane(&ray, center, axes[i]).unwrap_or(center),
            Handle3::Ring(i) => {
                let pts = geom.ring(proj, i);
                if let Some((g, gs)) = pts
                    .iter()
                    .filter_map(|(wp, s)| s.map(|s| (*wp, s)))
                    .min_by(|a, b| a.1.distance(origin).total_cmp(&b.1.distance(origin)))
                {
                    let t = axes[i].cross(g - center).normalize_or(Vector3::X);
                    if let (Some(a), Some(b), Some(c)) = (
                        proj.project(g),
                        proj.project(g + t * size * 0.05),
                        proj.project(center),
                    ) {
                        d.tangent = (b - a).normalized();
                        d.radius_px = gs.distance(c).max(40.0);
                    }
                }
            }
            Handle3::Uniform | Handle3::Rect(..) => {}
        }
        st.scene_ui.drag = Some(d);
    }

    // Update a drag.
    if let Some(d) = st.scene_ui.drag.as_mut() {
        if !primary_down {
            st.scene_ui.drag = None;
        } else if let Some(m) = mouse {
            let ray = proj.ray(m);
            let w = &mut engine.world;
            match (d.tool, d.handle) {
                (Tool::Move, Handle3::Axis(i)) => {
                    if let Some(s) = closest_on_axis(&ray, d.center, d.axes[i]) {
                        let mut delta = s - d.start_s;
                        if snap {
                            let step = st.prefs.move_snap.max(1e-4);
                            delta = (delta / step).round() * step;
                        }
                        for (id, p0, _, _) in &d.objects {
                            w.set_position(*id, *p0 + d.axes[i] * delta);
                        }
                    }
                }
                (Tool::Move, Handle3::Plane(i)) => {
                    if let Some(hit) = ray_plane(&ray, d.center, d.axes[i]) {
                        let mut off = hit - d.start_hit;
                        if snap {
                            let a = &d.axes;
                            let step = st.prefs.move_snap.max(1e-4);
                            let comp = |v: Vector3| (v / step).round() * step;
                            let c = comp(Vector3::new(off.dot(a[0]), off.dot(a[1]), off.dot(a[2])));
                            off = a[0] * c.x + a[1] * c.y + a[2] * c.z;
                        }
                        for (id, p0, _, _) in &d.objects {
                            w.set_position(*id, *p0 + off);
                        }
                    }
                }
                (Tool::Rotate, Handle3::Ring(i)) => {
                    let mut deg = ((m - d.start_mouse).dot(d.tangent) / d.radius_px).to_degrees();
                    if snap {
                        let step = st.prefs.rotate_snap.max(0.01);
                        deg = (deg / step).round() * step;
                    }
                    let q = Quaternion::angle_axis(deg, d.axes[i]);
                    for (id, p0, r0, _) in &d.objects {
                        w.set_rotation(*id, (q * *r0).normalize());
                        if around_center {
                            w.set_position(*id, d.center + q * (*p0 - d.center));
                        }
                    }
                }
                (Tool::Scale, h) => {
                    let mut f = match h {
                        Handle3::Axis(i) => closest_on_axis(&ray, d.center, d.axes[i])
                            .map_or(1.0, |s| 1.0 + (s - d.start_s) / d.size),
                        _ => {
                            let dm = m - d.start_mouse;
                            1.0 + (dm.x - dm.y) * 0.01
                        }
                    };
                    if snap {
                        let step = st.prefs.scale_snap.max(1e-4);
                        f = (f / step).round() * step;
                    }
                    let f = if f.abs() < 0.01 {
                        0.01f32.copysign(f)
                    } else {
                        f
                    };
                    d.factor = f;
                    let fv = match h {
                        Handle3::Axis(0) => Vector3::new(f, 1.0, 1.0),
                        Handle3::Axis(1) => Vector3::new(1.0, f, 1.0),
                        Handle3::Axis(_) => Vector3::new(1.0, 1.0, f),
                        _ => Vector3::splat(f),
                    };
                    for (id, p0, _, s0) in &d.objects {
                        if around_center {
                            // Spread positions out from the center along the handle axes.
                            let a = &d.axes;
                            let o = *p0 - d.center;
                            let p = d.center
                                + a[0] * (o.dot(a[0]) * fv.x)
                                + a[1] * (o.dot(a[1]) * fv.y)
                                + a[2] * (o.dot(a[2]) * fv.z);
                            w.set_position(*id, p);
                        }
                        if let Some(t) = w.transform_mut(*id) {
                            t.local_scale = match h {
                                Handle3::Axis(0) => Vector3::new(s0.x * f, s0.y, s0.z),
                                Handle3::Axis(1) => Vector3::new(s0.x, s0.y * f, s0.z),
                                Handle3::Axis(_) => Vector3::new(s0.x, s0.y, s0.z * f),
                                _ => *s0 * f,
                            };
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Draw.
    let dragging = st.scene_ui.drag.as_ref().map(|d| d.handle);
    let hovered = dragging.or_else(|| {
        if busy {
            None
        } else {
            mouse
                .filter(|m| proj.rect.contains(*m))
                .and_then(|m| geom.hit(tool, proj, m))
        }
    });
    let factor = st.scene_ui.drag.as_ref().map_or(1.0, |d| d.factor);
    draw_handles(&geom, tool, hovered, factor, proj, painter);
    hovered.is_some()
}

fn draw_handles(
    g: &HandleGeom,
    tool: Tool,
    hovered: Option<Handle3>,
    factor: f32,
    proj: &Projector,
    p: &egui::Painter,
) {
    let Some(c) = proj.project(g.center) else {
        return;
    };
    let color = |h: Handle3, i: usize| {
        if hovered == Some(h) {
            HOVER
        } else {
            AXIS_COLORS[i]
        }
    };
    match tool {
        Tool::Move => {
            for i in 0..3 {
                if let Some(q) = g.plane_quad(proj, i) {
                    let col = color(Handle3::Plane(i), i);
                    p.add(egui::Shape::convex_polygon(
                        q.to_vec(),
                        col.gamma_multiply(0.35),
                        Stroke::new(1.0, col),
                    ));
                }
            }
            for i in 0..3 {
                let Some(e) = proj.project(g.center + g.axes[i] * g.size) else {
                    continue;
                };
                if c.distance(e) < 8.0 {
                    continue;
                }
                let col = color(Handle3::Axis(i), i);
                p.line_segment([c, e], Stroke::new(2.5, col));
                let dir = (e - c).normalized();
                let perp = vec2(-dir.y, dir.x);
                p.add(egui::Shape::convex_polygon(
                    vec![e + dir * 14.0, e + perp * 5.5, e - perp * 5.5],
                    col,
                    Stroke::NONE,
                ));
            }
        }
        Tool::Rotate => {
            for i in 0..3 {
                let col = color(Handle3::Ring(i), i);
                let pts = g.ring(proj, i);
                for w in pts.windows(2) {
                    if let (Some(a), Some(b)) = (w[0].1, w[1].1) {
                        p.line_segment([a, b], Stroke::new(2.0, col));
                    }
                }
            }
        }
        Tool::Scale => {
            for i in 0..3 {
                let len = if hovered == Some(Handle3::Axis(i)) {
                    factor
                } else {
                    1.0
                };
                let Some(e) = proj.project(g.center + g.axes[i] * g.size * len) else {
                    continue;
                };
                if c.distance(e) < 8.0 {
                    continue;
                }
                let col = color(Handle3::Axis(i), i);
                p.line_segment([c, e], Stroke::new(2.5, col));
                p.rect_filled(egui::Rect::from_center_size(e, vec2(9.0, 9.0)), 0.0, col);
            }
            let col = if hovered == Some(Handle3::Uniform) {
                HOVER
            } else {
                Color32::from_gray(200)
            };
            p.rect_filled(egui::Rect::from_center_size(c, vec2(10.0, 10.0)), 0.0, col);
        }
        Tool::Transform => {
            draw_handles(g, Tool::Rotate, hovered, factor, proj, p);
            draw_handles(g, Tool::Move, hovered, factor, proj, p);
            let col = if hovered == Some(Handle3::Uniform) {
                HOVER
            } else {
                Color32::from_gray(200)
            };
            p.rect_stroke(
                egui::Rect::from_center_size(c, vec2(12.0, 12.0)),
                0.0,
                Stroke::new(2.0, col),
                egui::StrokeKind::Middle,
            );
        }
        Tool::Hand | Tool::Rect => {}
    }
}

// ---- rect tool ----------------------------------------------------------------------------

/// The active object's bounds as a rectangle on its local plane that faces the camera most.
#[derive(Clone, Copy, Debug)]
struct RectGeom {
    /// World center of the rect.
    center: Vector3,
    /// Unit in-plane directions and plane normal (the object's local axes in world space).
    u: Vector3,
    v: Vector3,
    n: Vector3,
    /// Half sizes along `u` and `v` (world units).
    hu: f32,
    hv: f32,
    /// Local axis indices of `u` and `v` (which `local_scale` components resize them).
    iu: usize,
    iv: usize,
}

impl RectGeom {
    fn of(world: &World, id: GameObjectId, view_dir: Vector3) -> Option<Self> {
        let m = world.local_to_world(id);
        let (min, max) = mesh_of(world, id)
            .and_then(|(h, _)| {
                let b = world.resource::<AssetServer>().meshes.get(&h)?.bounds;
                Some((b.min(), b.max()))
            })
            .unwrap_or((Vector3::splat(-0.5), Vector3::splat(0.5)));
        let cols = [
            m.x_axis.truncate(),
            m.y_axis.truncate(),
            m.z_axis.truncate(),
        ];
        let len = cols.map(|c| c.length());
        if len.iter().any(|&l| l < 1e-6) {
            return None;
        }
        let dirs = [cols[0] / len[0], cols[1] / len[1], cols[2] / len[2]];
        let ni = (0..3)
            .max_by(|&a, &b| {
                dirs[a]
                    .dot(view_dir)
                    .abs()
                    .total_cmp(&dirs[b].dot(view_dir).abs())
            })
            .unwrap_or(2);
        let (iu, iv) = ((ni + 1) % 3, (ni + 2) % 3);
        let ext = max - min;
        Some(Self {
            center: m.transform_point3((min + max) * 0.5),
            u: dirs[iu],
            v: dirs[iv],
            n: dirs[ni],
            hu: (ext[iu] * 0.5 * len[iu]).max(1e-4),
            hv: (ext[iv] * 0.5 * len[iv]).max(1e-4),
            iu,
            iv,
        })
    }

    /// Dragging edge/corner `(su, sv)` by `(du, dv)` along `u`/`v`: the object's new
    /// position and local scale (from `p0`, `s0` at the start) and the resulting rect.
    fn resize(
        &self,
        p0: Vector3,
        s0: Vector3,
        (su, sv): (i8, i8),
        (du, dv): (f32, f32),
        alt: bool,
        shift: bool,
    ) -> RectResize {
        let grow = |s: i8, dd: f32, h: f32| {
            if s == 0 {
                1.0
            } else if alt {
                (h + s as f32 * dd) / h
            } else {
                (2.0 * h + s as f32 * dd) / (2.0 * h)
            }
        };
        let mut fu = grow(su, du, self.hu);
        let mut fv = grow(sv, dv, self.hv);
        if shift && su != 0 && sv != 0 {
            let f = if fu.abs() > fv.abs() { fu } else { fv };
            (fu, fv) = (f, f);
        }
        let clamp = |f: f32| {
            if f.abs() < 1e-3 {
                1e-3f32.copysign(f)
            } else {
                f
            }
        };
        let (fu, fv) = (clamp(fu), clamp(fv));
        // The far side stays fixed unless Alt resizes around the center.
        let shift_by = |s: i8, f: f32, h: f32| if alt { 0.0 } else { s as f32 * (h * f - h) };
        let (cu, cv) = (shift_by(su, fu, self.hu), shift_by(sv, fv, self.hv));
        let o = self.center - p0;
        let center = self.center + self.u * cu + self.v * cv;
        let position = center
            - (self.u * (o.dot(self.u) * fu)
                + self.v * (o.dot(self.v) * fv)
                + self.n * o.dot(self.n));
        let mut scale = s0;
        scale[self.iu] = s0[self.iu] * fu;
        scale[self.iv] = s0[self.iv] * fv;
        RectResize {
            position,
            scale,
            rect: Self {
                center,
                hu: self.hu * fu.abs(),
                hv: self.hv * fv.abs(),
                ..*self
            },
        }
    }

    fn point(&self, su: f32, sv: f32) -> Vector3 {
        self.center + self.u * (su * self.hu) + self.v * (sv * self.hv)
    }

    fn hit(&self, proj: &Projector, m: Pos2) -> Option<Handle3> {
        let sp = |su: f32, sv: f32| proj.project(self.point(su, sv));
        let corners = [(1, 1), (-1, 1), (-1, -1), (1, -1)];
        for (su, sv) in corners {
            if sp(su as f32, sv as f32).is_some_and(|p| p.distance(m) < 7.0) {
                return Some(Handle3::Rect(su, sv));
            }
        }
        let q = corners.map(|(su, sv)| sp(su as f32, sv as f32));
        let [Some(a), Some(b), Some(c), Some(d)] = q else {
            return None;
        };
        for (e, (p0, p1)) in [(b, c), (c, d), (d, a), (a, b)].into_iter().enumerate() {
            if dist_to_segment(m, p0, p1) < 5.0 {
                // Edges in order: -u, -v, +u, +v.
                return Some(match e {
                    0 => Handle3::Rect(-1, 0),
                    1 => Handle3::Rect(0, -1),
                    2 => Handle3::Rect(1, 0),
                    _ => Handle3::Rect(0, 1),
                });
            }
        }
        point_in_quad(m, &[a, b, c, d]).then_some(Handle3::Rect(0, 0))
    }
}

struct RectResize {
    position: Vector3,
    scale: Vector3,
    rect: RectGeom,
}

/// Unity's Rect tool for 3D objects: drag an edge or corner to resize the active object
/// (the opposite side stays put; Alt resizes around the center, Shift keeps the aspect
/// ratio), or drag inside to move the selection on the rect's plane.
fn rect_tool(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &egui::Ui,
    resp: &egui::Response,
    proj: &Projector,
    painter: &egui::Painter,
    active: GameObjectId,
) -> bool {
    let w = &engine.world;
    let view_dir = st.scene_cam.forward();
    let Some(geom) = st
        .scene_ui
        .drag
        .as_ref()
        .and_then(|d| d.rect)
        .or_else(|| RectGeom::of(w, active, view_dir))
    else {
        return false;
    };
    let (mouse, press_origin, primary_down, mods) = ui.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.press_origin(),
            i.pointer.primary_down(),
            i.modifiers,
        )
    });
    let busy = st.scene_ui.nav != Nav::None;

    if st.scene_ui.drag.is_none()
        && !busy
        && resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(origin) = press_origin
        && let Some(h) = geom.hit(proj, origin)
    {
        let now = oxide_core::time::realtime_since_startup();
        let label = if h == Handle3::Rect(0, 0) {
            "Move"
        } else {
            "Rect Resize"
        };
        st.record(w, label, None, now);
        let w = &engine.world;
        let mut objects: Vec<_> = st
            .top_level_selection(w)
            .into_iter()
            .map(|id| {
                let ls = w.transform(id).map_or(Vector3::ONE, |t| t.local_scale);
                (id, w.position(id), w.rotation(id), ls)
            })
            .collect();
        // The active object first: it's the one a resize applies to.
        if let Some(i) = objects.iter().position(|o| o.0 == active) {
            objects.swap(0, i);
        }
        let start_hit = ray_plane(&proj.ray(origin), geom.center, geom.n).unwrap_or(geom.center);
        st.scene_ui.drag = Some(Drag {
            handle: h,
            tool: Tool::Rect,
            start_mouse: origin,
            center: geom.center,
            axes: [geom.u, geom.v, geom.n],
            size: 1.0,
            start_s: 0.0,
            start_hit,
            tangent: Vec2::X,
            radius_px: 0.0,
            objects,
            factor: 1.0,
            rect: Some(geom),
        });
    }

    let mut live = geom;
    if let Some(d) = st.scene_ui.drag.as_ref() {
        if !primary_down {
            st.scene_ui.drag = None;
        } else if let Some(m) = mouse
            && let Some(hit) = ray_plane(&proj.ray(m), geom.center, geom.n)
            && let Handle3::Rect(su, sv) = d.handle
        {
            let w = &mut engine.world;
            let mut delta = hit - d.start_hit;
            let (du, dv) = (delta.dot(geom.u), delta.dot(geom.v));
            if su == 0 && sv == 0 {
                if mods.command {
                    let step = st.prefs.move_snap.max(1e-4);
                    let snap = |x: f32| (x / step).round() * step;
                    delta = geom.u * snap(du) + geom.v * snap(dv);
                }
                for (id, p0, _, _) in &d.objects {
                    w.set_position(*id, *p0 + delta);
                }
                live.center = geom.center + delta;
            } else if let Some(&(id, p0, _, s0)) = d.objects.first() {
                let r = geom.resize(p0, s0, (su, sv), (du, dv), mods.alt, mods.shift);
                w.set_position(id, r.position);
                if let Some(t) = w.transform_mut(id) {
                    t.local_scale = r.scale;
                }
                live = r.rect;
            }
        }
    }

    // Draw.
    let dragging = st.scene_ui.drag.as_ref().map(|d| d.handle);
    let hovered = dragging.or_else(|| {
        if busy {
            None
        } else {
            mouse
                .filter(|m| proj.rect.contains(*m))
                .and_then(|m| live.hit(proj, m))
        }
    });
    let corners = [(1, 1), (-1, 1), (-1, -1), (1, -1)];
    let pts = corners.map(|(su, sv)| proj.project(live.point(su as f32, sv as f32)));
    let edge_col = Color32::from_gray(230);
    for (e, i) in [(1, 2), (2, 3), (3, 0), (0, 1)].into_iter().enumerate() {
        if let (Some(a), Some(b)) = (pts[i.0], pts[i.1]) {
            let h = [
                Handle3::Rect(-1, 0),
                Handle3::Rect(0, -1),
                Handle3::Rect(1, 0),
                Handle3::Rect(0, 1),
            ][e];
            let col = if hovered == Some(h) { HOVER } else { edge_col };
            painter.line_segment([a, b], Stroke::new(1.5, col));
        }
    }
    for (k, (su, sv)) in corners.into_iter().enumerate() {
        if let Some(p) = pts[k] {
            let col = if hovered == Some(Handle3::Rect(su, sv)) {
                HOVER
            } else {
                Color32::from_rgb(58, 122, 248)
            };
            painter.circle_filled(p, 4.5, col);
            painter.circle_stroke(p, 4.5, Stroke::new(1.0, Color32::WHITE));
        }
    }
    if let Some(c) = proj.project(live.center) {
        painter.circle_stroke(c, 4.0, Stroke::new(1.0, edge_col));
    }
    hovered.is_some()
}

// ---- orientation gizmo --------------------------------------------------------------------

fn orientation_gizmo(st: &mut EditorState, ui: &egui::Ui, rect: egui::Rect) {
    let center = rect.right_top() + vec2(-48.0, 52.0);
    let inv = st.scene_cam.rotation().inverse();
    let mut ends: Vec<(f32, Pos2, usize, bool)> = Vec::new();
    for i in 0..3 {
        let mut axis = Vector3::ZERO;
        axis[i] = 1.0;
        for positive in [true, false] {
            let a = if positive { axis } else { -axis };
            let v = inv * a;
            ends.push((v.z, center + vec2(v.x, -v.y) * 28.0, i, positive));
        }
    }
    // Far ends first.
    ends.sort_by(|a, b| b.0.total_cmp(&a.0));
    let painter = ui.painter_at(rect);
    painter.circle_filled(center, 40.0, Color32::from_black_alpha(40));
    let mouse = ui.input(|i| i.pointer.hover_pos());
    let clicked = ui.input(|i| i.pointer.primary_clicked());
    let mut chosen = None;
    for (_, pos, i, positive) in &ends {
        let hovered = mouse.is_some_and(|m| m.distance(*pos) < 9.0);
        let col = if !positive {
            Color32::from_gray(170)
        } else if hovered {
            HOVER
        } else {
            AXIS_COLORS[*i]
        };
        if *positive {
            painter.line_segment([center, *pos], Stroke::new(2.0, col));
        }
        painter.circle_filled(*pos, if *positive { 7.0 } else { 5.0 }, col);
        if *positive {
            painter.text(
                *pos,
                egui::Align2::CENTER_CENTER,
                ["x", "y", "z"][*i],
                egui::FontId::proportional(10.0),
                Color32::BLACK,
            );
        }
        if hovered && clicked {
            chosen = Some((*i, *positive));
        }
    }
    painter.rect_filled(
        egui::Rect::from_center_size(center, vec2(9.0, 9.0)),
        1.0,
        Color32::from_gray(200),
    );
    let label = if st.scene_cam.orthographic {
        "Iso"
    } else {
        "Persp"
    };
    // Below the disc on a dark pill, so it reads on a bright sky too.
    let galley = painter.layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(11.0),
        Color32::from_gray(225),
    );
    let pill = egui::Rect::from_center_size(
        center + vec2(0.0, 40.0 + 4.0 + galley.size().y / 2.0),
        galley.size() + vec2(10.0, 2.0),
    );
    painter.rect_filled(pill, 3.0, Color32::from_black_alpha(110));
    painter.galley(
        pill.center() - galley.size() / 2.0,
        galley,
        Color32::from_gray(225),
    );
    if clicked && mouse.is_some_and(|m| m.distance(center) < 6.0) {
        st.scene_cam.orthographic = !st.scene_cam.orthographic;
    }
    if let Some((i, positive)) = chosen {
        // View from that axis toward the pivot.
        let mut axis = Vector3::ZERO;
        axis[i] = if positive { 1.0 } else { -1.0 };
        let cam = &mut st.scene_cam;
        let pos = cam.pivot + axis * cam.distance;
        let target = cam.pivot;
        if i == 1 {
            cam.pitch = if positive { 89.9 } else { -89.9 };
            cam.yaw = 0.0;
        } else {
            cam.look_at(pos, target);
        }
    }
}

// ---- rendering ----------------------------------------------------------------------------

pub fn render(st: &mut EditorState, engine: &mut Engine) {
    if st.scene_view.rect.is_none() {
        return;
    }
    let Some(view) = st.scene_view.view() else {
        return;
    };
    let [w, h] = st.scene_view.size;
    let cam = st.scene_cam.camera_view();
    let mut rf = extract::extract(&engine.world, Some(vec![cam]));
    rf.debug_view = st.scene_ui.draw_mode;
    apply_view_options(&st.scene_ui, &mut rf, st.scene_cam.forward());
    let hidden = st.visibility.hidden_bits(&engine.world);
    if !hidden.is_empty() {
        rf.draws.retain(|d| !hidden.contains(&d.id));
    }
    if st.scene_ui.grid {
        add_grid(&mut rf, &st.scene_cam, st.scene_ui.mode_2d);
    }
    if st.scene_ui.gizmos {
        for (a, b, c) in engine.world.collect_gizmos().lines {
            rf.line(a, b, c);
        }
        oxide_engine::navigation::draw_overlay(
            &mut engine.world,
            &mut rf,
            &st.nav_display,
            &st.selection,
        );
    }
    let assets = engine.world.resource::<AssetServer>();
    engine
        .renderer
        .render(&engine.gpu, assets, &rf, view, VIEW_FORMAT, w, h);
}

/// Scene view lighting and effects toggles, applied to the extracted frame.
pub fn apply_view_options(
    o: &SceneViewState,
    rf: &mut oxide_render::RenderFrame,
    cam_forward: Vector3,
) {
    use oxide_render::{ClearMode, SunLight};
    if !o.lighting {
        // Unity: a directional light shining from the camera, no scene lights or baked light.
        rf.lights.clear();
        rf.lightmaps.clear();
        rf.sun = Some(SunLight {
            direction: cam_forward,
            color: [1.0, 1.0, 1.0],
            shadows: None,
        });
        for d in &mut rf.draws {
            d.lightmap = None;
            d.light_probe = None;
        }
    }
    let fx = &o.effects;
    if !fx.on(fx.skybox) {
        for c in &mut rf.cameras {
            if matches!(c.clear, ClearMode::Sky) {
                c.clear = ClearMode::Color(Color::rgb(0.278, 0.278, 0.278).linear());
            }
        }
    }
    if !fx.on(fx.fog) {
        rf.environment.fog = None;
    }
    if !fx.on(fx.particles) {
        rf.particles.clear();
    }
    if !fx.on(fx.post_processing) {
        rf.environment.post.enabled = false;
    }
}

fn add_grid(rf: &mut oxide_render::RenderFrame, cam: &crate::SceneCamera, mode_2d: bool) {
    let pos = cam.position();
    if mode_2d {
        // XY plane at z = 0, sized to the orthographic view.
        let size = cam.distance * (cam.fov * 0.5).to_radians().tan();
        let step = 10f32.powf((size / 15.0).log10().floor().max(-1.0));
        let n = 60;
        let span = n as f32 * step;
        let (cx, cy) = ((pos.x / step).round() * step, (pos.y / step).round() * step);
        for i in -n..=n {
            let x = cx + i as f32 * step;
            let y = cy + i as f32 * step;
            let major = |c: f32| ((c / step).round() as i64) % 10 == 0;
            let col = |m: bool| Color::new(0.6, 0.6, 0.6, if m { 0.45 } else { 0.22 });
            rf.line(
                Vector3::new(x, cy - span, 0.0),
                Vector3::new(x, cy + span, 0.0),
                col(major(x)),
            );
            rf.line(
                Vector3::new(cx - span, y, 0.0),
                Vector3::new(cx + span, y, 0.0),
                col(major(y)),
            );
        }
        return;
    }
    let height = pos.y.abs().max(cam.distance * 0.3).max(1.0);
    let step = 10f32.powf((height / 15.0).log10().floor().max(0.0));
    let n = 60;
    let span = n as f32 * step;
    let cx = (pos.x / step).round() * step;
    let cz = (pos.z / step).round() * step;
    for i in -n..=n {
        let major = ((cx / step).round() as i64 + i as i64) % 10 == 0;
        let a = if major { 0.45 } else { 0.22 };
        let col = Color::new(0.6, 0.6, 0.6, a);
        let x = cx + i as f32 * step;
        rf.line(
            Vector3::new(x, 0.0, cz - span),
            Vector3::new(x, 0.0, cz + span),
            col,
        );
        let major = ((cz / step).round() as i64 + i as i64) % 10 == 0;
        let a = if major { 0.45 } else { 0.22 };
        let col = Color::new(0.6, 0.6, 0.6, a);
        let z = cz + i as f32 * step;
        rf.line(
            Vector3::new(cx - span, 0.0, z),
            Vector3::new(cx + span, 0.0, z),
            col,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_triangle_and_box() {
        let o = Vector3::new(0.2, 0.2, -5.0);
        let d = Vector3::Z;
        let t = ray_triangle(o, d, Vector3::ZERO, Vector3::X, Vector3::Y).unwrap();
        assert!((t - 5.0).abs() < 1e-5);
        assert!(
            ray_triangle(
                Vector3::new(2.0, 2.0, -5.0),
                d,
                Vector3::ZERO,
                Vector3::X,
                Vector3::Y
            )
            .is_none()
        );
        let t = ray_box(
            Vector3::splat(-1.0),
            Vector3::splat(1.0),
            Vector3::new(0.0, 0.0, -5.0),
            d * 2.0,
        )
        .unwrap();
        assert!((t - 2.0).abs() < 1e-5, "unnormalized direction scales t");
    }

    #[test]
    fn axis_closest_point() {
        let ray = Ray::new(Vector3::new(3.0, 5.0, -10.0), Vector3::Z);
        let s = closest_on_axis(&ray, Vector3::ZERO, Vector3::X).unwrap();
        assert!((s - 3.0).abs() < 1e-4);
    }

    #[test]
    fn pick_cube() {
        let mut w = World::new();
        oxide_engine::init_world(&mut w, std::env::temp_dir());
        w.run_scripts = false;
        let id = oxide_engine::objects::create_primitive(&mut w, oxide_assets::PrimitiveType::Cube);
        w.set_position(id, Vector3::new(0.0, 0.0, 5.0));
        let hit = pick(&w, &Ray::new(Vector3::ZERO, Vector3::Z)).unwrap();
        assert_eq!(hit.0, id);
        assert!((hit.1 - 4.5).abs() < 1e-3, "{}", hit.1);
        assert!(pick(&w, &Ray::new(Vector3::ZERO, Vector3::X)).is_none());
        // Unpickable objects are skipped.
        assert!(pick_where(&w, &Ray::new(Vector3::ZERO, Vector3::Z), |o| o != id).is_none());
    }

    #[test]
    fn rect_faces_the_camera() {
        let mut w = World::new();
        oxide_engine::init_world(&mut w, std::env::temp_dir());
        w.run_scripts = false;
        let id = oxide_engine::objects::create_primitive(&mut w, oxide_assets::PrimitiveType::Cube);
        w.set_position(id, Vector3::new(1.0, 2.0, 3.0));
        w.transform_mut(id).unwrap().local_scale = Vector3::new(2.0, 1.0, 4.0);
        // Looking along +Z: the rect is the X/Y face.
        let r = RectGeom::of(&w, id, Vector3::Z).unwrap();
        assert_eq!((r.iu, r.iv), (0, 1));
        assert!((r.hu - 1.0).abs() < 1e-4 && (r.hv - 0.5).abs() < 1e-4);
        assert!((r.center - Vector3::new(1.0, 2.0, 3.0)).length() < 1e-4);
        // Looking down: the X/Z face (u = Z, v = X).
        let r = RectGeom::of(&w, id, -Vector3::Y).unwrap();
        assert_eq!((r.iu, r.iv), (2, 0));
        assert!((r.hu - 2.0).abs() < 1e-4 && (r.hv - 1.0).abs() < 1e-4);

        // Drag the +X edge of the X/Y rect 1 unit right: width 2 -> 3, left edge stays.
        let r = RectGeom::of(&w, id, Vector3::Z).unwrap();
        let p0 = Vector3::new(1.0, 2.0, 3.0);
        let s0 = Vector3::new(2.0, 1.0, 4.0);
        let near = |a: Vector3, b: Vector3| (a - b).length() < 1e-4;
        let out = r.resize(p0, s0, (1, 0), (1.0, 0.0), false, false);
        assert!(near(out.scale, Vector3::new(3.0, 1.0, 4.0)));
        assert!(near(out.position, Vector3::new(1.5, 2.0, 3.0)));
        assert!((out.rect.point(-1.0, 0.0).x - r.point(-1.0, 0.0).x).abs() < 1e-4);
        // Alt: around the center, both sides move.
        let out = r.resize(p0, s0, (1, 0), (1.0, 0.0), true, false);
        assert!(near(out.scale, Vector3::new(4.0, 1.0, 4.0)) && near(out.position, p0));
        // Shift on a corner keeps the aspect ratio.
        let out = r.resize(p0, s0, (1, 1), (1.0, 0.0), false, true);
        assert!(near(out.scale, Vector3::new(3.0, 1.5, 4.0)));
        // A pivot off the bounds center: the fixed edge still stays put.
        let off = r.resize(p0 - Vector3::X, s0, (1, 0), (1.0, 0.0), false, false);
        assert!(near(off.position, Vector3::new(0.0, 2.0, 3.0)));
    }
}
