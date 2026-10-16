//! What the renderer draws: a plain-data snapshot of one frame, extracted from the scene
//! by the engine (cameras, lights, draw items, environment, debug lines).

use oxide_assets::AssetId;
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProjectionKind {
    /// Vertical field of view in degrees.
    Perspective { fov: f32 },
    /// Half the vertical view height in world units.
    Orthographic { size: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClearMode {
    Sky,
    /// Linear color.
    Color(Color),
    DepthOnly,
    Nothing,
}

#[derive(Clone, Debug)]
pub struct CameraView {
    pub position: Vector3,
    pub rotation: Quaternion,
    pub projection: ProjectionKind,
    pub near: f32,
    pub far: f32,
    /// Normalized `x, y, width, height` with a bottom-left origin (Unity viewport rect).
    pub viewport: [f32; 4],
    pub clear: ClearMode,
    pub culling_mask: u32,
    /// Skip draws hidden behind occluders (Unity `Camera.useOcclusionCulling`).
    pub occlusion_culling: bool,
}

impl CameraView {
    pub fn view(&self) -> Matrix4x4 {
        Matrix4x4::look_to(
            self.position,
            self.rotation * Vector3::Z,
            self.rotation * Vector3::Y,
        )
    }

    /// Reversed-Z projection (near → 1, far → 0) for precision in large worlds.
    pub fn projection(&self, aspect: f32) -> Matrix4x4 {
        match self.projection {
            ProjectionKind::Perspective { fov } => Matrix4x4::perspective(
                fov.clamp(1.0, 179.0).to_radians(),
                aspect,
                self.far,
                self.near,
            ),
            ProjectionKind::Orthographic { size } => {
                let w = size * aspect;
                Matrix4x4::ortho(-w, w, -size, size, self.far, self.near)
            }
        }
    }

    pub fn view_projection(&self, aspect: f32) -> Matrix4x4 {
        self.projection(aspect) * self.view()
    }

    /// World-space ray through a viewport point (0..1, bottom-left origin).
    pub fn viewport_point_to_ray(&self, x: f32, y: f32, aspect: f32) -> oxide_math::Ray {
        let inv = self.view_projection(aspect).inverse();
        let ndc = oxide_math::Vector2::new(x * 2.0 - 1.0, y * 2.0 - 1.0);
        let near = inv.project_point3(ndc.extend(1.0));
        let far = inv.project_point3(ndc.extend(0.0001));
        oxide_math::Ray::new(near, (far - near).normalize_or(Vector3::Z))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ShadowSettings {
    pub soft: bool,
    pub strength: f32,
    pub bias: f32,
    pub normal_bias: f32,
}

#[derive(Clone, Debug)]
pub struct SunLight {
    /// Direction the light travels (the light's forward vector).
    pub direction: Vector3,
    /// Linear color × intensity.
    pub color: [f32; 3],
    pub shadows: Option<ShadowSettings>,
}

#[derive(Clone, Debug)]
pub struct PunctualLight {
    pub position: Vector3,
    pub range: f32,
    /// Linear color × intensity.
    pub color: [f32; 3],
    /// `(direction, outer angle°, inner angle°)` for spot lights.
    pub spot: Option<(Vector3, f32, f32)>,
}

#[derive(Clone, Debug)]
pub struct DrawItem {
    pub mesh: AssetId,
    pub submesh: u32,
    pub material: AssetId,
    /// Local-to-world (identity for skinned draws, whose joints are in world space).
    pub model: Matrix4x4,
    pub cast_shadows: bool,
    /// Only casts shadows (Unity "Shadows Only").
    pub shadows_only: bool,
    pub receive_shadows: bool,
    pub layer: u8,
    /// Offset into [`RenderFrame::joints`] for skinned meshes.
    pub joint_offset: Option<u32>,
    /// Hides what is behind it (Occluder Static).
    pub occluder: bool,
    /// May be hidden by occluders.
    pub occludee: bool,
}

/// One terrain chunk drawn with the terrain (splat) shader.
#[derive(Clone, Debug)]
pub struct TerrainDraw {
    /// The `TerrainData` asset (layers, splat map).
    pub data: AssetId,
    pub mesh: AssetId,
    pub model: Matrix4x4,
    pub receive_shadows: bool,
    /// Simplified mesh that never rises above the real ground, used as an occluder.
    pub occluder: Option<AssetId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParticleBlend {
    Alpha,
    Additive,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleInstance {
    pub position: [f32; 3],
    pub size: f32,
    /// Linear RGB (may exceed 1 for HDR glow) and alpha.
    pub color: [f32; 4],
    pub rotation: f32,
    pub _pad: [f32; 3],
}

/// Particles of one system (one texture and blend mode).
#[derive(Clone, Debug)]
pub struct ParticleBatch {
    pub texture: Option<AssetId>,
    pub blend: ParticleBlend,
    pub particles: Vec<ParticleInstance>,
}

#[derive(Clone, Copy, Debug)]
pub enum FogKind {
    Linear { start: f32, end: f32 },
    Exponential { density: f32 },
    ExponentialSquared { density: f32 },
}

#[derive(Clone, Debug)]
pub struct Environment {
    /// sRGB inputs, converted in the renderer.
    pub sky_tint: Color,
    pub ground_color: Color,
    pub sky_exposure: f32,
    pub sun_size: f32,
    pub ambient_intensity: f32,
    pub reflection_intensity: f32,
    pub fog: Option<(FogKind, Color)>,
    pub exposure: f32,
    /// `(intensity, threshold)`.
    pub bloom: Option<(f32, f32)>,
    pub vignette: f32,
    pub shadow_distance: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sky_tint: Color::rgb(0.5, 0.5, 0.5),
            ground_color: Color::rgb(0.369, 0.349, 0.341),
            sky_exposure: 1.3,
            sun_size: 0.04,
            ambient_intensity: 1.0,
            reflection_intensity: 1.0,
            fog: None,
            exposure: 1.0,
            bloom: Some((0.4, 1.0)),
            vignette: 0.15,
            shadow_distance: 150.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    /// Linear RGBA.
    pub color: [f32; 4],
}

#[derive(Clone, Debug, Default)]
pub struct RenderFrame {
    /// Rendered in order; later cameras draw on top (sorted by Unity `depth` beforehand).
    pub cameras: Vec<CameraView>,
    pub sun: Option<SunLight>,
    pub lights: Vec<PunctualLight>,
    pub draws: Vec<DrawItem>,
    pub terrain_draws: Vec<TerrainDraw>,
    /// Boxes that occlude like solid geometry (closed Occlusion Portals): a unit cube
    /// centered on the origin under each matrix.
    pub occluder_boxes: Vec<Matrix4x4>,
    pub particles: Vec<ParticleBatch>,

    /// World-space skinning matrices (bone local-to-world × bind pose).
    pub joints: Vec<Matrix4x4>,
    pub environment: Environment,
    /// Pairs of vertices (line list).
    pub lines: Vec<LineVertex>,
    pub time: f32,
}

impl RenderFrame {
    pub fn line(&mut self, a: Vector3, b: Vector3, color: Color) {
        let c = color.linear();
        let c = [c.r, c.g, c.b, c.a];
        self.lines.push(LineVertex {
            position: a.to_array(),
            color: c,
        });
        self.lines.push(LineVertex {
            position: b.to_array(),
            color: c,
        });
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct RenderStats {
    pub draw_calls: u32,
    pub instances: u32,
    pub triangles: u64,
    pub culled: u32,
    pub shadow_draw_calls: u32,
    pub cameras: u32,
    /// Draws and terrain chunks skipped because occluders hide them.
    pub occlusion_culled: u32,
    pub occluders: u32,
    pub occluder_triangles: u32,
    /// CPU time spent rasterizing occluders and testing bounds.
    pub occlusion_ms: f32,
}
