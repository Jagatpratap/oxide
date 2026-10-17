//! Built-in rendering components: Camera, Light, MeshFilter, MeshRenderer,
//! SkinnedMeshRenderer, plus the scene's RenderSettings (Lighting window → Environment).

use crate::component::Component;
use crate::id::GameObjectId;
use oxide_assets::{Handle, Material, Mesh};
use oxide_core::Reflect;
use oxide_math::{Color, Vector3, Vector4};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum CameraClearFlags {
    #[default]
    Skybox,
    SolidColor,
    DepthOnly,
    Nothing,
}

/// Post-process anti-aliasing (Unity URP `AntialiasingMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum AntialiasingMode {
    #[default]
    None,
    FastApproximateAntialiasing,
    /// Turns MSAA off for this camera, like URP.
    TemporalAntiAliasing,
}

/// MSAA sample count (URP asset "Anti Aliasing (MSAA)").
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Default, Reflect, serde::Serialize, serde::Deserialize,
)]
pub enum MsaaQuality {
    Disabled,
    Msaa2x,
    #[default]
    Msaa4x,
    Msaa8x,
}

/// URP camera "Render Type".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum CameraRenderType {
    /// Renders to the screen or a texture; can have a stack of Overlay cameras.
    #[default]
    Base,
    /// Draws on top of a Base camera's output, only when it is in that camera's Stack.
    Overlay,
}

/// Unity `Camera.GateFitMode`: how the sensor (film gate) fits the screen when the aspect
/// ratios differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum GateFitMode {
    /// The sensor height fills the screen height.
    Vertical,
    /// The sensor width fills the screen width.
    #[default]
    Horizontal,
    /// The screen shows only the sensor (the larger of the two fits, cropping).
    Fill,
    /// The screen shows all of the sensor (the smaller fit, with extra around it).
    Overscan,
    /// Stretch the sensor to the screen.
    None,
}

/// Unity `Camera`. <https://docs.unity3d.com/Manual/class-Camera.html>
#[derive(Clone, Debug, Reflect)]
#[reflect(default)]
pub struct Camera {
    /// URP Render Type: Base, or Overlay (drawn by a Base camera's Stack).
    pub render_type: CameraRenderType,
    /// Base cameras: the Overlay cameras drawn on top, in order (URP camera Stack).
    pub camera_stack: Vec<GameObjectId>,
    /// Overlay cameras: clear the depth buffer before drawing (URP "Clear Depth").
    pub clear_depth: bool,
    pub projection: Projection,
    /// Vertical field of view in degrees.
    #[reflect(range(1, 179))]
    pub field_of_view: f32,
    pub orthographic_size: f32,
    pub near_clip_plane: f32,
    pub far_clip_plane: f32,
    pub clear_flags: CameraClearFlags,
    pub background_color: Color,
    /// Cameras render in ascending depth order; the highest is on top.
    pub depth: f32,
    /// Normalized viewport `x, y, width, height`.
    pub viewport_rect: Vector4,
    pub culling_mask: u32,
    /// Post-processing (tonemapping, bloom, ...) on this camera.
    pub post_processing: bool,
    /// Skip renderers hidden behind occluders (Unity `Camera.useOcclusionCulling`).
    pub use_occlusion_culling: bool,
    /// Post-process anti-aliasing (URP camera "Anti-aliasing").
    pub antialiasing: AntialiasingMode,
    /// Use the project's MSAA on this camera (Unity `Camera.allowMSAA`).
    pub allow_msaa: bool,
    /// TAA: share of the history kept each frame (URP "Base Blend Factor").
    #[reflect(range(0, 1))]
    pub taa_base_blend_factor: f32,
    /// TAA: sub-pixel jitter size (URP "Jitter Scale").
    #[reflect(range(0, 1))]
    pub taa_jitter_scale: f32,
    /// TAA: history clamp tightness; lower = less ghosting, more flicker.
    #[reflect(range(0.5, 2))]
    pub taa_variance_clamp_scale: f32,
    /// Render into this texture instead of the screen (Unity `Camera.targetTexture`).
    pub target_texture: Option<Handle<oxide_assets::RenderTexture>>,
    /// Which display this camera renders to, 0-7 (Unity `Camera.targetDisplay`: Display 1-8).
    pub target_display: u32,
    // Physical Camera (Unity `Camera.usePhysicalProperties`).
    /// Field of view comes from Focal Length and Sensor Size.
    pub use_physical_properties: bool,
    /// Millimetres.
    pub focal_length: f32,
    /// Millimetres (width, height); 36 × 24 is 35 mm full frame.
    pub sensor_size: oxide_math::Vector2,
    /// Off-center projection, in sensor sizes (Unity "Shift").
    pub lens_shift: oxide_math::Vector2,
    pub gate_fit: GateFitMode,
    /// Sensor sensitivity (Unity `Camera.iso`).
    pub iso: u32,
    /// Seconds (Unity `Camera.shutterSpeed`).
    pub shutter_speed: f32,
    /// f-number (Unity `Camera.aperture`).
    pub aperture: f32,
    /// Metres (Unity `Camera.focusDistance`).
    pub focus_distance: f32,
    #[reflect(range(3, 11))]
    pub blade_count: u32,
    /// Aperture shape range (Unity `Camera.curvature`).
    pub curvature: oxide_math::Vector2,
    #[reflect(range(0, 1))]
    pub barrel_clipping: f32,
    #[reflect(range(-1, 1))]
    pub anamorphism: f32,
}

impl Camera {
    /// Unity `Camera.FocalLengthToFieldOfView`: vertical field of view (degrees).
    pub fn focal_length_to_field_of_view(focal_length: f32, sensor_height: f32) -> f32 {
        2.0 * (sensor_height / (2.0 * focal_length.max(1e-3)))
            .atan()
            .to_degrees()
    }

    /// Unity `Camera.FieldOfViewToFocalLength`.
    pub fn field_of_view_to_focal_length(fov: f32, sensor_height: f32) -> f32 {
        sensor_height / (2.0 * (fov.to_radians() * 0.5).tan().max(1e-6))
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            projection: Projection::Perspective,
            field_of_view: 60.0,
            orthographic_size: 5.0,
            near_clip_plane: 0.3,
            far_clip_plane: 1000.0,
            clear_flags: CameraClearFlags::Skybox,
            background_color: Color::rgb(49.0 / 255.0, 77.0 / 255.0, 121.0 / 255.0),
            depth: -1.0,
            viewport_rect: Vector4::new(0.0, 0.0, 1.0, 1.0),
            culling_mask: u32::MAX,
            post_processing: true,
            use_occlusion_culling: true,
            antialiasing: AntialiasingMode::None,
            allow_msaa: true,
            taa_base_blend_factor: 0.875,
            taa_jitter_scale: 1.0,
            taa_variance_clamp_scale: 0.9,
            target_texture: None,
            target_display: 0,
            render_type: CameraRenderType::Base,
            camera_stack: Vec::new(),
            clear_depth: true,
            use_physical_properties: false,
            focal_length: 50.0,
            sensor_size: oxide_math::Vector2::new(36.0, 24.0),
            lens_shift: oxide_math::Vector2::ZERO,
            gate_fit: GateFitMode::Horizontal,
            iso: 200,
            shutter_speed: 0.005,
            aperture: 16.0,
            focus_distance: 10.0,
            blade_count: 5,
            curvature: oxide_math::Vector2::new(2.0, 11.0),
            barrel_clipping: 0.25,
            anamorphism: 0.0,
        }
    }
}

impl Component for Camera {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum LightType {
    Directional,
    #[default]
    Point,
    Spot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum LightShadows {
    None,
    Hard,
    #[default]
    Soft,
}

/// How a light takes part in baked lighting (Unity `LightmapBakeType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum LightmapBakeType {
    /// Lit in real time only; not in lightmaps or probes.
    #[default]
    Realtime,
    /// Direct light in real time, its bounced (indirect) light baked (Baked Indirect).
    Mixed,
    /// Direct and indirect light baked; not rendered in real time (dynamic objects get it
    /// from light probes).
    Baked,
}

/// Where a static renderer gets its baked light (Unity `ReceiveGI`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum ReceiveGI {
    #[default]
    Lightmaps,
    LightProbes,
}

/// Unity `Light`. <https://docs.unity3d.com/Manual/class-Light.html>
#[derive(Clone, Debug, Reflect)]
pub struct Light {
    pub light_type: LightType,
    pub color: Color,
    pub intensity: f32,
    /// Point/Spot reach in world units.
    pub range: f32,
    /// Spot outer cone angle (degrees).
    #[reflect(range(1, 179))]
    pub spot_angle: f32,
    #[reflect(range(0, 179))]
    pub inner_spot_angle: f32,
    pub shadows: LightShadows,
    #[reflect(range(0, 1))]
    pub shadow_strength: f32,
    pub shadow_bias: f32,
    pub shadow_normal_bias: f32,
    pub culling_mask: u32,
    /// Realtime, Mixed or Baked (Light "Mode").
    pub lightmap_bake_type: LightmapBakeType,
}

impl Default for Light {
    fn default() -> Self {
        Self {
            light_type: LightType::Point,
            color: Color::WHITE,
            intensity: 1.0,
            range: 10.0,
            spot_angle: 30.0,
            inner_spot_angle: 21.8,
            shadows: LightShadows::None,
            shadow_strength: 1.0,
            shadow_bias: 0.05,
            shadow_normal_bias: 0.4,
            culling_mask: u32::MAX,
            lightmap_bake_type: LightmapBakeType::Realtime,
        }
    }
}

impl Light {
    /// Unity's default scene "Directional Light": warm white, soft shadows.
    pub fn directional() -> Self {
        Self {
            light_type: LightType::Directional,
            color: Color::rgb(1.0, 0.957, 0.839),
            intensity: 1.0,
            shadows: LightShadows::Soft,
            lightmap_bake_type: LightmapBakeType::Mixed,
            ..Default::default()
        }
    }
}

impl Component for Light {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity `MeshFilter`.
#[derive(Clone, Debug, Default, Reflect)]
pub struct MeshFilter {
    pub mesh: Option<Handle<Mesh>>,
}

impl Component for MeshFilter {
    fn allow_multiple(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum ShadowCastingMode {
    Off,
    #[default]
    On,
    TwoSided,
    ShadowsOnly,
}

/// Unity `MeshRenderer`: draws the sibling MeshFilter's mesh, one material per sub-mesh.
#[derive(Clone, Debug, Reflect)]
pub struct MeshRenderer {
    pub materials: Vec<Handle<Material>>,
    pub cast_shadows: ShadowCastingMode,
    pub receive_shadows: bool,
    /// Occluders may hide this renderer even when its object isn't Occludee Static
    /// (Unity `Renderer.allowOcclusionWhenDynamic`).
    pub allow_occlusion_when_dynamic: bool,
    /// Static (Contribute GI) renderers: lightmaps or light probes.
    pub receive_gi: ReceiveGI,
    /// Relative lightmap resolution (Unity "Scale In Lightmap").
    pub scale_in_lightmap: f32,
    /// Lightmap Parameters asset (Unity "Lightmap Parameters"); else the Lighting Settings'
    /// default.
    pub lightmap_parameters: Option<Handle<LightmapParameters>>,
    /// Set by baking: which lightmap (-1 = none) and where in it.
    pub lightmap_index: i32,
    pub lightmap_scale_offset: Vector4,
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self {
            materials: vec![Handle::from_path(oxide_assets::DEFAULT_MATERIAL)],
            cast_shadows: ShadowCastingMode::On,
            receive_shadows: true,
            allow_occlusion_when_dynamic: true,
            receive_gi: ReceiveGI::Lightmaps,
            scale_in_lightmap: 1.0,
            lightmap_parameters: None,
            lightmap_index: -1,
            lightmap_scale_offset: Vector4::new(1.0, 1.0, 0.0, 0.0),
        }
    }
}

impl MeshRenderer {
    pub fn with_material(m: Handle<Material>) -> Self {
        Self {
            materials: vec![m],
            ..Default::default()
        }
    }
}

impl Component for MeshRenderer {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity `SkinnedMeshRenderer`: a mesh deformed by bone transforms.
#[derive(Clone, Debug, Reflect)]
pub struct SkinnedMeshRenderer {
    pub mesh: Option<Handle<Mesh>>,
    pub materials: Vec<Handle<Material>>,
    /// Bone objects, in the order of the mesh's bone indices / bind poses.
    pub bones: Vec<GameObjectId>,
    pub root_bone: Option<GameObjectId>,
    pub cast_shadows: ShadowCastingMode,
    pub receive_shadows: bool,
    /// Unity `Renderer.allowOcclusionWhenDynamic`.
    pub allow_occlusion_when_dynamic: bool,
}

impl Default for SkinnedMeshRenderer {
    fn default() -> Self {
        Self {
            mesh: None,
            materials: Vec::new(),
            bones: Vec::new(),
            root_bone: None,
            cast_shadows: ShadowCastingMode::On,
            receive_shadows: true,
            allow_occlusion_when_dynamic: true,
        }
    }
}

impl Component for SkinnedMeshRenderer {
    fn allow_multiple(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum FogMode {
    Linear,
    #[default]
    Exponential,
    ExponentialSquared,
}

/// Scene-wide lighting and environment (Unity `RenderSettings`, Lighting window).
/// <https://docs.unity3d.com/Manual/lighting-window.html>
#[derive(Clone, Debug, Reflect)]
pub struct RenderSettings {
    /// Procedural skybox: sky tint, ground color, exposure, sun disk size.
    pub sky_tint: Color,
    pub ground_color: Color,
    pub sky_exposure: f32,
    #[reflect(range(0, 1))]
    pub sun_size: f32,
    /// Atmosphere thickness of the procedural sky.
    #[reflect(range(0, 5))]
    pub atmosphere_thickness: f32,
    /// Ambient light multiplier (Environment Lighting → Intensity Multiplier).
    #[reflect(range(0, 8))]
    pub ambient_intensity: f32,
    /// Reflection intensity multiplier.
    #[reflect(range(0, 1))]
    pub reflection_intensity: f32,
    pub fog: bool,
    pub fog_mode: FogMode,
    pub fog_color: Color,
    pub fog_density: f32,
    pub fog_start_distance: f32,
    pub fog_end_distance: f32,
    /// The directional light used as the sun (defaults to the brightest one).
    pub sun: Option<GameObjectId>,
    /// Exposure applied before tonemapping.
    pub exposure: f32,
    /// Tonemapping when no Volume overrides it.
    pub tonemapping: crate::volume::TonemappingMode,
    pub bloom: bool,
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    /// Vignette strength (0 = off).
    #[reflect(range(0, 1))]
    pub vignette: f32,
    /// Baked lightmaps (Unity `LightmapSettings.lightmaps`), written by Generate Lighting.
    pub lightmaps: Vec<Handle<oxide_assets::Texture2D>>,
    /// Baked light probes: world positions and 27 SH floats (9 RGB coefficients) each.
    pub light_probe_positions: Vec<Vector3>,
    pub light_probe_coefficients: Vec<f32>,
    /// Lightmapping settings (Lighting window): texels per unit.
    pub lightmap_resolution: f32,
    pub max_lightmap_size: u32,
    /// Indirect samples per texel.
    pub bake_samples: u32,
    #[reflect(range(0, 4))]
    pub bake_bounces: u32,
    pub lightmap_padding: u32,
    /// Lighting Settings asset (Unity's per-scene `LightingSettings`); when set, Generate
    /// Lighting uses it instead of the values above.
    pub lighting_settings: Option<Handle<LightingSettings>>,
    /// Screen Space Ambient Occlusion (URP renderer feature).
    pub ssao: bool,
    #[reflect(range(0, 4))]
    pub ssao_intensity: f32,
    /// Sampling radius in world units.
    #[reflect(range(0.05, 2))]
    pub ssao_radius: f32,
    /// How much AO also darkens direct light.
    #[reflect(range(0, 1))]
    pub ssao_direct_lighting_strength: f32,
    /// AO fades out by this distance from the camera.
    pub ssao_falloff_distance: f32,
    #[reflect(range(4, 32))]
    pub ssao_samples: u32,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            sky_tint: Color::rgb(0.5, 0.5, 0.5),
            ground_color: Color::rgb(0.369, 0.349, 0.341),
            sky_exposure: 1.3,
            sun_size: 0.04,
            atmosphere_thickness: 1.0,
            ambient_intensity: 1.0,
            reflection_intensity: 1.0,
            fog: false,
            fog_mode: FogMode::Exponential,
            fog_color: Color::rgb(0.5, 0.5, 0.5),
            fog_density: 0.01,
            fog_start_distance: 0.0,
            fog_end_distance: 300.0,
            sun: None,
            exposure: 1.0,
            tonemapping: crate::volume::TonemappingMode::ACES,
            bloom: true,
            bloom_intensity: 0.4,
            bloom_threshold: 1.0,
            vignette: 0.15,
            lightmaps: Vec::new(),
            light_probe_positions: Vec::new(),
            light_probe_coefficients: Vec::new(),
            lightmap_resolution: 10.0,
            max_lightmap_size: 1024,
            bake_samples: 64,
            bake_bounces: 2,
            lightmap_padding: 2,
            lighting_settings: None,
            ssao: false,
            ssao_intensity: 1.5,
            ssao_radius: 0.35,
            ssao_direct_lighting_strength: 0.25,
            ssao_falloff_distance: 100.0,
            ssao_samples: 12,
        }
    }
}

/// Unity `Vector3.up`-style helper used by built-in scene setup.
pub fn default_sun_rotation() -> Vector3 {
    Vector3::new(50.0, -30.0, 0.0)
}

/// Stand-in for a script component whose type isn't loaded (not compiled yet, renamed or
/// deleted). Keeps the saved fields so nothing is lost; it turns back into the real component
/// when the script loads (Unity's "The associated script can not be loaded").
#[derive(Clone, Default, Reflect)]
pub struct MissingScript {
    /// Type name of the script.
    pub script: String,
    #[reflect(skip)]
    pub data: std::collections::BTreeMap<String, oxide_core::reflect::Value>,
    /// Lifecycle of the real script when it was replaced (so it isn't Awoken twice).
    #[reflect(skip)]
    pub awoken: bool,
    #[reflect(skip)]
    pub started: bool,
}

impl Component for MissingScript {}

/// One level of a [`LODGroup`].
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(default)]
pub struct Lod {
    /// Shown while the group covers at least this fraction of the screen height (Unity
    /// `LOD.screenRelativeTransitionHeight`).
    #[reflect(range(0, 1))]
    pub screen_relative_height: f32,
    /// Objects whose renderers belong to this level.
    pub renderers: Vec<GameObjectId>,
}

impl Default for Lod {
    fn default() -> Self {
        Self {
            screen_relative_height: 0.5,
            renderers: Vec::new(),
        }
    }
}

/// Unity `LODGroup`: draws simpler meshes as the object gets smaller on screen, and nothing
/// once it is smaller than the last level. Levels go from most to least detailed.
/// <https://docs.unity3d.com/Manual/class-LODGroup.html>
#[derive(Clone, Debug, Reflect)]
pub struct LODGroup {
    pub lods: Vec<Lod>,
    /// Object-space size used for the screen-height test (0 = from the renderers' bounds).
    pub size: f32,
}

impl Default for LODGroup {
    fn default() -> Self {
        Self {
            lods: [0.6, 0.3, 0.1]
                .into_iter()
                .map(|h| Lod {
                    screen_relative_height: h,
                    renderers: Vec::new(),
                })
                .collect(),
            size: 0.0,
        }
    }
}

impl Component for LODGroup {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Which level of a group shows at `relative_height` (fraction of screen height); `None`
/// when it is culled.
pub fn lod_level(lods: &[Lod], relative_height: f32) -> Option<usize> {
    lods.iter()
        .position(|l| relative_height >= l.screen_relative_height)
}

/// Unity `OcclusionPortal`: a box that hides what is behind it while closed (a door, a
/// gate). Open it when the door opens. <https://docs.unity3d.com/Manual/class-OcclusionPortal.html>
#[derive(Clone, Debug, Reflect)]
pub struct OcclusionPortal {
    pub open: bool,
    /// Box center, local to the object.
    pub center: Vector3,
    pub size: Vector3,
}

impl Default for OcclusionPortal {
    fn default() -> Self {
        Self {
            open: true,
            center: Vector3::ZERO,
            size: Vector3::ONE,
        }
    }
}

impl Component for OcclusionPortal {
    fn on_draw_gizmos(
        &self,
        world: &crate::World,
        id: GameObjectId,
        gizmos: &mut crate::component::Gizmos,
    ) {
        gizmos.color = if self.open {
            Color::rgb(0.3, 1.0, 0.4)
        } else {
            Color::rgb(1.0, 0.35, 0.3)
        };
        let m = world.local_to_world(id);
        let h = self.size * 0.5;
        let corner = |i: usize| {
            let s = Vector3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            );
            m.transform_point3(self.center + h * s)
        };
        for (a, b) in [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ] {
            gizmos.draw_line(corner(a), corner(b));
        }
    }

    fn allow_multiple(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum ReflectionProbeMode {
    /// Captures only Reflection Probe Static objects, once (and again when edited).
    Baked,
    /// Captures everything on the culling mask, as `refresh_mode` says.
    #[default]
    Realtime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum ReflectionProbeRefreshMode {
    /// When the probe first appears, and again when it moves or its settings change.
    #[default]
    OnAwake,
    EveryFrame,
    /// Only when a script calls [`ReflectionProbe::render_probe`].
    ViaScripting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum ReflectionProbeClearFlags {
    #[default]
    Skybox,
    SolidColor,
}

/// Unity `ReflectionProbe`: captures its surroundings into a cubemap that renderers inside
/// its box use for reflections. <https://docs.unity3d.com/Manual/class-ReflectionProbe.html>
#[derive(Clone, Debug, Reflect)]
pub struct ReflectionProbe {
    pub mode: ReflectionProbeMode,
    pub refresh_mode: ReflectionProbeRefreshMode,
    /// Higher importance wins where boxes overlap; equal importance prefers the smaller box.
    pub importance: i32,
    #[reflect(range(0, 4))]
    pub intensity: f32,
    /// Project reflections onto the box (good for rooms and corridors).
    pub box_projection: bool,
    /// Distance inside the box over which the probe fades into the next one.
    pub blend_distance: f32,
    /// World-aligned box size (not affected by rotation or scale).
    pub size: Vector3,
    /// Box center relative to the object's position.
    pub center: Vector3,
    pub near_clip_plane: f32,
    pub far_clip_plane: f32,
    pub clear_flags: ReflectionProbeClearFlags,
    pub background_color: Color,
    pub culling_mask: u32,
    /// Bumped by [`ReflectionProbe::render_probe`].
    #[reflect(skip)]
    pub render_requests: u32,
}

impl Default for ReflectionProbe {
    fn default() -> Self {
        Self {
            mode: ReflectionProbeMode::Realtime,
            refresh_mode: ReflectionProbeRefreshMode::OnAwake,
            importance: 1,
            intensity: 1.0,
            box_projection: false,
            blend_distance: 1.0,
            size: Vector3::splat(10.0),
            center: Vector3::ZERO,
            near_clip_plane: 0.3,
            far_clip_plane: 1000.0,
            clear_flags: ReflectionProbeClearFlags::Skybox,
            background_color: Color::rgb(49.0 / 255.0, 77.0 / 255.0, 121.0 / 255.0),
            culling_mask: u32::MAX,
            render_requests: 0,
        }
    }
}

impl ReflectionProbe {
    /// Unity `ReflectionProbe.RenderProbe()`: capture again on the next frame (any refresh
    /// mode; the only trigger for `ViaScripting`).
    pub fn render_probe(&mut self) {
        self.render_requests = self.render_requests.wrapping_add(1);
    }

    /// World-space box `(min, max)` for a probe at `position`.
    pub fn bounds(&self, position: Vector3) -> (Vector3, Vector3) {
        let c = position + self.center;
        let h = self.size.abs() * 0.5;
        (c - h, c + h)
    }
}

impl Component for ReflectionProbe {
    fn on_draw_gizmos(
        &self,
        world: &crate::World,
        id: GameObjectId,
        gizmos: &mut crate::component::Gizmos,
    ) {
        gizmos.color = Color::rgb(1.0, 0.85, 0.35);
        let (min, max) = self.bounds(world.position(id));
        let corner = |i: usize| {
            Vector3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            )
        };
        for (a, b) in [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ] {
            gizmos.draw_line(corner(a), corner(b));
        }
    }

    fn allow_multiple(&self) -> bool {
        false
    }
}

/// When a renderer feature runs (Unity `RenderPassEvent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum RenderPassEvent {
    AfterRenderingOpaques,
    AfterRenderingTransparents,
    BeforeRenderingPostProcessing,
    #[default]
    AfterRenderingPostProcessing,
}

/// Depth comparison of a Render Objects feature (Unity `CompareFunction`, the useful ones).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum CompareFunction {
    #[default]
    LessEqual,
    /// Only where hidden behind something: see-through-walls silhouettes.
    Greater,
    Always,
}

/// URP Full Screen Pass Renderer Feature: runs a WGSL shader over the camera image. The
/// `.wgsl` file defines `fn fs_main(in: FullscreenIn) -> vec4f` and can call
/// `sample_source(uv)`, `sample_depth(uv)` and read `fx.params0`, `fx.params1`, `fx.time`.
/// Only Before/After Rendering Post Processing apply.
/// <https://docs.unity3d.com/Manual/urp/renderer-features/renderer-feature-full-screen-pass.html>
#[derive(Clone, Debug, Default, Reflect)]
pub struct FullScreenPassRendererFeature {
    pub injection_point: RenderPassEvent,
    pub shader: Option<Handle<oxide_assets::Shader>>,
    pub params0: Vector4,
    pub params1: Vector4,
}

impl Component for FullScreenPassRendererFeature {}

/// URP Render Objects Renderer Feature: draws renderers on `layer_mask` again at an event,
/// optionally with an override material and another depth test (e.g. Greater with an
/// unlit material for silhouettes of players behind walls).
/// <https://docs.unity3d.com/Manual/urp/renderer-features/how-to-custom-effect-render-objects.html>
#[derive(Clone, Debug, Reflect)]
pub struct RenderObjectsRendererFeature {
    pub event: RenderPassEvent,
    pub layer_mask: u32,
    pub override_material: Option<Handle<Material>>,
    pub depth_test: CompareFunction,
    pub write_depth: bool,
}

impl Default for RenderObjectsRendererFeature {
    fn default() -> Self {
        Self {
            event: RenderPassEvent::AfterRenderingOpaques,
            layer_mask: 0,
            override_material: None,
            depth_test: CompareFunction::LessEqual,
            write_depth: true,
        }
    }
}

impl Component for RenderObjectsRendererFeature {}

/// Unity `LightProbeGroup`: where light probes are baked (positions local to the object).
/// Dynamic objects take their light from the probes around them.
/// <https://docs.unity3d.com/Manual/class-LightProbeGroup.html>
#[derive(Clone, Debug, Reflect)]
pub struct LightProbeGroup {
    pub probe_positions: Vec<Vector3>,
}

impl Default for LightProbeGroup {
    fn default() -> Self {
        // Unity's default: the corners of a 1 m cube.
        let p = |x: f32, y: f32, z: f32| Vector3::new(x, y, z) * 0.5;
        Self {
            probe_positions: vec![
                p(-1.0, -1.0, -1.0),
                p(1.0, -1.0, -1.0),
                p(-1.0, 1.0, -1.0),
                p(1.0, 1.0, -1.0),
                p(-1.0, -1.0, 1.0),
                p(1.0, -1.0, 1.0),
                p(-1.0, 1.0, 1.0),
                p(1.0, 1.0, 1.0),
            ],
        }
    }
}

impl Component for LightProbeGroup {
    fn on_draw_gizmos(
        &self,
        world: &crate::World,
        id: GameObjectId,
        gizmos: &mut crate::component::Gizmos,
    ) {
        gizmos.color = Color::rgb(1.0, 0.9, 0.3);
        let m = world.local_to_world(id);
        for p in &self.probe_positions {
            let c = m.transform_point3(*p);
            for axis in [Vector3::X, Vector3::Y, Vector3::Z] {
                gizmos.draw_line(c - axis * 0.1, c + axis * 0.1);
            }
        }
    }

    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity's Lighting Settings asset (`.lighting`): lightmapping settings shared by scenes.
/// <https://docs.unity3d.com/Manual/class-LightingSettings.html>
#[derive(Clone, Debug, Reflect)]
pub struct LightingSettings {
    /// Bake lightmaps and light probes at all (Unity "Baked Global Illumination").
    pub baked_global_illumination: bool,
    /// Lightmap texels per world unit.
    pub lightmap_resolution: f32,
    pub max_lightmap_size: u32,
    /// Indirect paths per texel.
    pub indirect_samples: u32,
    #[reflect(range(0, 4))]
    pub max_bounces: u32,
    /// Texels between objects in the atlas.
    pub lightmap_padding: u32,
    /// Smooth noise in the indirect light (Unity "Filtering").
    pub filtering: bool,
    /// Used by renderers without their own Lightmap Parameters.
    pub default_lightmap_parameters: Option<Handle<LightmapParameters>>,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            baked_global_illumination: true,
            lightmap_resolution: 10.0,
            max_lightmap_size: 1024,
            indirect_samples: 64,
            max_bounces: 2,
            lightmap_padding: 2,
            filtering: true,
            default_lightmap_parameters: None,
        }
    }
}

impl oxide_assets::Asset for LightingSettings {
    const TYPE_NAME: &'static str = "LightingSettings";
}

/// Unity's Lightmap Parameters asset (`.giparams`): per-renderer baking options.
/// <https://docs.unity3d.com/Manual/class-LightmapParameters.html>
#[derive(Clone, Debug, Reflect)]
pub struct LightmapParameters {
    /// Multiplies the lightmap resolution of renderers using these parameters.
    pub resolution: f32,
}

impl Default for LightmapParameters {
    fn default() -> Self {
        Self { resolution: 1.0 }
    }
}

impl oxide_assets::Asset for LightmapParameters {
    const TYPE_NAME: &'static str = "LightmapParameters";
}

/// Unity `SpriteDrawMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum SpriteDrawMode {
    /// The sprite at its own size.
    #[default]
    Simple,
    /// 9-slice: corners keep their size, edges and centre stretch to `size`.
    Sliced,
    /// The sprite repeats to fill `size`.
    Tiled,
}

/// Unity `SpriteRenderer`: draws a sprite in the world (2D games, billboards, UI-like
/// world elements), sorted by sorting layer and order in layer.
/// <https://docs.unity3d.com/Manual/class-SpriteRenderer.html>
#[derive(Clone, Debug, Reflect)]
pub struct SpriteRenderer {
    pub sprite: Option<Handle<oxide_assets::Sprite>>,
    pub color: oxide_math::Color,
    pub flip_x: bool,
    pub flip_y: bool,
    pub draw_mode: SpriteDrawMode,
    /// World size for Sliced and Tiled.
    pub size: oxide_math::Vector2,
    pub sorting_layer: String,
    pub order_in_layer: i32,
}

impl Default for SpriteRenderer {
    fn default() -> Self {
        Self {
            sprite: None,
            color: oxide_math::Color::WHITE,
            flip_x: false,
            flip_y: false,
            draw_mode: SpriteDrawMode::Simple,
            size: oxide_math::Vector2::new(1.0, 1.0),
            sorting_layer: "Default".into(),
            order_in_layer: 0,
        }
    }
}

impl Component for SpriteRenderer {
    fn allow_multiple(&self) -> bool {
        false
    }
}
