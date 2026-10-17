//! Unity's other visual-effect components: URP's Decal Projector, the Line Renderer and
//! Trail Renderer (camera-facing or flat ribbons with width curves, colour gradients,
//! rounded corners and end caps, texture modes) and the Billboard Renderer.
//! Reference: <https://docs.unity3d.com/Manual/visual-effects-lines-trails-billboards.html>

use oxide_assets::{Asset, AssetId, AssetServer, Handle, Material, Mesh, SurfaceType, Texture2D};
use oxide_core::{AnimationCurve, Gradient, Reflect, Time};
use oxide_math::{Color, Matrix4x4, Vector2, Vector3, Vector4};
use oxide_render::{
    DecalDraw, DrawItem, ParticleBatch, ParticleBlend, ParticleInstance, ParticleMode,
};
use oxide_scene::{Component, GameObjectId, World};
use std::collections::HashMap;

/// The material lines and trails use when they have none (unlit, transparent, white).
pub const DEFAULT_LINE_MATERIAL: &str = "builtin:Default-Line";

// ---- Decal Projector -----------------------------------------------------------------------

/// URP `DecalProjector`: projects a material's Base Map and Base Color onto the surfaces
/// inside its box, along its local +Z.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct DecalProjector {
    pub material: Option<Handle<Material>>,
    /// Width, height and projection depth.
    pub size: Vector3,
    /// Box centre relative to the object (URP default: half the depth forward).
    pub pivot: Vector3,
    pub uv_scale: Vector2,
    pub uv_offset: Vector2,
    /// Opacity (0..1).
    pub fade_factor: f32,
    /// Beyond this distance from the camera the decal isn't drawn.
    pub draw_distance: f32,
    /// Fraction of the draw distance where it starts fading (0..1).
    pub fade_scale: f32,
    /// Angle (degrees, 0..180) between the surface and the projector where fading starts
    /// and ends; equal values turn angle fading off.
    pub start_angle_fade: f32,
    pub end_angle_fade: f32,
}

impl Default for DecalProjector {
    fn default() -> Self {
        Self {
            material: None,
            size: Vector3::ONE,
            pivot: Vector3::new(0.0, 0.0, 0.5),
            uv_scale: Vector2::ONE,
            uv_offset: Vector2::ZERO,
            fade_factor: 1.0,
            draw_distance: 1000.0,
            fade_scale: 0.9,
            start_angle_fade: 180.0,
            end_angle_fade: 180.0,
        }
    }
}

impl Component for DecalProjector {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Decal Projectors as decal draws for a camera at `camera`.
pub fn extract_decals(world: &World, camera: Vector3, out: &mut Vec<DecalDraw>) {
    let Some(assets) = world.try_resource::<AssetServer>() else {
        return;
    };
    for (id, d) in world.query::<DecalProjector>() {
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let pos = world.position(id);
        let dist = pos.distance(camera);
        if dist > d.draw_distance {
            continue;
        }
        let start = d.draw_distance * d.fade_scale.clamp(0.0, 1.0);
        let distance_fade = if dist > start {
            1.0 - (dist - start) / (d.draw_distance - start).max(1e-4)
        } else {
            1.0
        };
        let mat = d.material.as_ref().and_then(|h| assets.materials.get(h));
        let base = mat.map_or(Color::WHITE, |m| m.base_color).linear();
        let texture = mat.and_then(|m| m.base_map.as_ref()).map(|h| h.id());
        let alpha =
            mat.map_or(1.0, |m| m.base_color.a) * d.fade_factor.clamp(0.0, 1.0) * distance_fade;
        let to_world = world.local_to_world(id)
            * Matrix4x4::from_translation(d.pivot)
            * Matrix4x4::from_scale(d.size.max(Vector3::splat(1e-4)));
        let (a, b) = (
            d.start_angle_fade.clamp(0.0, 180.0),
            d.end_angle_fade.clamp(0.0, 180.0),
        );
        out.push(DecalDraw {
            to_world,
            texture,
            color: [base.r, base.g, base.b, alpha],
            uv: [d.uv_scale.x, d.uv_scale.y, d.uv_offset.x, d.uv_offset.y],
            angle_fade: [a.min(b), a.max(b)],
        });
    }
}

// ---- ribbons ------------------------------------------------------------------------------

/// Unity `LineAlignment`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum LineAlignment {
    /// Faces the camera.
    #[default]
    View,
    /// Lies flat, facing the object's Z axis.
    TransformZ,
}

/// Unity `LineTextureMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum LineTextureMode {
    /// Once over the whole length.
    #[default]
    Stretch,
    /// Repeats every world unit (times `texture_scale.x`).
    Tile,
    /// Once over the whole length, spaced by vertex.
    DistributePerSegment,
    /// Once per segment.
    RepeatPerSegment,
}

/// A point of a ribbon, in world space.
#[derive(Clone, Copy, Debug)]
pub struct RibbonPoint {
    pub position: Vector3,
    pub width: f32,
    /// Linear RGBA.
    pub color: Vector4,
}

/// How a ribbon is shaped.
#[derive(Clone, Copy, Debug)]
pub struct RibbonStyle {
    /// View: the camera position; TransformZ: the plane's normal.
    pub facing: RibbonFacing,
    pub looped: bool,
    pub corner_vertices: u32,
    pub end_cap_vertices: u32,
    pub texture_mode: LineTextureMode,
    pub texture_scale: Vector2,
}

#[derive(Clone, Copy, Debug)]
pub enum RibbonFacing {
    Camera(Vector3),
    Normal(Vector3),
}

/// Builds a ribbon mesh through `points`: joined quads (mitred at corners), optional
/// rounded corners and end caps, UVs by texture mode and vertex colours.
pub fn ribbon_mesh(points: &[RibbonPoint], style: &RibbonStyle) -> Mesh {
    let mut pts: Vec<RibbonPoint> = points.to_vec();
    pts.dedup_by(|b, a| a.position.distance_squared(b.position) < 1e-10);
    let n = pts.len();
    let mut mesh = Mesh {
        name: "Ribbon".into(),
        ..Default::default()
    };
    if n < 2 {
        mesh.finish();
        return mesh;
    }
    let looped = style.looped && n > 2;
    let facing_at = |p: Vector3| match style.facing {
        RibbonFacing::Camera(c) => (c - p).normalize_or(Vector3::Z),
        RibbonFacing::Normal(nrm) => nrm.normalize_or(Vector3::Z),
    };
    // Distance along the line for UVs.
    let mut dist = vec![0.0f32; n];
    for i in 1..n {
        dist[i] = dist[i - 1] + pts[i].position.distance(pts[i - 1].position);
    }
    let total = dist[n - 1]
        + if looped {
            pts[n - 1].position.distance(pts[0].position)
        } else {
            0.0
        };
    let u_of = |i: usize, d: f32| -> f32 {
        match style.texture_mode {
            LineTextureMode::Stretch => d / total.max(1e-6),
            LineTextureMode::Tile => d * style.texture_scale.x,
            LineTextureMode::DistributePerSegment => {
                i as f32 / (if looped { n } else { n - 1 }) as f32
            }
            LineTextureMode::RepeatPerSegment => i as f32,
        }
    };
    let dir = |a: Vector3, b: Vector3| (b - a).normalize_or_zero();
    // Each point's left/right vertices (with extra fan vertices at rounded corners).
    let mut verts: Vec<Vector3> = Vec::new();
    let mut uvs: Vec<Vector2> = Vec::new();
    let mut cols: Vec<Vector4> = Vec::new();
    let mut norms: Vec<Vector3> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    // Pairs (left, right) in order along the ribbon.
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let count = if looped { n + 1 } else { n };
    for k in 0..count {
        let i = k % n;
        let p = pts[i];
        let prev = if i > 0 {
            Some(pts[i - 1].position)
        } else if looped {
            Some(pts[n - 1].position)
        } else {
            None
        };
        let next = if i + 1 < n {
            Some(pts[i + 1].position)
        } else if looped {
            Some(pts[0].position)
        } else {
            None
        };
        let d_in = prev.map(|q| dir(q, p.position));
        let d_out = next.map(|q| dir(p.position, q));
        let face = facing_at(p.position);
        let side_of = |t: Vector3| t.cross(face).normalize_or_zero();
        let u = u_of(k, if k == n { total } else { dist[i] });
        let half = p.width * 0.5;
        let mut push_pair = |side: Vector3, u: f32| {
            let base = verts.len() as u32;
            verts.push(p.position - side * half);
            verts.push(p.position + side * half);
            uvs.push(Vector2::new(u, 1.0));
            uvs.push(Vector2::new(u, 0.0));
            cols.push(p.color);
            cols.push(p.color);
            norms.push(face);
            norms.push(face);
            pairs.push((base, base + 1));
        };
        match (d_in, d_out) {
            (Some(a), Some(b)) if style.corner_vertices > 0 && a.dot(b) < 0.9999 => {
                // Rounded corner: sweep the side from the incoming to the outgoing segment.
                let steps = style.corner_vertices + 1;
                for s in 0..=steps {
                    let t = s as f32 / steps as f32;
                    push_pair(side_of(a.lerp(b, t).normalize_or(b)), u);
                }
            }
            (Some(a), Some(b)) => {
                // Mitre: the averaged side, lengthened to keep the width (limited).
                let t = (a + b).normalize_or(b);
                let side = side_of(t);
                let k = side.dot(side_of(b)).abs().max(0.35);
                push_pair(side / k, u);
            }
            (Some(a), None) => push_pair(side_of(a), u),
            (None, Some(b)) => push_pair(side_of(b), u),
            (None, None) => {}
        }
    }
    for w in pairs.windows(2) {
        let ((l0, r0), (l1, r1)) = (w[0], w[1]);
        idx.extend_from_slice(&[l0, l1, r0, r0, l1, r1]);
    }
    // Round end caps: fans around the first and last points.
    if !looped && style.end_cap_vertices > 0 {
        for (end, sign) in [(0usize, -1.0f32), (n - 1, 1.0)] {
            let p = pts[end];
            let along = if end == 0 {
                dir(p.position, pts[1].position)
            } else {
                dir(pts[n - 2].position, p.position)
            } * sign;
            let face = facing_at(p.position);
            let side = along.cross(face).normalize_or_zero();
            let center = verts.len() as u32;
            let u = if end == 0 { 0.0 } else { u_of(n - 1, total) };
            verts.push(p.position);
            uvs.push(Vector2::new(u, 0.5));
            cols.push(p.color);
            norms.push(face);
            let steps = style.end_cap_vertices + 1;
            for s in 0..=steps {
                let a = std::f32::consts::PI * s as f32 / steps as f32;
                let off = side * a.cos() + along * a.sin();
                verts.push(p.position + off * p.width * 0.5);
                uvs.push(Vector2::new(u, 0.5 + 0.5 * a.cos()));
                cols.push(p.color);
                norms.push(face);
            }
            for s in 0..steps {
                idx.extend_from_slice(&[center, center + 1 + s, center + 2 + s]);
            }
        }
    }
    mesh.vertices = verts;
    mesh.uv = uvs;
    mesh.colors = cols;
    mesh.normals = norms;
    mesh.indices = idx;
    mesh.finish();
    mesh
}

fn line_color(c: Color) -> Vector4 {
    let l = c.linear();
    Vector4::new(l.r, l.g, l.b, c.a)
}

// ---- Line Renderer ------------------------------------------------------------------------

/// Unity `LineRenderer`.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct LineRenderer {
    pub positions: Vec<Vector3>,
    /// Positions are world space (else relative to the object).
    pub use_world_space: bool,
    /// Connects the last point back to the first.
    pub looped: bool,
    /// Width along the line (0 = start, 1 = end), times `width_multiplier`.
    pub width_curve: AnimationCurve,
    pub width_multiplier: f32,
    /// Colour along the line.
    pub color: Gradient,
    pub corner_vertices: u32,
    pub end_cap_vertices: u32,
    pub alignment: LineAlignment,
    pub texture_mode: LineTextureMode,
    pub texture_scale: Vector2,
    /// Empty uses the default line material (unlit, transparent).
    pub material: Option<Handle<Material>>,
}

impl Default for LineRenderer {
    fn default() -> Self {
        Self {
            positions: vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 1.0)],
            use_world_space: true,
            looped: false,
            width_curve: AnimationCurve::constant(0.0, 1.0, 1.0),
            width_multiplier: 0.1,
            color: Gradient::two(Color::WHITE, Color::WHITE),
            corner_vertices: 0,
            end_cap_vertices: 0,
            alignment: LineAlignment::View,
            texture_mode: LineTextureMode::Stretch,
            texture_scale: Vector2::ONE,
            material: None,
        }
    }
}

impl Component for LineRenderer {
    fn allow_multiple(&self) -> bool {
        false
    }
}

impl LineRenderer {
    pub fn position_count(&self) -> usize {
        self.positions.len()
    }
    pub fn set_position(&mut self, index: usize, p: Vector3) {
        if index >= self.positions.len() {
            self.positions.resize(index + 1, Vector3::ZERO);
        }
        self.positions[index] = p;
    }
    pub fn set_positions(&mut self, positions: &[Vector3]) {
        self.positions = positions.to_vec();
    }
}

// ---- Trail Renderer -----------------------------------------------------------------------

/// Unity `TrailRenderer`: a ribbon behind a moving object.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct TrailRenderer {
    /// How long a point lasts (seconds).
    pub time: f32,
    /// How far the object moves before a new point is added.
    pub min_vertex_distance: f32,
    /// Width from the head (0) to the tail (1), times `width_multiplier`.
    pub width_curve: AnimationCurve,
    pub width_multiplier: f32,
    /// Colour from the head (0) to the tail (1).
    pub color: Gradient,
    pub corner_vertices: u32,
    pub end_cap_vertices: u32,
    pub alignment: LineAlignment,
    pub texture_mode: LineTextureMode,
    pub texture_scale: Vector2,
    /// New points are added while true.
    pub emitting: bool,
    /// Destroys the object once its trail is gone.
    pub autodestruct: bool,
    pub material: Option<Handle<Material>>,
    #[reflect(skip)]
    #[doc(hidden)]
    pub points: Vec<(Vector3, f32)>,
}

impl Default for TrailRenderer {
    fn default() -> Self {
        Self {
            time: 5.0,
            min_vertex_distance: 0.1,
            width_curve: AnimationCurve::constant(0.0, 1.0, 1.0),
            width_multiplier: 1.0,
            color: Gradient::two(Color::WHITE, Color::new(1.0, 1.0, 1.0, 0.0)),
            corner_vertices: 0,
            end_cap_vertices: 0,
            alignment: LineAlignment::View,
            texture_mode: LineTextureMode::Stretch,
            texture_scale: Vector2::ONE,
            emitting: true,
            autodestruct: false,
            material: None,
            points: Vec::new(),
        }
    }
}

impl Component for TrailRenderer {
    fn allow_multiple(&self) -> bool {
        false
    }
}

impl TrailRenderer {
    /// Unity `Clear()`: removes every point.
    pub fn clear(&mut self) {
        self.points.clear();
    }
    pub fn position_count(&self) -> usize {
        self.points.len()
    }
}

// ---- Billboard Renderer -------------------------------------------------------------------

/// Unity `BillboardAsset`: an impostor image (or images for view angles around Y).
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct BillboardAsset {
    pub width: f32,
    pub height: f32,
    /// How far below the object's origin the image starts.
    pub bottom: f32,
    pub texture: Option<Handle<Texture2D>>,
    /// Texture rectangles (min x, min y, width, height; origin top left), one per view
    /// angle around Y, evenly spaced from the object's front.
    pub images: Vec<Vector4>,
    pub color: Color,
}

impl Default for BillboardAsset {
    fn default() -> Self {
        Self {
            width: 1.0,
            height: 1.0,
            bottom: 0.0,
            texture: None,
            images: vec![Vector4::new(0.0, 0.0, 1.0, 1.0)],
            color: Color::WHITE,
        }
    }
}

impl Asset for BillboardAsset {
    const TYPE_NAME: &'static str = "BillboardAsset";
}

/// Unity `BillboardRenderer`: draws a Billboard Asset turned toward the camera (upright).
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct BillboardRenderer {
    pub billboard: Option<Handle<BillboardAsset>>,
}

impl Component for BillboardRenderer {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// The image a billboard shows for a camera looking from `to_camera` (object space, XZ).
pub fn billboard_image(images: &[Vector4], to_camera: Vector3) -> Vector4 {
    if images.is_empty() {
        return Vector4::new(0.0, 0.0, 1.0, 1.0);
    }
    let angle = to_camera
        .x
        .atan2(to_camera.z)
        .rem_euclid(std::f32::consts::TAU);
    let i = ((angle / std::f32::consts::TAU * images.len() as f32).round() as usize) % images.len();
    images[i]
}

// ---- per-frame update and extraction -----------------------------------------------------

/// The camera the ribbons face (the main camera, else any).
fn camera_position(world: &World) -> Option<Vector3> {
    world
        .find_with_tag("MainCamera")
        .into_iter()
        .next()
        .or_else(|| {
            world
                .query::<oxide_scene::Camera>()
                .map(|(id, _)| id)
                .next()
        })
        .map(|id| world.position(id))
}

/// The last game frame trails recorded.
struct TrailFrame(u64);

fn mesh_path(kind: &str, id: GameObjectId) -> String {
    format!("runtime:{kind}/{}", id.to_bits())
}

/// Records trail points, then rebuilds line and trail meshes (each frame, edit mode too).
pub fn update(world: &mut World) {
    let (mut dt, now, frame) = world.try_resource::<Time>().map_or((0.0, 0.0, 0), |t| {
        (t.delta_time(), t.time(), t.frame_count())
    });
    // Only frames the game advanced (Edit mode keeps the last delta time).
    let last = world.resources.get::<TrailFrame>().map(|f| f.0);
    if last == Some(frame) {
        dt = 0.0;
    }
    world.resources.insert(TrailFrame(frame));
    let camera = camera_position(world).unwrap_or(Vector3::ZERO);
    // Trails: add points as the object moves, drop old ones.
    let mut destroy = Vec::new();
    for (id, cid) in world.query_ids::<TrailRenderer>() {
        let pos = world.position(id);
        let Some(t) = world.component_mut::<TrailRenderer>(id, cid) else {
            continue;
        };
        let had = !t.points.is_empty();
        if dt > 0.0 {
            if t.emitting
                && t.points
                    .last()
                    .is_none_or(|(p, _)| p.distance(pos) >= t.min_vertex_distance.max(1e-4))
            {
                t.points.push((pos, now));
            }
            let life = t.time.max(0.0);
            t.points.retain(|(_, born)| now - born <= life);
        }
        if t.autodestruct && had && t.points.is_empty() {
            destroy.push(id);
        }
    }
    for id in destroy {
        world.destroy(id);
    }
    // Meshes.
    let mut meshes: Vec<(String, Mesh)> = Vec::new();
    for (id, l) in world.query::<LineRenderer>() {
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let m = world.local_to_world(id);
        let n = l.positions.len();
        let points: Vec<RibbonPoint> = l
            .positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let t = if n > 1 {
                    i as f32 / (n - 1) as f32
                } else {
                    0.0
                };
                RibbonPoint {
                    position: if l.use_world_space {
                        *p
                    } else {
                        m.transform_point3(*p)
                    },
                    width: (l.width_curve.evaluate(t) * l.width_multiplier).max(0.0),
                    color: line_color(l.color.evaluate(t)),
                }
            })
            .collect();
        let style = RibbonStyle {
            facing: match l.alignment {
                LineAlignment::View => RibbonFacing::Camera(camera),
                LineAlignment::TransformZ => RibbonFacing::Normal(m.transform_vector3(Vector3::Z)),
            },
            looped: l.looped,
            corner_vertices: l.corner_vertices,
            end_cap_vertices: l.end_cap_vertices,
            texture_mode: l.texture_mode,
            texture_scale: l.texture_scale,
        };
        meshes.push((mesh_path("LineRenderer", id), ribbon_mesh(&points, &style)));
    }
    for (id, t) in world.query::<TrailRenderer>() {
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let head = world.position(id);
        // Newest first: the head is the object itself.
        let mut pts: Vec<Vector3> = t.points.iter().rev().map(|(p, _)| *p).collect();
        if t.emitting && pts.first().is_none_or(|p| p.distance(head) > 1e-4) {
            pts.insert(0, head);
        }
        let n = pts.len();
        let points: Vec<RibbonPoint> = pts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let u = if n > 1 {
                    i as f32 / (n - 1) as f32
                } else {
                    0.0
                };
                RibbonPoint {
                    position: *p,
                    width: (t.width_curve.evaluate(u) * t.width_multiplier).max(0.0),
                    color: line_color(t.color.evaluate(u)),
                }
            })
            .collect();
        let style = RibbonStyle {
            facing: match t.alignment {
                LineAlignment::View => RibbonFacing::Camera(camera),
                LineAlignment::TransformZ => {
                    RibbonFacing::Normal(world.local_to_world(id).transform_vector3(Vector3::Z))
                }
            },
            looped: false,
            corner_vertices: t.corner_vertices,
            end_cap_vertices: t.end_cap_vertices,
            texture_mode: t.texture_mode,
            texture_scale: t.texture_scale,
        };
        meshes.push((mesh_path("TrailRenderer", id), ribbon_mesh(&points, &style)));
    }
    let Some(assets) = world.try_resource_mut::<AssetServer>() else {
        return;
    };
    for (path, mesh) in meshes {
        let same = assets
            .meshes
            .handle_of(&path)
            .and_then(|h| assets.meshes.get(&h))
            .is_some_and(|old| {
                old.vertices == mesh.vertices
                    && old.colors == mesh.colors
                    && old.uv == mesh.uv
                    && old.indices == mesh.indices
            });
        if !same {
            assets.meshes.insert(path, mesh);
        }
    }
}

fn ribbon_draw(mesh: &str, material: AssetId, layer: u8) -> DrawItem {
    DrawItem {
        id: 0,
        mesh: AssetId::from_path(mesh),
        submesh: 0,
        material,
        model: Matrix4x4::IDENTITY,
        cast_shadows: false,
        shadows_only: false,
        receive_shadows: false,
        layer,
        joint_offset: None,
        occluder: false,
        occludee: false,
        reflection_probe_static: false,
        lightmap: None,
        light_probe: None,
        sorting: Default::default(),
        rendering_layers: u32::MAX,
    }
}

/// Line and Trail Renderers as mesh draws.
pub fn extract_draws(world: &World, out: &mut Vec<DrawItem>) {
    let Some(assets) = world.try_resource::<AssetServer>() else {
        return;
    };
    let material = |m: &Option<Handle<Material>>| {
        m.as_ref()
            .map_or_else(|| AssetId::from_path(DEFAULT_LINE_MATERIAL), |h| h.id())
    };
    let has_mesh = |path: &str| {
        assets
            .meshes
            .handle_of(path)
            .and_then(|h| assets.meshes.get(&h))
            .is_some_and(|m| !m.indices.is_empty())
    };
    for (id, l) in world.query::<LineRenderer>() {
        let path = mesh_path("LineRenderer", id);
        if world.get(id).is_some_and(|o| o.active_in_hierarchy()) && has_mesh(&path) {
            let layer = world.get(id).map_or(0, |o| o.layer);
            out.push(ribbon_draw(&path, material(&l.material), layer));
        }
    }
    for (id, t) in world.query::<TrailRenderer>() {
        let path = mesh_path("TrailRenderer", id);
        if world.get(id).is_some_and(|o| o.active_in_hierarchy()) && has_mesh(&path) {
            let layer = world.get(id).map_or(0, |o| o.layer);
            out.push(ribbon_draw(&path, material(&t.material), layer));
        }
    }
}

/// Billboard Renderers as upright particle quads facing the camera at `camera`.
pub fn extract_billboards(world: &World, camera: Vector3, out: &mut Vec<ParticleBatch>) {
    let Some(assets) = world.try_resource::<AssetServer>() else {
        return;
    };
    let mut batches: HashMap<Option<AssetId>, Vec<ParticleInstance>> = HashMap::new();
    for (id, b) in world.query::<BillboardRenderer>() {
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let Some(asset) = b.billboard.as_ref().and_then(|h| assets.data.get(h)) else {
            continue;
        };
        let (pos, rot, scale) = (
            world.position(id),
            world.rotation(id),
            world.lossy_scale(id),
        );
        let local_cam = rot.inverse() * (camera - pos);
        let uv = billboard_image(&asset.images, local_cam);
        let (w, h) = (asset.width * scale.x, asset.height * scale.y);
        let center = pos + Vector3::Y * (h * 0.5 - asset.bottom * scale.y);
        let c = asset.color.linear();
        batches
            .entry(asset.texture.as_ref().map(|t| t.id()))
            .or_default()
            .push(
                ParticleInstance::billboard(
                    center.to_array(),
                    w,
                    [c.r, c.g, c.b, asset.color.a],
                    0.0,
                )
                .with_mode(ParticleMode::Vertical, [0.0; 3])
                .with_uv(uv.to_array())
                .with_aspect(h / w.max(1e-4)),
            );
    }
    for (texture, particles) in batches {
        out.push(ParticleBatch {
            texture,
            blend: ParticleBlend::Alpha,
            particles,
        });
    }
}

/// The default line material (unlit, transparent, white, double sided).
pub fn register_default_material(assets: &mut AssetServer) {
    if assets.materials.handle_of(DEFAULT_LINE_MATERIAL).is_none() {
        let mut m = Material::unlit("Default-Line", Color::WHITE);
        m.surface_type = SurfaceType::Transparent;
        m.double_sided = true;
        assets.materials.insert(DEFAULT_LINE_MATERIAL, m);
    }
}
