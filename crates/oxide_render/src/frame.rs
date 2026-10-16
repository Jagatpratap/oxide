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
    /// Shadow settings when the light casts shadows (rendered into the shadow atlas).
    pub shadow: Option<ShadowSettings>,
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
    /// Baked lightmap: index into [`RenderFrame::lightmaps`] and Unity `lightmapScaleOffset`.
    pub lightmap: Option<(u32, oxide_math::Vector4)>,
    /// Light from baked light probes (L2 SH, cosine-convolved, ÷π) for objects without a
    /// lightmap.
    pub light_probe: Option<[Vector3; 9]>,
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

/// When a renderer feature runs (Unity `RenderPassEvent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderPassEvent {
    AfterRenderingOpaques,
    AfterRenderingTransparents,
    /// On the HDR image, after TAA / depth of field / motion blur, before bloom and grading.
    BeforeRenderingPostProcessing,
    /// On the final (tonemapped, linear) image.
    AfterRenderingPostProcessing,
}

/// Depth test of a Render Objects feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DepthTest {
    /// Visible surfaces only (the usual test).
    LessEqual,
    /// Only where something nearer hides it (x-ray / see-through-walls silhouettes).
    Greater,
    /// Ignores depth (draws on top).
    Always,
}

/// URP renderer features configured for this frame.
#[derive(Clone, Debug)]
pub enum RendererFeature {
    /// Full Screen Pass: a custom WGSL fragment shader over the camera image.
    FullScreen {
        event: RenderPassEvent,
        /// A `Shader` asset with `fn fs_main(in: FullscreenIn) -> vec4f`.
        shader: AssetId,
        params: [[f32; 4]; 2],
    },
    /// Render Objects: draws renderers on `layer_mask` again, optionally with another
    /// material and depth test.
    RenderObjects {
        event: RenderPassEvent,
        layer_mask: u32,
        override_material: Option<AssetId>,
        depth_test: DepthTest,
        write_depth: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum FogKind {
    Linear { start: f32, end: f32 },
    Exponential { density: f32 },
    ExponentialSquared { density: f32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TonemapMode {
    None,
    /// Unity's Neutral curve: compresses the range, keeps hue and saturation.
    Neutral,
    #[default]
    Aces,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomSettings {
    pub intensity: f32,
    /// Gamma-space brightness above which pixels bloom.
    pub threshold: f32,
    /// 0..1: how far the glow spreads.
    pub scatter: f32,
    /// Linear tint.
    pub tint: [f32; 3],
    /// Brightest value that can bloom.
    pub clamp: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VignetteSettings {
    /// Linear color.
    pub color: [f32; 3],
    pub center: [f32; 2],
    pub intensity: f32,
    pub smoothness: f32,
    pub rounded: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DofMode {
    #[default]
    Off,
    Gaussian,
    Bokeh,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DofSettings {
    pub mode: DofMode,
    pub gaussian_start: f32,
    pub gaussian_end: f32,
    pub gaussian_max_radius: f32,
    pub focus_distance: f32,
    /// Millimetres.
    pub focal_length: f32,
    /// f-number.
    pub aperture: f32,
}

impl Default for DofSettings {
    fn default() -> Self {
        Self {
            mode: DofMode::Off,
            gaussian_start: 10.0,
            gaussian_end: 30.0,
            gaussian_max_radius: 1.0,
            focus_distance: 10.0,
            focal_length: 50.0,
            aperture: 5.6,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LensDistortionSettings {
    pub intensity: f32,
    pub multiplier: [f32; 2],
    pub center: [f32; 2],
    pub scale: f32,
}

/// The resolved post-processing stack (Unity's Volume stack for the camera).
/// Trackball values (`lift`, `shadows`, ...) are Unity's: `xyz` color with 1 = neutral and
/// `w` offset.
#[derive(Clone, Debug, PartialEq)]
pub struct PostSettings {
    /// Off for cameras with post-processing disabled (tonemapping and grading too).
    pub enabled: bool,
    /// Linear multiplier before grading (RenderSettings exposure × 2^post exposure).
    pub exposure: f32,
    pub tonemapping: TonemapMode,
    pub bloom: BloomSettings,
    /// White balance `(temperature, tint)`, -100..100.
    pub white_balance: [f32; 2],
    /// -100..100.
    pub contrast: f32,
    pub saturation: f32,
    /// Degrees.
    pub hue_shift: f32,
    /// Linear color.
    pub color_filter: [f32; 3],
    /// Output rows in percent: `[r_out, g_out, b_out]` each `[r_in, g_in, b_in]`.
    pub channel_mixer: [[f32; 3]; 3],
    pub lift: [f32; 4],
    pub gamma: [f32; 4],
    pub gain: [f32; 4],
    pub shadows: [f32; 4],
    pub midtones: [f32; 4],
    pub highlights: [f32; 4],
    /// Shadows start/end, highlights start/end.
    pub smh_ranges: [f32; 4],
    /// sRGB colors (grey = none).
    pub split_shadows: [f32; 3],
    pub split_highlights: [f32; 3],
    pub split_balance: f32,
    pub vignette: VignetteSettings,
    pub chromatic_aberration: f32,
    /// `(intensity, response)`.
    pub film_grain: [f32; 2],
    pub lens_distortion: LensDistortionSettings,
    pub depth_of_field: DofSettings,
    /// `(intensity, clamp)`: blur length scale and longest blur as a fraction of the screen.
    pub motion_blur: [f32; 2],
}

impl Default for PostSettings {
    fn default() -> Self {
        let neutral = [1.0, 1.0, 1.0, 0.0];
        Self {
            enabled: true,
            exposure: 1.0,
            tonemapping: TonemapMode::Aces,
            bloom: BloomSettings {
                intensity: 0.4,
                threshold: 1.0,
                scatter: 0.7,
                tint: [1.0; 3],
                clamp: 65472.0,
            },
            white_balance: [0.0; 2],
            contrast: 0.0,
            saturation: 0.0,
            hue_shift: 0.0,
            color_filter: [1.0; 3],
            channel_mixer: [[100.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 0.0, 100.0]],
            lift: neutral,
            gamma: neutral,
            gain: neutral,
            shadows: neutral,
            midtones: neutral,
            highlights: neutral,
            smh_ranges: [0.0, 0.3, 0.55, 1.0],
            split_shadows: [0.5; 3],
            split_highlights: [0.5; 3],
            split_balance: 0.0,
            vignette: VignetteSettings {
                color: [0.0; 3],
                center: [0.5, 0.5],
                intensity: 0.0,
                smoothness: 0.2,
                rounded: false,
            },
            chromatic_aberration: 0.0,
            film_grain: [0.0, 0.8],
            lens_distortion: LensDistortionSettings {
                intensity: 0.0,
                multiplier: [1.0, 1.0],
                center: [0.5, 0.5],
                scale: 1.0,
            },
            depth_of_field: DofSettings::default(),
            motion_blur: [0.0, 0.05],
        }
    }
}

impl PostSettings {
    /// The defaults without bloom: ACES tonemapping only (handy for tests).
    pub fn neutral() -> Self {
        Self {
            bloom: BloomSettings {
                intensity: 0.0,
                ..Self::default().bloom
            },
            ..Self::default()
        }
    }
}

/// URP's Screen Space Ambient Occlusion renderer feature.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsaoSettings {
    pub intensity: f32,
    /// World-space sampling radius.
    pub radius: f32,
    /// How much AO also darkens direct light (0..1).
    pub direct_lighting_strength: f32,
    /// AO fades out by this distance from the camera.
    pub falloff_distance: f32,
    /// Samples per pixel (4..32).
    pub samples: u32,
}

impl Default for SsaoSettings {
    fn default() -> Self {
        Self {
            intensity: 1.5,
            radius: 0.35,
            direct_lighting_strength: 0.25,
            falloff_distance: 100.0,
            samples: 12,
        }
    }
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
    /// Post-processing after the Volume stack is evaluated.
    pub post: PostSettings,
    /// Screen-space ambient occlusion (first camera only).
    pub ssao: Option<SsaoSettings>,
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
            post: PostSettings::default(),
            ssao: None,
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
    /// Cameras rendering into Render Textures (Unity `Camera.targetTexture`), drawn before
    /// the main cameras, at most once per frame each.
    pub texture_cameras: Vec<(AssetId, CameraView)>,
    /// Renderer features, in order (Unity's Renderer Features list).
    pub renderer_features: Vec<RendererFeature>,
    /// Baked lightmaps (RGBM textures of one size; Unity `LightmapSettings.lightmaps`).
    pub lightmaps: Vec<AssetId>,

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
