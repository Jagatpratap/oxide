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

/// Post-process anti-aliasing of a camera (Unity URP `AntialiasingMode`). MSAA is separate:
/// [`Environment::msaa_samples`] and [`AntiAliasingSettings::allow_msaa`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AntiAliasing {
    #[default]
    None,
    /// Fast Approximate Anti-aliasing: one edge-smoothing pass after tonemapping.
    Fxaa,
    /// Temporal Anti-aliasing: jittered frames accumulated with motion vectors. Turns
    /// MSAA off for the camera, like URP.
    Taa,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AntiAliasingSettings {
    pub mode: AntiAliasing,
    /// Unity `Camera.allowMSAA`.
    pub allow_msaa: bool,
    /// How much of the history is kept each frame (URP "Base Blend Factor").
    pub taa_base_blend_factor: f32,
    /// Size of the sub-pixel jitter (URP "Jitter Scale").
    pub taa_jitter_scale: f32,
    /// How tightly history is clamped to the current neighbourhood (URP "Variance Clamp
    /// Scale"): lower = less ghosting, more flicker.
    pub taa_variance_clamp_scale: f32,
    /// Identifies the view whose TAA history this is (the camera object; views of the same
    /// size with different ids keep separate histories).
    pub history_id: u64,
}

impl Default for AntiAliasingSettings {
    fn default() -> Self {
        Self {
            mode: AntiAliasing::None,
            allow_msaa: true,
            taa_base_blend_factor: 0.875,
            taa_jitter_scale: 1.0,
            taa_variance_clamp_scale: 0.9,
            history_id: 0,
        }
    }
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
    /// Anti-aliasing. With several cameras in one frame, the first camera's setting applies
    /// to the whole target (TAA needs a single camera; otherwise it falls back to FXAA).
    pub antialiasing: AntiAliasingSettings,
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
    /// Stable identity of the renderer's object (0 = none), used for motion vectors.
    pub id: u64,
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
    /// Drawn into Baked reflection probes (Reflection Probe Static).
    pub reflection_probe_static: bool,
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

/// When a reflection probe captures its surroundings (Unity `ReflectionProbeRefreshMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeRefresh {
    /// Once, and again when the probe moves or its settings change.
    OnAwake,
    /// Every frame (at most a couple of probes per frame, round robin).
    EveryFrame,
    /// Only when [`ReflectionProbeView::render_requests`] changes (`RenderProbe()`).
    ViaScripting,
}

/// One reflection probe: a cubemap captured at `position`, used by renderers inside its box.
/// <https://docs.unity3d.com/Manual/class-ReflectionProbe.html>
#[derive(Clone, Debug)]
pub struct ReflectionProbeView {
    /// Stable identity (the probe's GameObject), so captures persist across frames.
    pub id: u64,
    pub position: Vector3,
    /// World-space influence box.
    pub box_min: Vector3,
    pub box_max: Vector3,
    pub box_projection: bool,
    pub blend_distance: f32,
    pub importance: i32,
    pub intensity: f32,
    pub near: f32,
    pub far: f32,
    pub culling_mask: u32,
    pub clear: ClearMode,
    /// Capture only Reflection Probe Static renderers (Unity "Baked" type).
    pub baked: bool,
    pub refresh: ProbeRefresh,
    /// Bumped by `RenderProbe()`; a change triggers a capture.
    pub render_requests: u32,
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
    /// MSAA samples (1, 2, 4 or 8; clamped to what the GPU supports).
    pub msaa_samples: u32,
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
            msaa_samples: 4,
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
    pub reflection_probes: Vec<ReflectionProbeView>,

    /// World-space skinning matrices (bone local-to-world × bind pose).
    pub joints: Vec<Matrix4x4>,
    pub environment: Environment,
    /// Pairs of vertices (line list).
    pub lines: Vec<LineVertex>,
    pub time: f32,
    /// Frame number (Unity `Time.frameCount`): drives TAA jitter, and tells repeated renders
    /// of one frame (Scene and Game views) apart from new frames.
    pub frame_index: u64,
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
    /// Reflection probes with a cubemap slot.
    pub reflection_probes: u32,
    /// Cubemap captures done this frame (sky included).
    pub probe_captures: u32,
}
