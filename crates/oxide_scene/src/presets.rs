//! Unity's curve and gradient preset libraries (`.curves`, `.gradients`): named curves and
//! gradients the Curve and Gradient Editors offer as presets, saved as project assets so
//! a tuned curve (an engine torque curve, a tyre grip curve) can be reused anywhere.
//! Reference: <https://docs.unity3d.com/Manual/EditingCurves.html> (Presets)

use oxide_assets::Asset;
use oxide_core::Reflect;
use oxide_core::curves::{AnimationCurve, Gradient};

#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct CurvePreset {
    pub name: String,
    pub curve: AnimationCurve,
}

/// Unity `CurvePresetLibrary`.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct CurvePresetLibrary {
    pub presets: Vec<CurvePreset>,
}

impl Asset for CurvePresetLibrary {
    const TYPE_NAME: &'static str = "CurvePresetLibrary";
}

#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct GradientPreset {
    pub name: String,
    pub gradient: Gradient,
}

/// Unity `GradientPresetLibrary`.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct GradientPresetLibrary {
    pub presets: Vec<GradientPreset>,
}

impl Asset for GradientPresetLibrary {
    const TYPE_NAME: &'static str = "GradientPresetLibrary";
}
