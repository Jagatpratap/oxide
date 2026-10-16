//! Unity `QualitySettings`: named quality levels, one of them active, saved with the project
//! (`ProjectSettings/QualitySettings.yaml`).
//! <https://docs.unity3d.com/Manual/class-QualitySettings.html>

use crate::MsaaQuality;
use oxide_core::Reflect;

/// Unity `ShadowQuality`: which realtime shadows are drawn at this level.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Default, Reflect, serde::Serialize, serde::Deserialize,
)]
pub enum ShadowQuality {
    Disable,
    HardOnly,
    #[default]
    All,
}

/// One quality level (a column of Unity's Quality settings matrix).
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct QualityLevel {
    pub name: String,
    /// Realtime shadows: off, hard only (soft lights render hard), or all.
    pub shadows: ShadowQuality,
    /// Max distance from the camera that shadows are drawn.
    pub shadow_distance: f32,
    /// Multiplies LOD transition distances: 2 keeps detail twice as far.
    pub lod_bias: f32,
    /// LOD levels below this are never used (0 = all levels).
    #[reflect(range(0, 7))]
    pub maximum_lod_level: u32,
    /// Multisample anti-aliasing for cameras that allow it (URP asset MSAA).
    pub msaa: MsaaQuality,
    /// Wait for the display's vertical blank (no tearing, frame rate capped to the display).
    pub vsync: bool,
    /// Realtime reflection probes update; when off they keep their last capture.
    pub realtime_reflection_probes: bool,
}

impl QualityLevel {
    fn new(
        name: &str,
        shadows: ShadowQuality,
        shadow_distance: f32,
        lod_bias: f32,
        msaa: MsaaQuality,
    ) -> Self {
        Self {
            name: name.into(),
            shadows,
            shadow_distance,
            lod_bias,
            maximum_lod_level: 0,
            msaa,
            vsync: true,
            realtime_reflection_probes: true,
        }
    }
}

impl Default for QualityLevel {
    fn default() -> Self {
        QualitySettings::default().levels[QualitySettings::DEFAULT_LEVEL].clone()
    }
}

/// The project's quality levels and the active one (Unity `QualitySettings`).
#[derive(Clone, Debug, PartialEq)]
pub struct QualitySettings {
    pub levels: Vec<QualityLevel>,
    current: usize,
}

impl Default for QualitySettings {
    /// Unity's six default levels; "High" is active.
    fn default() -> Self {
        use MsaaQuality::*;
        use ShadowQuality::*;
        let mut very_low = QualityLevel::new("Very Low", Disable, 20.0, 0.3, Disabled);
        very_low.maximum_lod_level = 1;
        very_low.realtime_reflection_probes = false;
        let mut low = QualityLevel::new("Low", HardOnly, 40.0, 0.5, Disabled);
        low.realtime_reflection_probes = false;
        Self {
            levels: vec![
                very_low,
                low,
                QualityLevel::new("Medium", All, 70.0, 0.7, Msaa2x),
                QualityLevel::new("High", All, 150.0, 1.0, Msaa4x),
                QualityLevel::new("Very High", All, 250.0, 1.5, Msaa4x),
                QualityLevel::new("Ultra", All, 400.0, 2.0, Msaa8x),
            ],
            current: Self::DEFAULT_LEVEL,
        }
    }
}

impl QualitySettings {
    pub const DEFAULT_LEVEL: usize = 3;

    pub fn new(levels: Vec<QualityLevel>, current: usize) -> Self {
        let mut q = Self { levels, current: 0 };
        if q.levels.is_empty() {
            q = Self::default();
        }
        q.set_quality_level(current);
        q
    }

    /// Unity `QualitySettings.GetQualityLevel`.
    pub fn quality_level(&self) -> usize {
        self.current
    }

    /// Unity `QualitySettings.SetQualityLevel` (clamped to the existing levels).
    pub fn set_quality_level(&mut self, index: usize) {
        self.current = index.min(self.levels.len().saturating_sub(1));
    }

    /// Unity `QualitySettings.names`.
    pub fn names(&self) -> Vec<&str> {
        self.levels.iter().map(|l| l.name.as_str()).collect()
    }

    /// The active level's settings.
    pub fn current(&self) -> &QualityLevel {
        &self.levels[self.current]
    }

    pub fn current_mut(&mut self) -> &mut QualityLevel {
        &mut self.levels[self.current]
    }

    /// Settings of the World's active level (defaults when there's no QualitySettings).
    pub fn of(world: &crate::World) -> QualityLevel {
        world
            .try_resource::<QualitySettings>()
            .map_or_else(QualityLevel::default, |q| q.current().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_clamp_and_default() {
        let mut q = QualitySettings::default();
        assert_eq!(q.current().name, "High");
        q.set_quality_level(99);
        assert_eq!(q.current().name, "Ultra");
        let q = QualitySettings::new(Vec::new(), 1);
        assert_eq!(q.names().len(), 6);
        assert_eq!(q.quality_level(), 1);
    }
}
