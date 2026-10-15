//! Unity `Material` for the built-in Lit/Unlit shaders, with URP Lit's property names.
//! Reference: <https://docs.unity3d.com/Manual/urp/lit-shader.html>

use crate::handle::{Asset, Handle};
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

#[derive(Clone, Debug, Reflect)]
pub struct Material {
    pub name: String,
    pub shader: ShaderKind,
    pub surface_type: SurfaceType,
    /// `_BaseColor` (sRGB).
    pub base_color: Color,
    /// `_BaseMap`.
    pub base_map: Option<Handle<Texture2D>>,
    /// `_BaseMap` tiling and offset.
    pub tiling: Vector2,
    pub offset: Vector2,
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
            surface_type: SurfaceType::Opaque,
            base_color: Color::WHITE,
            base_map: None,
            tiling: Vector2::ONE,
            offset: Vector2::ZERO,
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

    pub fn is_transparent(&self) -> bool {
        self.surface_type == SurfaceType::Transparent
    }
}
