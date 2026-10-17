//! Unity `Material` for the built-in Lit/Unlit shaders, with URP Lit's property names.
//! Reference: <https://docs.unity3d.com/Manual/urp/lit-shader.html>

use crate::handle::{Asset, Handle};
use crate::shader::ShaderGraphRef;
use crate::texture::Texture2D;
use oxide_core::Reflect;
use oxide_math::{Color, Vector2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum ShaderKind {
    /// Physically based (URP "Lit").
    #[default]
    Lit,
    /// No lighting (URP "Unlit").
    Unlit,
    /// Blinn-Phong: diffuse plus a specular highlight in Specular Color (URP "Simple Lit"),
    /// cheaper than Lit on low-end GPUs.
    SimpleLit,
    /// Baked lighting only: lightmaps and light probes, no realtime lights (URP "Baked Lit").
    BakedLit,
    /// Skybox materials (Lighting window "Skybox Material"): Unity's procedural sky (its
    /// look comes from the Lighting settings).
    SkyboxProcedural,
    /// Six textures, one per cube face (`front_tex` … `down_tex`).
    Skybox6Sided,
    /// A Cubemap texture (`cubemap`).
    SkyboxCubemap,
    /// A latitude-longitude panorama (`base_map`).
    SkyboxPanoramic,
}

impl ShaderKind {
    /// Whether this is a skybox shader (not drawn on meshes).
    pub fn is_skybox(self) -> bool {
        matches!(
            self,
            ShaderKind::SkyboxProcedural
                | ShaderKind::Skybox6Sided
                | ShaderKind::SkyboxCubemap
                | ShaderKind::SkyboxPanoramic
        )
    }
}

/// URP Lit "Workflow Mode".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum WorkflowMode {
    /// Metallic and smoothness (metallic map: G roughness, B metallic).
    #[default]
    Metallic,
    /// A specular colour instead of metallic (specular map: RGB specular, A smoothness).
    Specular,
}

/// URP "Surface Type" + "Alpha Clipping".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum SurfaceType {
    #[default]
    Opaque,
    /// Opaque with alpha clipping at `alpha_cutoff`.
    Cutout,
    Transparent,
}

/// A Shader Graph property value on a material (Unity `Material.SetFloat` / `SetColor` /
/// `SetTexture` by reference name).
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct MaterialProperty {
    pub name: String,
    /// Floats use `x`; colours are sRGB like `base_color` (converted on upload).
    pub value: oxide_math::Vector4,
    pub texture: Option<Handle<Texture2D>>,
}

#[derive(Clone, Debug, Reflect)]
pub struct Material {
    pub name: String,
    pub shader: ShaderKind,
    /// Draws with this Shader Graph instead of the built-in Lit/Unlit shader.
    pub shader_graph: Option<Handle<ShaderGraphRef>>,
    /// Values of the Shader Graph's exposed properties (missing ones use the graph's
    /// defaults).
    pub properties: Vec<MaterialProperty>,
    pub surface_type: SurfaceType,
    /// `_BaseColor` (sRGB).
    pub base_color: Color,
    /// `_BaseMap`.
    pub base_map: Option<Handle<Texture2D>>,
    /// `_BaseMap` tiling and offset.
    pub tiling: Vector2,
    pub offset: Vector2,
    /// Lit: metallic or specular workflow.
    pub workflow_mode: WorkflowMode,
    /// `_SpecColor`: Lit's Specular workflow and Simple Lit's highlight (sRGB).
    pub specular_color: Color,
    #[reflect(range(0, 1))]
    pub metallic: f32,
    /// `_Smoothness` = 1 - roughness.
    #[reflect(range(0, 1))]
    pub smoothness: f32,
    /// Packed glTF-style: G = roughness, B = metallic (multiplied with the values above).
    pub metallic_gloss_map: Option<Handle<Texture2D>>,
    /// `_BumpMap` (tangent space, OpenGL / +Y convention, like Unity).
    pub normal_map: Option<Handle<Texture2D>>,
    pub normal_scale: f32,
    /// `_OcclusionMap` (R channel).
    pub occlusion_map: Option<Handle<Texture2D>>,
    #[reflect(range(0, 1))]
    pub occlusion_strength: f32,
    /// `_EmissionColor` (sRGB; HDR intensity in `emission_intensity`).
    pub emission_color: Color,
    pub emission_intensity: f32,
    pub emission_map: Option<Handle<Texture2D>>,
    #[reflect(range(0, 1))]
    pub alpha_cutoff: f32,
    /// URP "Render Face: Both".
    pub double_sided: bool,
    /// URP "Sorting Priority": added to the shader's queue (-50..50).
    #[reflect(range(-50, 50))]
    pub sorting_priority: i32,
    /// Unity `Material.renderQueue`: -1 = From Shader (Geometry 2000, AlphaTest 2450,
    /// Transparent 3000, plus Sorting Priority). Lower queues draw first within the opaque and
    /// the transparent passes.
    pub render_queue: i32,
    /// Whether objects with this material cast shadows (in addition to the renderer setting).
    pub cast_shadows: bool,
    /// Enabled shader keywords (Unity `Material.shaderKeywords`): a Shader Graph's Boolean
    /// keyword `REF` or Enum entry `REF_ENTRY`. Each combination is its own shader variant.
    pub keywords: Vec<String>,
    // Skybox shaders (the tint is `base_color`, Unity's grey 0.5 being neutral).
    /// Skybox brightness (Unity `_Exposure`).
    #[reflect(range(0, 8))]
    pub exposure: f32,
    /// Skybox rotation around the up axis in degrees (Unity `_Rotation`).
    #[reflect(range(0, 360))]
    pub rotation: f32,
    /// Skybox/Cubemap: the cube texture.
    pub cubemap: Option<Handle<crate::cubemap::TextureCube>>,
    /// Skybox/6 Sided faces: Front [+Z], Back [-Z], Left [+X], Right [-X], Up [+Y], Down [-Y].
    pub front_tex: Option<Handle<Texture2D>>,
    pub back_tex: Option<Handle<Texture2D>>,
    pub left_tex: Option<Handle<Texture2D>>,
    pub right_tex: Option<Handle<Texture2D>>,
    pub up_tex: Option<Handle<Texture2D>>,
    pub down_tex: Option<Handle<Texture2D>>,
}

/// Unity's built-in render queues (`UnityEngine.Rendering.RenderQueue`).
pub mod render_queue {
    pub const BACKGROUND: i32 = 1000;
    pub const GEOMETRY: i32 = 2000;
    pub const ALPHA_TEST: i32 = 2450;
    /// The last opaque queue (`GeometryLast`).
    pub const GEOMETRY_LAST: i32 = 2500;
    pub const TRANSPARENT: i32 = 3000;
    pub const OVERLAY: i32 = 4000;
}

impl Material {
    /// The queue this material draws in: `render_queue`, or From Shader (its surface type's
    /// queue plus Sorting Priority).
    pub fn effective_render_queue(&self) -> i32 {
        if self.render_queue >= 0 {
            return self.render_queue.min(5000);
        }
        let base = match self.surface_type {
            SurfaceType::Opaque => render_queue::GEOMETRY,
            SurfaceType::Cutout => render_queue::ALPHA_TEST,
            SurfaceType::Transparent => render_queue::TRANSPARENT,
        };
        base + self.sorting_priority.clamp(-50, 50)
    }

    /// Unity `Material.EnableKeyword`.
    pub fn enable_keyword(&mut self, k: &str) {
        if !self.keywords.iter().any(|x| x == k) {
            self.keywords.push(k.to_owned());
        }
    }
    /// Unity `Material.DisableKeyword`.
    pub fn disable_keyword(&mut self, k: &str) {
        self.keywords.retain(|x| x != k);
    }
    /// Unity `Material.IsKeywordEnabled`.
    pub fn is_keyword_enabled(&self, k: &str) -> bool {
        self.keywords.iter().any(|x| x == k)
    }

    /// Sets a keyword's state: Boolean 0/1 or the Enum entry index (turning off the other
    /// entries).
    pub fn set_keyword_state(&mut self, reference: &str, entries: &[String], state: u32) {
        if entries.is_empty() {
            self.disable_keyword(reference);
        }
        for e in entries {
            self.disable_keyword(&format!("{reference}_{}", e.to_uppercase()));
        }
        if let Some(n) = crate::shader::keyword_names(reference, entries, state) {
            self.enable_keyword(&n);
        }
    }

    /// The built-in Lit shader's keywords this material turns on (Unity sets them from the
    /// material's maps and options): `_NORMALMAP`, `_METALLICSPECGLOSSMAP`, `_OCCLUSIONMAP`,
    /// `_EMISSION`, `_ALPHATEST_ON`.
    pub fn builtin_keywords(&self) -> [(&'static str, bool); 5] {
        [
            ("_NORMALMAP", self.normal_map.is_some()),
            ("_METALLICSPECGLOSSMAP", self.metallic_gloss_map.is_some()),
            ("_OCCLUSIONMAP", self.occlusion_map.is_some()),
            (
                "_EMISSION",
                self.emission_map.is_some()
                    || (self.emission_intensity > 0.0
                        && (self.emission_color.r + self.emission_color.g + self.emission_color.b)
                            > 0.0),
            ),
            ("_ALPHATEST_ON", self.surface_type == SurfaceType::Cutout),
        ]
    }
}

impl Asset for Material {
    const TYPE_NAME: &'static str = "Material";
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: "Default-Material".into(),
            shader: ShaderKind::Lit,
            shader_graph: None,
            properties: Vec::new(),
            surface_type: SurfaceType::Opaque,
            base_color: Color::WHITE,
            base_map: None,
            tiling: Vector2::ONE,
            offset: Vector2::ZERO,
            workflow_mode: WorkflowMode::Metallic,
            specular_color: Color::rgb(0.2, 0.2, 0.2),
            metallic: 0.0,
            smoothness: 0.5,
            metallic_gloss_map: None,
            normal_map: None,
            normal_scale: 1.0,
            occlusion_map: None,
            occlusion_strength: 1.0,
            emission_color: Color::BLACK,
            emission_intensity: 1.0,
            emission_map: None,
            alpha_cutoff: 0.5,
            double_sided: false,
            sorting_priority: 0,
            render_queue: -1,
            cast_shadows: true,
            keywords: Vec::new(),
            exposure: 1.0,
            rotation: 0.0,
            cubemap: None,
            front_tex: None,
            back_tex: None,
            left_tex: None,
            right_tex: None,
            up_tex: None,
            down_tex: None,
        }
    }
}

impl Material {
    pub fn lit(name: &str, color: Color) -> Self {
        Self {
            name: name.into(),
            base_color: color,
            ..Default::default()
        }
    }

    pub fn unlit(name: &str, color: Color) -> Self {
        Self {
            name: name.into(),
            shader: ShaderKind::Unlit,
            base_color: color,
            ..Default::default()
        }
    }

    /// Unity `Material.SetFloat` / `SetVector` / `SetColor` for Shader Graph properties.
    pub fn set_vector(&mut self, name: &str, v: oxide_math::Vector4) {
        match self.properties.iter_mut().find(|p| p.name == name) {
            Some(p) => p.value = v,
            None => self.properties.push(MaterialProperty {
                name: name.to_owned(),
                value: v,
                texture: None,
            }),
        }
    }

    pub fn set_float(&mut self, name: &str, v: f32) {
        self.set_vector(name, oxide_math::Vector4::new(v, 0.0, 0.0, 0.0));
    }

    /// Unity `Material.SetTexture` for a Shader Graph texture property.
    pub fn set_texture(&mut self, name: &str, t: Option<Handle<Texture2D>>) {
        match self.properties.iter_mut().find(|p| p.name == name) {
            Some(p) => p.texture = t,
            None => self.properties.push(MaterialProperty {
                name: name.to_owned(),
                value: oxide_math::Vector4::ZERO,
                texture: t,
            }),
        }
    }

    pub fn property(&self, name: &str) -> Option<&MaterialProperty> {
        self.properties.iter().find(|p| p.name == name)
    }

    pub fn is_transparent(&self) -> bool {
        self.surface_type == SurfaceType::Transparent
    }
}
