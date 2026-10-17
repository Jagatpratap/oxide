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
    /// Draw order tweak within a queue (Unity `renderQueue` offset).
    pub sorting_priority: i32,
    /// Whether objects with this material cast shadows (in addition to the renderer setting).
    pub cast_shadows: bool,
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
            cast_shadows: true,
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
