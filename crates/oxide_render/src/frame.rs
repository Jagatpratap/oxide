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
    /// Unity Physical Camera: focal length and sensor size (mm), lens shift (in sensor
    /// sizes) and how the sensor fits the screen.
    Physical {
        focal_length: f32,
        sensor: [f32; 2],
        lens_shift: [f32; 2],
        gate_fit: GateFit,
    },
}

/// Unity `Camera.GateFitMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GateFit {
    Vertical,
    #[default]
    Horizontal,
    Fill,
    Overscan,
    None,
}

impl ProjectionKind {
    /// Vertical field of view (degrees) at a screen aspect; `None` for orthographic.
    pub fn vertical_fov(&self, aspect: f32) -> Option<f32> {
        match *self {
            ProjectionKind::Perspective { fov } => Some(fov),
            ProjectionKind::Orthographic { .. } => None,
            ProjectionKind::Physical {
                focal_length,
                sensor,
                gate_fit,
                ..
            } => {
                let f = focal_length.max(1e-3);
                let v = 2.0 * (sensor[1] / (2.0 * f)).atan();
                // Vertical fov when the sensor width fills the screen width.
                let h = 2.0 * (sensor[0] / (2.0 * f) / aspect.max(1e-6)).atan();
                let fov = match gate_fit {
                    GateFit::Vertical | GateFit::None => v,
                    GateFit::Horizontal => h,
                    GateFit::Fill => v.min(h),
                    GateFit::Overscan => v.max(h),
                };
                Some(fov.to_degrees().clamp(1.0, 179.0))
            }
        }
    }
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
    /// Subpixel Morphological Anti-aliasing: edge detection, shape-based blend weights and
    /// neighbourhood blending after tonemapping ([`AntiAliasingSettings::smaa_quality`]).
    Smaa,
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
    /// SMAA quality (URP "Quality": Low, Medium, High).
    pub smaa_quality: crate::aa::SmaaQuality,
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
            smaa_quality: crate::aa::SmaaQuality::Medium,
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
            ProjectionKind::Orthographic { size } => {
                let w = size * aspect;
                Matrix4x4::ortho(-w, w, -size, size, self.far, self.near)
            }
            p => {
                let fov = p.vertical_fov(aspect).unwrap_or(60.0);
                let mut m = Matrix4x4::perspective(
                    fov.clamp(1.0, 179.0).to_radians(),
                    aspect,
                    self.far,
                    self.near,
                );
                if let ProjectionKind::Physical { lens_shift, .. } = p {
                    // Lens shift moves the image by a share of the view: an off-center
                    // frustum (clip x += 2·shift·w).
                    let shift = Matrix4x4::from_cols(
                        oxide_math::Vector4::X,
                        oxide_math::Vector4::Y,
                        oxide_math::Vector4::Z,
                        oxide_math::Vector4::new(
                            2.0 * lens_shift[0],
                            2.0 * lens_shift[1],
                            0.0,
                            1.0,
                        ),
                    );
                    m = shift * m;
                }
                m
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
    /// Rendering layers it lights (URP light "Rendering Layers"; objects' masks must overlap).
    pub rendering_layers: u32,
    /// Light cookie: the texture tiled across the world, `size` metres per tile and
    /// `offset`, along [`Self::right`] and its up vector.
    pub cookie: Option<SunCookie>,
    /// The light's right vector (cookie orientation).
    pub right: Vector3,
    /// A Mixed light (baked data may stand in for it on lightmapped objects).
    pub mixed: bool,
    /// Its shadowmask channel (Shadowmask mode).
    pub shadowmask_channel: Option<u8>,
}

/// The Lighting Mode of the baked data (Unity `MixedLightingMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MixedMode {
    /// Mixed lights stay realtime (only their bounce is baked).
    #[default]
    IndirectOnly,
    /// Lightmapped objects take Mixed lights' static shadows from the shadowmask.
    Shadowmask,
    /// Mixed lights are baked into lightmapped objects; the main light's realtime shadow is
    /// subtracted from their lightmap.
    Subtractive,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixedLighting {
    pub mode: MixedMode,
    /// Subtractive: the darkest a realtime shadow makes baked light (sRGB).
    pub shadow_color: Color,
}

impl Default for MixedLighting {
    fn default() -> Self {
        Self {
            mode: MixedMode::IndirectOnly,
            shadow_color: Color::rgb(0.42, 0.478, 0.627),
        }
    }
}

/// A directional light's cookie (URP "Cookie Size" and "Cookie Offset").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunCookie {
    pub texture: AssetId,
    pub size: [f32; 2],
    pub offset: [f32; 2],
}

impl Default for SunLight {
    fn default() -> Self {
        Self {
            direction: Vector3::NEG_Y,
            color: [1.0; 3],
            shadows: None,
            rendering_layers: u32::MAX,
            cookie: None,
            right: Vector3::X,
            mixed: false,
            shadowmask_channel: None,
        }
    }
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
    /// Rendering layers it lights.
    pub rendering_layers: u32,
    /// Spot light cookie: the texture projected through the cone.
    pub cookie: Option<AssetId>,
    /// The light's right vector (cookie orientation).
    pub right: Vector3,
    pub mixed: bool,
    pub shadowmask_channel: Option<u8>,
}

impl Default for PunctualLight {
    fn default() -> Self {
        Self {
            position: Vector3::ZERO,
            range: 10.0,
            color: [1.0; 3],
            spot: None,
            shadow: None,
            rendering_layers: u32::MAX,
            cookie: None,
            right: Vector3::X,
            mixed: false,
            shadowmask_channel: None,
        }
    }
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
    pub sorting: DrawSorting,
    /// Rendering layers of the renderer (`Renderer.renderingLayerMask`): lights only reach
    /// objects whose mask overlaps theirs.
    pub rendering_layers: u32,
}

/// A renderer's draw-order settings (Unity Sorting Layer, Order in Layer, Priority).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawSorting {
    /// Index of the sorting layer (Tags and Layers order).
    pub layer: i32,
    pub order: i32,
    pub priority: i32,
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

/// How a VFX Graph output blends (Unity's Blend Mode on an output context).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VfxBlend {
    #[default]
    Alpha,
    Additive,
    Opaque,
}

/// How VFX Graph quads face (Unity's Orient block).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VfxOrient {
    #[default]
    FaceCamera,
    AlongVelocity,
}

/// A compiled VFX Graph system: its generated hooks and output settings.
#[derive(Clone, Debug, Default)]
pub struct VfxProgram {
    /// Changes whenever the code or settings change (GPU programs are cached by it).
    pub hash: u64,
    /// Asset name, for error messages.
    pub name: String,
    /// `vfx_init`, `vfx_update` and `vfx_output` (see `vfx.rs`).
    pub hooks: String,
    pub capacity: u32,
    pub blend: VfxBlend,
    pub orient: VfxOrient,
    /// Flipbook grid (columns, rows); 0 = no flipbook.
    pub flipbook: [u32; 2],
    /// Particles live in the effect's local space (they move with it).
    pub local_space: bool,
    /// No texture: draw Unity's default soft round particle.
    pub soft_dot: bool,
    /// Baked gradients and curves the hooks read (`luts`).
    pub luts: Vec<[f32; 4]>,
}

/// One VFX Graph system of a Visual Effect to simulate on the GPU and draw.
#[derive(Clone, Debug)]
pub struct VfxDraw {
    /// The effect instance and system (its particles persist between frames under it).
    pub key: u64,
    pub program: std::sync::Arc<VfxProgram>,
    /// Particles to spawn this frame.
    pub spawn: u32,
    /// Kill every particle first (Reinit / Play after Stop).
    pub reset: bool,
    /// Advance the simulation this frame (false while paused).
    pub simulate: bool,
    /// The effect's update count: the system steps once per new value, however many views
    /// render it (Edit mode has no frame counter of its own).
    pub step: u64,
    pub visible: bool,
    pub dt: f32,
    pub time: f32,
    pub seed: u32,
    pub object_to_world: Matrix4x4,
    /// Exposed property values (vec4 slots).
    pub properties: Vec<[f32; 4]>,
    pub texture: Option<AssetId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParticleBlend {
    Alpha,
    Additive,
}

/// RGBA8 (sRGB) pixels for a Render Texture asset, written before the frame draws. Writes
/// with a `serial` already written to that target are skipped.
#[derive(Clone, Debug)]
pub struct TextureWrite {
    pub target: AssetId,
    pub width: u32,
    pub height: u32,
    pub rgba: std::sync::Arc<Vec<u8>>,
    pub serial: u64,
}

/// How a particle quad is oriented (Unity's Particle System Renderer modes, plus the
/// segments trails and lines are drawn with).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ParticleMode {
    /// Faces the camera.
    #[default]
    Billboard = 0,
    /// Stretched along `axis` (its half length), `size` wide.
    Stretched = 1,
    /// Flat on the XZ plane.
    Horizontal = 2,
    /// Faces the camera but stays upright.
    Vertical = 3,
    /// A ribbon piece: `position` is the middle, `axis` half the segment, `size` the width;
    /// it turns to face the camera around the segment.
    Segment = 4,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleInstance {
    pub position: [f32; 3],
    pub size: f32,
    /// Linear RGB (may exceed 1 for HDR glow) and alpha.
    pub color: [f32; 4],
    pub rotation: f32,
    /// [`ParticleMode`] as a float.
    pub mode: f32,
    pub _pad: [f32; 2],
    /// Texture rectangle (min x, min y, width, height; origin top left).
    pub uv: [f32; 4],
    /// Stretched / Segment: the half-length vector.
    pub axis: [f32; 3],
    /// Height ÷ width (non-uniform sizes).
    pub aspect: f32,
}

impl ParticleInstance {
    /// A camera-facing particle covering the whole texture.
    pub fn billboard(position: [f32; 3], size: f32, color: [f32; 4], rotation: f32) -> Self {
        Self {
            position,
            size,
            color,
            rotation,
            mode: ParticleMode::Billboard as u8 as f32,
            _pad: [0.0; 2],
            uv: [0.0, 0.0, 1.0, 1.0],
            axis: [0.0; 3],
            aspect: 1.0,
        }
    }

    pub fn with_mode(mut self, mode: ParticleMode, axis: [f32; 3]) -> Self {
        self.mode = mode as u8 as f32;
        self.axis = axis;
        self
    }

    pub fn with_uv(mut self, uv: [f32; 4]) -> Self {
        self.uv = uv;
        self
    }

    /// Height as a multiple of the width.
    pub fn with_aspect(mut self, aspect: f32) -> Self {
        self.aspect = aspect;
        self
    }
}

/// One sprite quad for the sprite pass: the unit square (0..1) placed by `model`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteInstance {
    pub model: [[f32; 4]; 4],
    /// UV origin and size in the texture (0..1, origin top-left).
    pub uv_rect: [f32; 4],
    /// Linear RGB and alpha.
    pub color: [f32; 4],
}

/// A sprite to draw (Unity SpriteRenderer or a tile): drawn after transparent objects,
/// sorted by sorting layer, then order in layer, then back to front.
#[derive(Clone, Copy, Debug)]
pub struct SpriteDraw {
    pub texture: Option<AssetId>,
    /// Index of the sorting layer (Project Settings → Tags and Layers).
    pub sorting_layer: i32,
    pub order: i32,
    pub instance: SpriteInstance,
}

/// One grass / detail billboard (Unity Grass and Grass Billboard details).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GrassInstance {
    /// Ground point (the quad's bottom centre).
    pub position: [f32; 3],
    pub width: f32,
    pub height: f32,
    /// Yaw in radians.
    pub rotation: f32,
    /// How far the tip sways in the wind (fraction of the height).
    pub wind: f32,
    /// 0 = two crossed quads, 1 = camera-facing quad.
    pub mode: f32,
    /// Linear RGB tint; alpha = wind speed.
    pub color: [f32; 4],
}

/// Grass instances sharing one texture.
#[derive(Clone, Debug, Default)]
pub struct GrassBatch {
    pub texture: Option<AssetId>,
    pub instances: Vec<GrassInstance>,
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
    /// A cubemap used instead of capturing (Custom probes, and Baked probes' saved
    /// captures): the Cubemap asset, its version, and whether it is RGBM-encoded HDR.
    pub source: Option<(AssetId, u64, bool)>,
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
    /// Lens dirt texture and intensity.
    pub dirt: Option<(AssetId, f32)>,
}

/// Samples per Color Curves curve.
pub const CURVE_SAMPLES: usize = 128;

/// URP Color Curves, sampled over 0..1: Master, Red, Green, Blue (outputs 0..1), Hue vs
/// Hue (0.5 = no shift), Hue vs Sat, Sat vs Sat, Lum vs Sat (0.5 = ×1).
pub type ColorCurveSamples = [[f32; CURVE_SAMPLES]; 8];

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
    pub screen_space_lens_flare: ScreenSpaceLensFlareSettings,
    /// Color Curves (None: unchanged).
    pub curves: Option<std::sync::Arc<ColorCurveSamples>>,
    /// Color Lookup texture (Unity strip) and contribution.
    pub lookup: Option<(AssetId, f32)>,
    /// Panini Projection `(distance, crop to fit)`.
    pub panini: [f32; 2],
}

/// URP's Screen Space Lens Flare: ghosts, a halo and streaks built from a bloom mip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenSpaceLensFlareSettings {
    /// 0 turns it off.
    pub intensity: f32,
    /// Linear tint.
    pub tint: [f32; 3],
    /// Bloom mip the flares sample (0 = half resolution).
    pub bloom_mip: u32,
    pub first_flare_intensity: f32,
    pub secondary_flare_intensity: f32,
    pub warped_flare_intensity: f32,
    pub warped_flare_scale: [f32; 2],
    /// Ghost repetitions (1..3).
    pub samples: u32,
    /// Each repetition is this much dimmer.
    pub sample_dimmer: f32,
    /// Fades ghosts sampled near the screen edge.
    pub vignette_effect: f32,
    pub starting_position: f32,
    pub scale: f32,
    pub streaks_intensity: f32,
    pub streaks_length: f32,
    /// Degrees.
    pub streaks_orientation: f32,
    pub streaks_threshold: f32,
    /// Output size as a bloom mip index (0 = half, 1 = quarter, 2 = eighth).
    pub resolution_mip: u32,
    pub chromatic_aberration: f32,
}

impl Default for ScreenSpaceLensFlareSettings {
    fn default() -> Self {
        Self {
            intensity: 0.0,
            tint: [1.0; 3],
            bloom_mip: 1,
            first_flare_intensity: 1.0,
            secondary_flare_intensity: 1.0,
            warped_flare_intensity: 1.0,
            warped_flare_scale: [1.0, 1.0],
            samples: 1,
            sample_dimmer: 0.5,
            vignette_effect: 1.0,
            starting_position: 1.25,
            scale: 1.5,
            streaks_intensity: 0.0,
            streaks_length: 0.5,
            streaks_orientation: 0.0,
            streaks_threshold: 0.25,
            resolution_mip: 1,
            chromatic_aberration: 0.5,
        }
    }
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
                dirt: None,
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
            screen_space_lens_flare: ScreenSpaceLensFlareSettings::default(),
            curves: None,
            lookup: None,
            panini: [0.0, 1.0],
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
    /// A textured skybox instead of the procedural sky.
    pub skybox: Option<Skybox>,
}

/// Where a textured skybox's texels come from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkyboxSource {
    /// A Cubemap asset.
    Cubemap(AssetId),
    /// Six textures in cube face order (+X, -X, +Y, -Y, +Z, -Z).
    SixSided([AssetId; 6]),
    /// An equirectangular panorama.
    Panoramic(AssetId),
}

/// A textured skybox (Unity's Skybox/Cubemap, 6 Sided and Panoramic shaders).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skybox {
    pub source: SkyboxSource,
    /// sRGB tint; 0.5 grey is neutral (Unity doubles it).
    pub tint: Color,
    pub exposure: f32,
    /// Degrees around +Y.
    pub rotation: f32,
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
            skybox: None,
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

/// Which render queues `DrawRenderers` draws (Unity `RenderQueueRange`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderQueue {
    /// Opaque and alpha-tested, front to back.
    #[default]
    Opaque,
    /// Alpha-blended, back to front.
    Transparent,
    /// Both (opaque first).
    All,
}

/// One command of a custom render pipeline (Unity's `ScriptableRenderContext` calls), run
/// in order for each camera instead of URP's own sequence.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderStep {
    /// Clears to the camera's background (sky or colour) and/or depth.
    ClearRenderTarget {
        color: bool,
        depth: bool,
    },
    /// The sky where nothing has been drawn yet.
    DrawSkybox,
    /// Renderers in `queue` on `layer_mask`, optionally with another material.
    DrawRenderers {
        queue: RenderQueue,
        layer_mask: u32,
        override_material: Option<AssetId>,
    },
    DrawTerrain,
    /// Particle Systems and Visual Effects.
    DrawEffects,
    /// Sprites and 3D text.
    DrawSprites,
    /// Gizmo and debug lines.
    DrawGizmos,
}

/// URP's rendering paths (Universal Renderer "Rendering Path").
/// <https://docs.unity3d.com/Manual/urp/rendering-paths-introduction.html>
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RenderingPath {
    /// Every object lit in its own draw (Oxide's forward is clustered either way).
    Forward,
    /// Forward with clustered lights (no per-object light limit).
    #[default]
    ForwardPlus,
    /// Opaque objects write a G-buffer; one full-screen pass lights them. Transparent,
    /// terrain and other forward-only objects are drawn afterwards. No MSAA (as in URP).
    Deferred,
}

/// Unity's Scene view draw modes: shading modes and debug views.
/// <https://docs.unity3d.com/Manual/ViewModes.html>
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DebugView {
    #[default]
    Shaded,
    Wireframe,
    ShadedWireframe,
    /// Surface color without lighting.
    Albedo,
    /// World-space normals as colors.
    Normals,
    /// Lighting on a neutral gray surface.
    LightingOnly,
    /// Each sun shadow cascade tinted (red, green, blue, yellow).
    ShadowCascades,
    /// Diffuse light from lightmaps and light probes only.
    BakedLightmap,
}

impl DebugView {
    pub const ALL: [DebugView; 8] = [
        DebugView::Shaded,
        DebugView::Wireframe,
        DebugView::ShadedWireframe,
        DebugView::Albedo,
        DebugView::Normals,
        DebugView::LightingOnly,
        DebugView::ShadowCascades,
        DebugView::BakedLightmap,
    ];

    pub fn name(self) -> &'static str {
        match self {
            DebugView::Shaded => "Shaded",
            DebugView::Wireframe => "Wireframe",
            DebugView::ShadedWireframe => "Shaded Wireframe",
            DebugView::Albedo => "Albedo",
            DebugView::Normals => "Normals",
            DebugView::LightingOnly => "Lighting Only",
            DebugView::ShadowCascades => "Shadow Cascades",
            DebugView::BakedLightmap => "Baked Lightmap",
        }
    }

    /// Value of the shaders' debug mode (`frame.misc.y`): 0 = normal shading.
    pub fn shader_mode(self) -> u32 {
        match self {
            DebugView::Albedo => 1,
            DebugView::Normals => 2,
            DebugView::LightingOnly => 3,
            DebugView::ShadowCascades => 4,
            DebugView::BakedLightmap => 5,
            _ => 0,
        }
    }

    /// Draws mesh edges.
    pub fn wire(self) -> bool {
        matches!(self, DebugView::Wireframe | DebugView::ShadedWireframe)
    }

    /// Draws shaded surfaces (everything but pure Wireframe).
    pub fn shaded(self) -> bool {
        self != DebugView::Wireframe
    }
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
    /// URP Decal Projectors (first camera).
    pub decals: Vec<crate::decals::DecalDraw>,
    /// Pixels written into Render Textures before drawing (video frames).
    pub texture_writes: Vec<TextureWrite>,
    /// VFX Graph systems (GPU particles).
    pub vfx: Vec<VfxDraw>,
    /// Terrain grass and detail billboards.
    pub grass: Vec<GrassBatch>,
    pub sprites: Vec<SpriteDraw>,
    /// Lens flares (Unity Lens Flare (SRP)), drawn over the first camera's view before
    /// bloom.
    pub lens_flares: Vec<crate::flares::LensFlareSource>,
    /// 3D text (TextMeshPro), one entry per text object and atlas page.
    pub texts: Vec<crate::text::TextDraw>,
    pub reflection_probes: Vec<ReflectionProbeView>,
    /// Cameras rendering into Render Textures (Unity `Camera.targetTexture`), drawn before
    /// the main cameras, at most once per frame each.
    pub texture_cameras: Vec<(AssetId, CameraView)>,
    /// Renderer features, in order (Unity's Renderer Features list).
    pub renderer_features: Vec<RendererFeature>,
    /// Baked lightmaps (RGBM textures of one size; Unity `LightmapSettings.lightmaps`).
    pub lightmaps: Vec<AssetId>,
    /// Per lightmap (when baked): shadowmask and dominant-direction textures.
    pub shadowmasks: Vec<AssetId>,
    pub lightmap_directions: Vec<AssetId>,
    /// How Mixed lights combine with the baked data.
    pub mixed_lighting: MixedLighting,
    /// Scene view draw mode (shaded, wireframe or a debug view of one input).
    pub debug_view: DebugView,
    /// From the Render Pipeline Asset (Graphics / Quality settings).
    pub rendering_path: RenderingPath,
    /// Global shader keywords (Unity `Shader.EnableKeyword`).
    pub global_keywords: Vec<String>,
    /// A custom render pipeline's commands; `None` = the Universal Render Pipeline.
    pub custom_steps: Option<Vec<RenderStep>>,
    /// Universal Renderer filtering: layers drawn in the opaque and transparent queues
    /// (on top of each camera's culling mask). `None` = every layer.
    pub opaque_layer_mask: Option<u32>,
    pub transparent_layer_mask: Option<u32>,

    /// World-space skinning matrices (bone local-to-world × bind pose).
    pub joints: Vec<Matrix4x4>,
    pub environment: Environment,
    /// Pairs of vertices (line list).
    pub lines: Vec<LineVertex>,
    pub time: f32,
    /// Frame number (Unity `Time.frameCount`): drives TAA jitter, and tells repeated renders
    /// of one frame (Scene and Game views) apart from new frames.
    pub frame_index: u64,
    /// URP Render Scale of game views: the scene and post-processing render at this size and
    /// are stretched to the target. None (or 1) renders at full size.
    pub render_scale: Option<RenderScale>,
    /// Texture Quality: the largest mips skipped when textures upload.
    pub texture_mip_limit: u32,
    /// Anisotropy of trilinear textures (1 = off); None keeps the default (8×).
    pub texture_anisotropy: Option<u16>,
}

/// See [`RenderFrame::render_scale`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderScale {
    pub scale: f32,
    /// Nearest-pixel upscaling instead of bilinear.
    pub point: bool,
}

impl RenderScale {
    /// The scaled size of a `width`×`height` target (at least 1×1).
    pub fn size(self, width: u32, height: u32) -> (u32, u32) {
        let s = self.scale.clamp(0.1, 2.0);
        (
            ((width as f32 * s).round() as u32).max(1),
            ((height as f32 * s).round() as u32).max(1),
        )
    }
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
