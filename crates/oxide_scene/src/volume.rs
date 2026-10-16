//! Post-processing Volumes (URP's Volume framework).
//!
//! A [`Volume`] is global, or local to the colliders on its GameObject (blending in over
//! `blend_distance`). Its profile is the override components on the same GameObject
//! ([`Bloom`], [`Tonemapping`], [`ColorAdjustments`], ...). Each parameter has Unity's
//! override checkbox: only overridden parameters change the result. Volumes are applied in
//! ascending `priority`, each interpolating its overridden parameters toward its values by
//! its weight. <https://docs.unity3d.com/Manual/urp/volumes-landing-page.html>

use crate::component::Component;
use oxide_core::Reflect;
use oxide_core::reflect::{FieldValue, Value, ValueKind};
use oxide_math::{Color, Vector2, Vector4};

/// One parameter of a volume override: its value and whether this volume overrides it
/// (Unity `VolumeParameter<T>` with `overrideState`).
#[derive(Clone, Debug, PartialEq)]
pub struct VolumeParameter<T> {
    pub override_state: bool,
    pub value: T,
}

impl<T> VolumeParameter<T> {
    /// A parameter that is not overridden (shows `value` greyed out in the Inspector).
    pub const fn new(value: T) -> Self {
        Self {
            override_state: false,
            value,
        }
    }

    /// Overrides with `value` (Unity `Override(value)`).
    pub fn set(&mut self, value: T) {
        self.override_state = true;
        self.value = value;
    }
}

/// Serialized as `{ override, value }`. Setting a plain value overrides it.
impl<T: FieldValue> FieldValue for VolumeParameter<T> {
    const KIND: ValueKind = ValueKind::Object;
    const OPTIONS: &'static [&'static str] = T::OPTIONS;
    const TYPE_NAME: Option<&'static str> = T::TYPE_NAME;

    fn to_value(&self) -> Value {
        Value::Object(
            [
                ("override".to_owned(), Value::Bool(self.override_state)),
                ("value".to_owned(), self.value.to_value()),
            ]
            .into_iter()
            .collect(),
        )
    }

    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::Object(mut map) => Some(Self {
                override_state: matches!(map.remove("override"), Some(Value::Bool(true))),
                value: T::from_value(map.remove("value")?)?,
            }),
            v => T::from_value(v).map(|value| Self {
                override_state: true,
                value,
            }),
        }
    }

    fn set_from(&mut self, v: Value) -> bool {
        match v {
            Value::Object(map) => map.into_iter().all(|(k, v)| match (k.as_str(), v) {
                ("override", Value::Bool(b)) => {
                    self.override_state = b;
                    true
                }
                ("value", v) => self.value.set_from(v),
                _ => false,
            }),
            v => {
                let ok = self.value.set_from(v);
                self.override_state |= ok;
                ok
            }
        }
    }
}

/// How a parameter type blends between volumes.
pub trait Interp: Clone {
    /// From `a` toward `b` by `t` (0..1). Defaults to switching to `b` for any weight.
    fn interp(a: &Self, b: &Self, t: f32) -> Self {
        let _ = (a, t);
        b.clone()
    }
}

impl Interp for f32 {
    fn interp(a: &Self, b: &Self, t: f32) -> Self {
        a + (b - a) * t
    }
}

impl Interp for Color {
    fn interp(a: &Self, b: &Self, t: f32) -> Self {
        Color::new(
            a.r + (b.r - a.r) * t,
            a.g + (b.g - a.g) * t,
            a.b + (b.b - a.b) * t,
            a.a + (b.a - a.a) * t,
        )
    }
}

impl Interp for Vector2 {
    fn interp(a: &Self, b: &Self, t: f32) -> Self {
        a.lerp(*b, t)
    }
}

impl Interp for Vector4 {
    fn interp(a: &Self, b: &Self, t: f32) -> Self {
        a.lerp(*b, t)
    }
}

impl Interp for bool {}

impl<T: Interp> VolumeParameter<T> {
    /// Applies this parameter to the accumulated value `into` with volume weight `w`.
    pub fn blend(&self, into: &mut T, w: f32) {
        if self.override_state && w > 0.0 {
            *into = T::interp(into, &self.value, w.min(1.0));
        }
    }
}

/// Unity `Volume`. Local volumes use the Box/Sphere colliders on their GameObject.
/// <https://docs.unity3d.com/Manual/urp/Volumes.html>
#[derive(Clone, Debug, Reflect)]
pub struct Volume {
    /// Applies everywhere (else only inside its colliders).
    pub is_global: bool,
    #[reflect(range(0, 1))]
    pub weight: f32,
    /// Volumes apply in ascending priority; higher wins where they overlap.
    pub priority: f32,
    /// Local volumes fade in over this distance outside their colliders.
    pub blend_distance: f32,
    /// Shared Volume Profile asset (Unity `sharedProfile`). Override components on this
    /// GameObject take precedence over the profile's.
    pub profile: Option<oxide_assets::Handle<VolumeProfile>>,
}

impl Default for Volume {
    fn default() -> Self {
        Self {
            is_global: true,
            weight: 1.0,
            priority: 0.0,
            blend_distance: 0.0,
            profile: None,
        }
    }
}

impl Component for Volume {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Marker for override components (they only act on a GameObject with a [`Volume`]).
macro_rules! volume_override {
    ($t:ty) => {
        impl Component for $t {
            fn allow_multiple(&self) -> bool {
                false
            }
        }
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum TonemappingMode {
    None,
    /// Only remaps the range; keeps hue and saturation.
    Neutral,
    #[default]
    ACES,
}
impl Interp for TonemappingMode {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum DepthOfFieldMode {
    #[default]
    Off,
    /// Blur by distance between `gaussian_start` and `gaussian_end` (cheap).
    Gaussian,
    /// Physical camera: focus distance, focal length and aperture (bokeh shapes).
    Bokeh,
}
impl Interp for DepthOfFieldMode {}

/// URP Bloom override.
#[derive(Clone, Debug, Reflect)]
pub struct Bloom {
    /// Brightness (gamma) above which pixels bloom.
    pub threshold: VolumeParameter<f32>,
    pub intensity: VolumeParameter<f32>,
    /// Spread of the glow (0..1).
    #[reflect(range(0, 1))]
    pub scatter: VolumeParameter<f32>,
    pub tint: VolumeParameter<Color>,
    /// Brightest value that can bloom (stops fireflies).
    pub clamp: VolumeParameter<f32>,
}

impl Default for Bloom {
    fn default() -> Self {
        Self {
            threshold: VolumeParameter::new(0.9),
            intensity: VolumeParameter::new(0.0),
            scatter: VolumeParameter::new(0.7),
            tint: VolumeParameter::new(Color::WHITE),
            clamp: VolumeParameter::new(65472.0),
        }
    }
}
volume_override!(Bloom);

/// URP Tonemapping override.
#[derive(Clone, Debug, Reflect)]
pub struct Tonemapping {
    pub mode: VolumeParameter<TonemappingMode>,
}

impl Default for Tonemapping {
    fn default() -> Self {
        Self {
            mode: VolumeParameter::new(TonemappingMode::None),
        }
    }
}
volume_override!(Tonemapping);

/// URP Color Adjustments override.
#[derive(Clone, Debug, Reflect)]
pub struct ColorAdjustments {
    /// Exposure in EV, applied before tonemapping.
    pub post_exposure: VolumeParameter<f32>,
    #[reflect(range(-100, 100))]
    pub contrast: VolumeParameter<f32>,
    /// Multiplies the image (HDR color).
    pub color_filter: VolumeParameter<Color>,
    #[reflect(range(-180, 180))]
    pub hue_shift: VolumeParameter<f32>,
    #[reflect(range(-100, 100))]
    pub saturation: VolumeParameter<f32>,
}

impl Default for ColorAdjustments {
    fn default() -> Self {
        Self {
            post_exposure: VolumeParameter::new(0.0),
            contrast: VolumeParameter::new(0.0),
            color_filter: VolumeParameter::new(Color::WHITE),
            hue_shift: VolumeParameter::new(0.0),
            saturation: VolumeParameter::new(0.0),
        }
    }
}
volume_override!(ColorAdjustments);

/// URP White Balance override.
#[derive(Clone, Debug, Reflect)]
pub struct WhiteBalance {
    /// Negative = cooler (blue), positive = warmer (yellow).
    #[reflect(range(-100, 100))]
    pub temperature: VolumeParameter<f32>,
    /// Negative = green, positive = magenta.
    #[reflect(range(-100, 100))]
    pub tint: VolumeParameter<f32>,
}

impl Default for WhiteBalance {
    fn default() -> Self {
        Self {
            temperature: VolumeParameter::new(0.0),
            tint: VolumeParameter::new(0.0),
        }
    }
}
volume_override!(WhiteBalance);

/// URP Lift Gamma Gain override. Each is a trackball: `xyz` color (1 = neutral), `w` offset.
#[derive(Clone, Debug, Reflect)]
pub struct LiftGammaGain {
    pub lift: VolumeParameter<Vector4>,
    pub gamma: VolumeParameter<Vector4>,
    pub gain: VolumeParameter<Vector4>,
}

impl Default for LiftGammaGain {
    fn default() -> Self {
        let neutral = Vector4::new(1.0, 1.0, 1.0, 0.0);
        Self {
            lift: VolumeParameter::new(neutral),
            gamma: VolumeParameter::new(neutral),
            gain: VolumeParameter::new(neutral),
        }
    }
}
volume_override!(LiftGammaGain);

/// URP Shadows Midtones Highlights override (trackballs as in [`LiftGammaGain`]).
#[derive(Clone, Debug, Reflect)]
pub struct ShadowsMidtonesHighlights {
    pub shadows: VolumeParameter<Vector4>,
    pub midtones: VolumeParameter<Vector4>,
    pub highlights: VolumeParameter<Vector4>,
    pub shadows_start: VolumeParameter<f32>,
    pub shadows_end: VolumeParameter<f32>,
    pub highlights_start: VolumeParameter<f32>,
    pub highlights_end: VolumeParameter<f32>,
}

impl Default for ShadowsMidtonesHighlights {
    fn default() -> Self {
        let neutral = Vector4::new(1.0, 1.0, 1.0, 0.0);
        Self {
            shadows: VolumeParameter::new(neutral),
            midtones: VolumeParameter::new(neutral),
            highlights: VolumeParameter::new(neutral),
            shadows_start: VolumeParameter::new(0.0),
            shadows_end: VolumeParameter::new(0.3),
            highlights_start: VolumeParameter::new(0.55),
            highlights_end: VolumeParameter::new(1.0),
        }
    }
}
volume_override!(ShadowsMidtonesHighlights);

/// URP Split Toning override: tints shadows and highlights (grey = no tint).
#[derive(Clone, Debug, Reflect)]
pub struct SplitToning {
    pub shadows: VolumeParameter<Color>,
    pub highlights: VolumeParameter<Color>,
    #[reflect(range(-100, 100))]
    pub balance: VolumeParameter<f32>,
}

impl Default for SplitToning {
    fn default() -> Self {
        Self {
            shadows: VolumeParameter::new(Color::rgb(0.5, 0.5, 0.5)),
            highlights: VolumeParameter::new(Color::rgb(0.5, 0.5, 0.5)),
            balance: VolumeParameter::new(0.0),
        }
    }
}
volume_override!(SplitToning);

/// URP Channel Mixer override: each output channel as percentages of the inputs.
#[derive(Clone, Debug, Reflect)]
pub struct ChannelMixer {
    pub red_out_red_in: VolumeParameter<f32>,
    pub red_out_green_in: VolumeParameter<f32>,
    pub red_out_blue_in: VolumeParameter<f32>,
    pub green_out_red_in: VolumeParameter<f32>,
    pub green_out_green_in: VolumeParameter<f32>,
    pub green_out_blue_in: VolumeParameter<f32>,
    pub blue_out_red_in: VolumeParameter<f32>,
    pub blue_out_green_in: VolumeParameter<f32>,
    pub blue_out_blue_in: VolumeParameter<f32>,
}

impl Default for ChannelMixer {
    fn default() -> Self {
        let p = VolumeParameter::new;
        Self {
            red_out_red_in: p(100.0),
            red_out_green_in: p(0.0),
            red_out_blue_in: p(0.0),
            green_out_red_in: p(0.0),
            green_out_green_in: p(100.0),
            green_out_blue_in: p(0.0),
            blue_out_red_in: p(0.0),
            blue_out_green_in: p(0.0),
            blue_out_blue_in: p(100.0),
        }
    }
}
volume_override!(ChannelMixer);

/// URP Vignette override.
#[derive(Clone, Debug, Reflect)]
pub struct Vignette {
    pub color: VolumeParameter<Color>,
    /// Viewport position of the center (0.5, 0.5 = middle).
    pub center: VolumeParameter<Vector2>,
    #[reflect(range(0, 1))]
    pub intensity: VolumeParameter<f32>,
    #[reflect(range(0.01, 1))]
    pub smoothness: VolumeParameter<f32>,
    /// Perfectly round instead of following the screen shape.
    pub rounded: VolumeParameter<bool>,
}

impl Default for Vignette {
    fn default() -> Self {
        Self {
            color: VolumeParameter::new(Color::BLACK),
            center: VolumeParameter::new(Vector2::new(0.5, 0.5)),
            intensity: VolumeParameter::new(0.0),
            smoothness: VolumeParameter::new(0.2),
            rounded: VolumeParameter::new(false),
        }
    }
}
volume_override!(Vignette);

/// URP Chromatic Aberration override.
#[derive(Clone, Debug, Reflect)]
pub struct ChromaticAberration {
    #[reflect(range(0, 1))]
    pub intensity: VolumeParameter<f32>,
}

impl Default for ChromaticAberration {
    fn default() -> Self {
        Self {
            intensity: VolumeParameter::new(0.0),
        }
    }
}
volume_override!(ChromaticAberration);

/// URP Film Grain override (procedural grain).
#[derive(Clone, Debug, Reflect)]
pub struct FilmGrain {
    #[reflect(range(0, 1))]
    pub intensity: VolumeParameter<f32>,
    /// How much less grain bright areas get.
    #[reflect(range(0, 1))]
    pub response: VolumeParameter<f32>,
}

impl Default for FilmGrain {
    fn default() -> Self {
        Self {
            intensity: VolumeParameter::new(0.0),
            response: VolumeParameter::new(0.8),
        }
    }
}
volume_override!(FilmGrain);

/// URP Lens Distortion override (barrel > 0, pincushion < 0).
#[derive(Clone, Debug, Reflect)]
pub struct LensDistortion {
    #[reflect(range(-1, 1))]
    pub intensity: VolumeParameter<f32>,
    #[reflect(range(0, 1))]
    pub x_multiplier: VolumeParameter<f32>,
    #[reflect(range(0, 1))]
    pub y_multiplier: VolumeParameter<f32>,
    pub center: VolumeParameter<Vector2>,
    /// Zoom to hide the distorted borders.
    #[reflect(range(0.01, 5))]
    pub scale: VolumeParameter<f32>,
}

impl Default for LensDistortion {
    fn default() -> Self {
        Self {
            intensity: VolumeParameter::new(0.0),
            x_multiplier: VolumeParameter::new(1.0),
            y_multiplier: VolumeParameter::new(1.0),
            center: VolumeParameter::new(Vector2::new(0.5, 0.5)),
            scale: VolumeParameter::new(1.0),
        }
    }
}
volume_override!(LensDistortion);

/// URP Depth Of Field override.
#[derive(Clone, Debug, Reflect)]
pub struct DepthOfField {
    pub mode: VolumeParameter<DepthOfFieldMode>,
    /// Gaussian: distance where blur starts.
    pub gaussian_start: VolumeParameter<f32>,
    /// Gaussian: distance where blur reaches its maximum.
    pub gaussian_end: VolumeParameter<f32>,
    /// Gaussian: maximum blur radius (in multiples of a 1080p pixel × 4).
    #[reflect(range(0.5, 1.5))]
    pub gaussian_max_radius: VolumeParameter<f32>,
    /// Bokeh: distance in focus.
    pub focus_distance: VolumeParameter<f32>,
    /// Bokeh: lens focal length in mm.
    #[reflect(range(1, 300))]
    pub focal_length: VolumeParameter<f32>,
    /// Bokeh: f-number (lower = more blur).
    #[reflect(range(1, 32))]
    pub aperture: VolumeParameter<f32>,
}

impl Default for DepthOfField {
    fn default() -> Self {
        Self {
            mode: VolumeParameter::new(DepthOfFieldMode::Off),
            gaussian_start: VolumeParameter::new(10.0),
            gaussian_end: VolumeParameter::new(30.0),
            gaussian_max_radius: VolumeParameter::new(1.0),
            focus_distance: VolumeParameter::new(10.0),
            focal_length: VolumeParameter::new(50.0),
            aperture: VolumeParameter::new(5.6),
        }
    }
}
volume_override!(DepthOfField);

/// URP Motion Blur override (camera motion).
#[derive(Clone, Debug, Reflect)]
pub struct MotionBlur {
    #[reflect(range(0, 1))]
    pub intensity: VolumeParameter<f32>,
    /// Longest blur as a fraction of the screen.
    #[reflect(range(0, 0.2))]
    pub clamp: VolumeParameter<f32>,
}

impl Default for MotionBlur {
    fn default() -> Self {
        Self {
            intensity: VolumeParameter::new(0.0),
            clamp: VolumeParameter::new(0.05),
        }
    }
}
volume_override!(MotionBlur);

/// Every override type with its Add Component path.
pub fn register(reg: &mut crate::ComponentRegistry) {
    reg.register::<Volume>("Miscellaneous/Volume");
    let o = "Rendering/Volume Overrides/";
    reg.register::<Bloom>(&format!("{o}Bloom"));
    reg.register::<Tonemapping>(&format!("{o}Tonemapping"));
    reg.register::<ColorAdjustments>(&format!("{o}Color Adjustments"));
    reg.register::<WhiteBalance>(&format!("{o}White Balance"));
    reg.register::<LiftGammaGain>(&format!("{o}Lift Gamma Gain"));
    reg.register::<ShadowsMidtonesHighlights>(&format!("{o}Shadows Midtones Highlights"));
    reg.register::<SplitToning>(&format!("{o}Split Toning"));
    reg.register::<ChannelMixer>(&format!("{o}Channel Mixer"));
    reg.register::<Vignette>(&format!("{o}Vignette"));
    reg.register::<ChromaticAberration>(&format!("{o}Chromatic Aberration"));
    reg.register::<FilmGrain>(&format!("{o}Film Grain"));
    reg.register::<LensDistortion>(&format!("{o}Lens Distortion"));
    reg.register::<DepthOfField>(&format!("{o}Depth Of Field"));
    reg.register::<MotionBlur>(&format!("{o}Motion Blur"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_core::reflect::Reflect as _;

    #[test]
    fn parameter_override_round_trip() {
        let mut b = Bloom::default();
        assert!(!b.intensity.override_state);
        // A plain value overrides; an object sets both parts.
        b.set_field("intensity", Value::Float(2.0)).unwrap();
        assert_eq!(
            b.intensity,
            VolumeParameter {
                override_state: true,
                value: 2.0
            }
        );
        let v = b.get_field("intensity").unwrap();
        let mut c = Bloom::default();
        c.set_field("intensity", v).unwrap();
        assert_eq!(c.intensity, b.intensity);
        c.set_field(
            "intensity",
            Value::Object(
                [("override".to_owned(), Value::Bool(false))]
                    .into_iter()
                    .collect(),
            ),
        )
        .unwrap();
        assert!(!c.intensity.override_state);
        assert_eq!(c.intensity.value, 2.0);
    }

    #[test]
    fn blending_only_touches_overridden_parameters() {
        let mut intensity = 1.0;
        let mut threshold = 0.9;
        let mut b = Bloom::default();
        b.intensity.set(3.0);
        b.intensity.blend(&mut intensity, 0.5);
        b.threshold.blend(&mut threshold, 0.5);
        assert_eq!(intensity, 2.0);
        assert_eq!(threshold, 0.9);
        let mut mode = TonemappingMode::ACES;
        let mut t = Tonemapping::default();
        t.mode.set(TonemappingMode::Neutral);
        t.mode.blend(&mut mode, 0.1);
        assert_eq!(mode, TonemappingMode::Neutral);
    }
}

/// Unity's Volume Profile asset (`.volumeProfile`): a reusable set of overrides that
/// Volumes point to. Each slot is empty until an override is added.
/// <https://docs.unity3d.com/Manual/urp/Volume-Profile.html>
#[derive(Clone, Debug, Default, Reflect)]
pub struct VolumeProfile {
    pub bloom: Option<Bloom>,
    pub tonemapping: Option<Tonemapping>,
    pub color_adjustments: Option<ColorAdjustments>,
    pub white_balance: Option<WhiteBalance>,
    pub lift_gamma_gain: Option<LiftGammaGain>,
    pub shadows_midtones_highlights: Option<ShadowsMidtonesHighlights>,
    pub split_toning: Option<SplitToning>,
    pub channel_mixer: Option<ChannelMixer>,
    pub vignette: Option<Vignette>,
    pub chromatic_aberration: Option<ChromaticAberration>,
    pub film_grain: Option<FilmGrain>,
    pub lens_distortion: Option<LensDistortion>,
    pub depth_of_field: Option<DepthOfField>,
    pub motion_blur: Option<MotionBlur>,
}

impl oxide_assets::Asset for VolumeProfile {
    const TYPE_NAME: &'static str = "VolumeProfile";
}
