//! Unity URP's Lens Flare (SRP): the Lens Flare data asset (`.lensflare`, Unity
//! `LensFlareDataSRP`) is a list of elements; a Lens Flare component (Unity
//! `LensFlareComponentSRP`) on a GameObject or Light draws it.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.render-pipelines.universal@17.0/manual/shared/lens-flare/lens-flare-component.html>

use crate::component::Component;
use oxide_assets::{Asset, Handle, Texture2D};
use oxide_core::{AnimationCurve, Gradient, Reflect};
use oxide_math::{Color, Vector2};

/// Unity `SRPLensFlareType`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LensFlareType {
    /// A texture.
    Image,
    #[default]
    Circle,
    /// A regular polygon (Side Count, Roundness).
    Polygon,
    /// A circle outline (thickness from Edge Offset).
    Ring,
}

/// Unity `SRPLensFlareBlendMode`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LensFlareBlendMode {
    #[default]
    Additive,
    Screen,
    Premultiply,
    Lerp,
}

/// Unity `SRPLensFlareDistribution`: how copies of a "Multiple Elements" element spread.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LensFlareDistribution {
    /// Evenly along Length Spread.
    #[default]
    Uniform,
    /// Placed and scaled by the Position and Scale curves.
    Curve,
    /// Random positions, colours, sizes and rotations from Seed.
    Random,
}

/// Unity `LensFlareDataElementSRP`.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct LensFlareDataElement {
    pub visible: bool,
    pub flare_type: LensFlareType,
    /// Image type: the texture drawn.
    pub texture: Option<Handle<Texture2D>>,
    /// Image type: keep the texture's width to height ratio.
    pub preserve_aspect_ratio: bool,
    /// Linear HDR tint.
    pub tint: Color,
    pub intensity: f32,
    pub blend_mode: LensFlareBlendMode,
    /// Multiplies by the Light's colour (when the component sits on a Light).
    pub modulate_by_light_color: bool,

    // Transform
    /// Along the ray from the source (0) through the screen centre (1) and beyond.
    pub position: f32,
    /// Offset in screen-height units.
    pub position_offset: Vector2,
    /// Rotates the ray around the source (degrees).
    pub angular_offset: f32,
    /// Scales how far along the ray the element moves, per axis.
    pub translation_scale: Vector2,
    pub uniform_scale: f32,
    /// Width and height; 1 is a tenth of the screen height.
    pub size_xy: Vector2,
    /// Degrees.
    pub rotation: f32,
    /// Turns the element to face the source.
    pub auto_rotate: bool,

    // Shape (Circle, Polygon, Ring)
    /// How the shape fades toward its edge: 1 is linear, lower values glow more softly.
    #[reflect(range(0, 1))]
    pub fall_off: f32,
    /// Where the falloff starts (0 = centre, 1 = hard edge); ring thickness for Rings.
    #[reflect(range(0, 1))]
    pub edge_offset: f32,
    /// Polygon: 0 = sharp corners, 1 = a circle.
    #[reflect(range(0, 1))]
    pub sdf_roundness: f32,
    #[reflect(range(3, 32))]
    pub side_count: u32,
    /// Draws outside the shape instead of inside.
    pub inverse_sdf: bool,

    // Multiple Elements
    pub allow_multiple_element: bool,
    #[reflect(range(1, 64))]
    pub count: u32,
    pub distribution: LensFlareDistribution,
    /// How far along the ray the copies spread.
    pub length_spread: f32,
    /// Colour of each copy across the spread.
    pub color_gradient: Gradient,
    /// Curve distribution: position along the spread by copy (0..1).
    pub position_curve: AnimationCurve,
    /// Curve distribution: scale by copy (0..1).
    pub scale_curve: AnimationCurve,
    /// Random distribution.
    pub seed: u32,
    #[reflect(range(0, 1))]
    pub intensity_variation: f32,
    pub position_variation: Vector2,
    #[reflect(range(0, 1))]
    pub scale_variation: f32,
    /// Degrees.
    pub rotation_variation: f32,

    // Radial Distortion
    pub enable_radial_distortion: bool,
    /// Size at the screen edge.
    pub target_size_distortion: Vector2,
    /// Blend toward the target size by distance (0..1).
    pub distortion_curve: AnimationCurve,
    /// Distance from the screen centre (else from the source).
    pub distortion_relative_to_center: bool,
}

impl Default for LensFlareDataElement {
    fn default() -> Self {
        Self {
            visible: true,
            flare_type: LensFlareType::Circle,
            texture: None,
            preserve_aspect_ratio: false,
            tint: Color::WHITE,
            intensity: 1.0,
            blend_mode: LensFlareBlendMode::Additive,
            modulate_by_light_color: true,
            position: 0.0,
            position_offset: Vector2::ZERO,
            angular_offset: 0.0,
            translation_scale: Vector2::ONE,
            uniform_scale: 1.0,
            size_xy: Vector2::ONE,
            rotation: 0.0,
            auto_rotate: false,
            fall_off: 1.0,
            edge_offset: 0.1,
            sdf_roundness: 0.0,
            side_count: 6,
            inverse_sdf: false,
            allow_multiple_element: false,
            count: 5,
            distribution: LensFlareDistribution::Uniform,
            length_spread: 1.0,
            color_gradient: Gradient::default(),
            position_curve: AnimationCurve::linear(0.0, 0.0, 1.0, 1.0),
            scale_curve: AnimationCurve::constant(0.0, 1.0, 1.0),
            seed: 0,
            intensity_variation: 0.75,
            position_variation: Vector2::new(1.0, 0.0),
            scale_variation: 1.0,
            rotation_variation: 180.0,
            enable_radial_distortion: false,
            target_size_distortion: Vector2::ONE,
            distortion_curve: AnimationCurve::linear(0.0, 0.0, 1.0, 1.0),
            distortion_relative_to_center: false,
        }
    }
}

/// Unity `LensFlareDataSRP` (`.lensflare`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct LensFlareData {
    pub elements: Vec<LensFlareDataElement>,
}

impl Asset for LensFlareData {
    const TYPE_NAME: &'static str = "LensFlareData";
}

impl Default for LensFlareData {
    /// A starter flare (glow, halo ring, a hexagon ghost and a trail of circles), so a
    /// new asset shows something at once; Unity's is empty.
    fn default() -> Self {
        let e = LensFlareDataElement::default;
        Self {
            elements: vec![
                LensFlareDataElement {
                    size_xy: Vector2::splat(2.5),
                    fall_off: 0.35,
                    edge_offset: 0.0,
                    tint: Color::new(1.0, 0.9, 0.75, 1.0),
                    ..e()
                },
                LensFlareDataElement {
                    flare_type: LensFlareType::Ring,
                    size_xy: Vector2::splat(4.0),
                    edge_offset: 0.06,
                    fall_off: 0.6,
                    intensity: 0.25,
                    tint: Color::new(0.6, 0.8, 1.0, 1.0),
                    ..e()
                },
                LensFlareDataElement {
                    flare_type: LensFlareType::Polygon,
                    position: 1.4,
                    size_xy: Vector2::splat(1.2),
                    edge_offset: 0.7,
                    sdf_roundness: 0.15,
                    intensity: 0.3,
                    tint: Color::new(0.5, 1.0, 0.6, 1.0),
                    ..e()
                },
                LensFlareDataElement {
                    position: 0.4,
                    size_xy: Vector2::splat(0.5),
                    edge_offset: 0.6,
                    intensity: 0.35,
                    allow_multiple_element: true,
                    count: 4,
                    length_spread: 1.6,
                    color_gradient: Gradient::two(
                        Color::new(1.0, 0.6, 0.3, 1.0),
                        Color::new(0.4, 0.6, 1.0, 1.0),
                    ),
                    ..e()
                },
            ],
        }
    }
}

/// Unity `LensFlareComponentSRP`. On a Light it follows the light: directional lights
/// flare from their direction, spot lights only inside their cone.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct LensFlareComponent {
    pub lens_flare_data: Option<Handle<LensFlareData>>,
    pub intensity: f32,
    /// Distance at which Distance Attenuation Curve reaches its end.
    pub max_attenuation_distance: f32,
    pub distance_attenuation_curve: AnimationCurve,
    /// Distance at which Scale By Distance Curve reaches its end.
    pub max_attenuation_scale: f32,
    pub scale_by_distance_curve: AnimationCurve,
    /// Spot lights fade outside their cone.
    pub attenuation_by_light_shape: bool,
    /// Intensity by the source's distance from the screen centre (0) to the edge (1).
    pub radial_screen_attenuation_curve: AnimationCurve,
    pub use_occlusion: bool,
    /// World units (degrees for directional lights) sampled around the source.
    pub occlusion_radius: f32,
    #[reflect(range(1, 64))]
    pub sample_count: u32,
    /// Moves the tested point toward the camera.
    pub occlusion_offset: f32,
    pub scale: f32,
    pub allow_off_screen: bool,
}

impl Default for LensFlareComponent {
    fn default() -> Self {
        Self {
            lens_flare_data: None,
            intensity: 1.0,
            max_attenuation_distance: 100.0,
            distance_attenuation_curve: AnimationCurve::linear(0.0, 1.0, 1.0, 0.0),
            max_attenuation_scale: 100.0,
            scale_by_distance_curve: AnimationCurve::linear(0.0, 1.0, 1.0, 0.0),
            attenuation_by_light_shape: true,
            radial_screen_attenuation_curve: AnimationCurve::constant(0.0, 1.0, 1.0),
            use_occlusion: true,
            occlusion_radius: 0.1,
            sample_count: 32,
            occlusion_offset: 0.05,
            scale: 1.0,
            allow_off_screen: false,
        }
    }
}

impl Component for LensFlareComponent {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_data_round_trips_through_reflection() {
        let data = LensFlareData::default();
        use oxide_core::reflect::FieldValue as _;
        let v = data.to_value();
        let mut back = LensFlareData { elements: vec![] };
        assert!(back.set_from(v));
        assert_eq!(back, data);
    }
}
