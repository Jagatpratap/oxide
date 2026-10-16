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

/// Unity `Camera`. <https://docs.unity3d.com/Manual/class-Camera.html>
#[derive(Clone, Debug, Reflect)]
pub struct Camera {
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
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self {
            materials: vec![Handle::from_path(oxide_assets::DEFAULT_MATERIAL)],
            cast_shadows: ShadowCastingMode::On,
            receive_shadows: true,
            allow_occlusion_when_dynamic: true,
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
    pub bloom: bool,
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    /// Max distance shadows are drawn (Quality → Shadow Distance).
    pub shadow_distance: f32,
    /// Vignette strength (0 = off).
    #[reflect(range(0, 1))]
    pub vignette: f32,
    /// Multiplies LOD transition distances (Unity `QualitySettings.lodBias`): 2 keeps
    /// detail twice as far.
    pub lod_bias: f32,
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
            bloom: true,
            bloom_intensity: 0.4,
            bloom_threshold: 1.0,
            shadow_distance: 150.0,
            vignette: 0.15,
            lod_bias: 1.0,
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
