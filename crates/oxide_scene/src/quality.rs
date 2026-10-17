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

/// Unity `UpscalingFilterSelection`: how a scaled render is stretched to the screen.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Default, Reflect, serde::Serialize, serde::Deserialize,
)]
pub enum UpscalingFilter {
    /// Smooth (bilinear).
    #[default]
    Linear,
    /// Sharp pixels (nearest).
    Point,
}

/// Unity's global texture mipmap limit ("Texture Quality").
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Default, Reflect, serde::Serialize, serde::Deserialize,
)]
pub enum TextureQuality {
    #[default]
    FullRes,
    HalfRes,
    QuarterRes,
    EighthRes,
}

impl TextureQuality {
    /// Mip levels skipped at upload.
    pub fn mip_limit(self) -> u32 {
        self as u32
    }

    pub fn from_mip_limit(n: u32) -> Self {
        match n {
            0 => Self::FullRes,
            1 => Self::HalfRes,
            2 => Self::QuarterRes,
            _ => Self::EighthRes,
        }
    }
}

/// Unity `AnisotropicFiltering`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Default, Reflect, serde::Serialize, serde::Deserialize,
)]
pub enum AnisotropicFiltering {
    /// No texture uses anisotropic filtering.
    Disable,
    /// Textures use their own Aniso Level (trilinear textures).
    #[default]
    PerTexture,
    /// Every filtered texture uses at least 9× (Unity's forced minimum).
    ForceEnable,
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
    /// Game cameras render at this fraction of the screen size and are stretched up (URP
    /// Render Scale); the UI stays sharp. The Scene view is never scaled.
    #[reflect(range(0.1, 2.0))]
    pub render_scale: f32,
    /// How a scaled render is stretched to the screen.
    pub upscaling_filter: UpscalingFilter,
    /// Global texture mipmap limit: lower levels skip the largest mips (less GPU memory).
    pub texture_quality: TextureQuality,
    pub anisotropic_textures: AnisotropicFiltering,
    /// Most realtime point and spot lights drawn per frame (URP Additional Lights limit);
    /// the brightest and nearest to the camera win.
    #[reflect(range(0, 256))]
    pub additional_lights_limit: u32,
    /// Overrides Graphics Settings' default render pipeline at this level (Unity's
    /// per-level "Render Pipeline Asset").
    pub render_pipeline:
        Option<oxide_assets::Handle<crate::render_pipeline::UniversalRenderPipelineAsset>>,
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
            render_scale: 1.0,
            upscaling_filter: UpscalingFilter::Linear,
            texture_quality: TextureQuality::FullRes,
            anisotropic_textures: AnisotropicFiltering::PerTexture,
            additional_lights_limit: 256,
            render_pipeline: None,
        }
    }

    /// This level with Adaptive Performance's scaling applied.
    pub fn scaled(&self, o: &QualityOverrides) -> Self {
        let mut l = self.clone();
        l.render_scale = (l.render_scale * o.render_scale).clamp(0.1, 2.0);
        l.lod_bias *= o.lod_bias;
        l.shadow_distance *= o.shadow_distance;
        l.texture_quality =
            TextureQuality::from_mip_limit(l.texture_quality.mip_limit() + o.extra_mip_limit);
        if let Some(cap) = o.max_msaa
            && (l.msaa as u32) > (cap as u32)
        {
            l.msaa = cap;
        }
        if let Some(cap) = o.max_shadows
            && (l.shadows as u32) > (cap as u32)
        {
            l.shadows = cap;
        }
        l.additional_lights_limit = l.additional_lights_limit.min(o.additional_lights_limit);
        l
    }
}

/// Runtime scaling on top of the active level, written by Adaptive Performance's scalers
/// (identity by default). [`QualitySettings::of`] applies it.
#[derive(Clone, Debug, PartialEq)]
pub struct QualityOverrides {
    pub render_scale: f32,
    pub lod_bias: f32,
    pub shadow_distance: f32,
    /// Extra mip levels skipped on top of Texture Quality.
    pub extra_mip_limit: u32,
    pub max_msaa: Option<MsaaQuality>,
    pub max_shadows: Option<ShadowQuality>,
    pub additional_lights_limit: u32,
}

impl Default for QualityOverrides {
    fn default() -> Self {
        Self {
            render_scale: 1.0,
            lod_bias: 1.0,
            shadow_distance: 1.0,
            extra_mip_limit: 0,
            max_msaa: None,
            max_shadows: None,
            additional_lights_limit: u32::MAX,
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
        very_low.texture_quality = TextureQuality::HalfRes;
        very_low.anisotropic_textures = AnisotropicFiltering::Disable;
        very_low.additional_lights_limit = 4;
        let mut low = QualityLevel::new("Low", HardOnly, 40.0, 0.5, Disabled);
        low.realtime_reflection_probes = false;
        low.anisotropic_textures = AnisotropicFiltering::Disable;
        low.additional_lights_limit = 8;
        let mut ultra = QualityLevel::new("Ultra", All, 400.0, 2.0, Msaa8x);
        ultra.anisotropic_textures = AnisotropicFiltering::ForceEnable;
        Self {
            levels: vec![
                very_low,
                low,
                QualityLevel::new("Medium", All, 70.0, 0.7, Msaa2x),
                QualityLevel::new("High", All, 150.0, 1.0, Msaa4x),
                QualityLevel::new("Very High", All, 250.0, 1.5, Msaa4x),
                ultra,
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

    /// Settings of the World's active level (defaults when there's no QualitySettings), with
    /// Adaptive Performance's [`QualityOverrides`] applied.
    pub fn of(world: &crate::World) -> QualityLevel {
        let level = world
            .try_resource::<QualitySettings>()
            .map_or_else(QualityLevel::default, |q| q.current().clone());
        match world.try_resource::<QualityOverrides>() {
            Some(o) => level.scaled(o),
            None => level,
        }
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

    #[test]
    fn overrides_scale_and_cap() {
        let high = QualitySettings::default().current().clone();
        let o = QualityOverrides {
            render_scale: 0.5,
            shadow_distance: 0.5,
            extra_mip_limit: 5,
            max_msaa: Some(MsaaQuality::Msaa2x),
            max_shadows: Some(ShadowQuality::HardOnly),
            additional_lights_limit: 3,
            ..Default::default()
        };
        let s = high.scaled(&o);
        assert_eq!(s.render_scale, 0.5);
        assert_eq!(s.shadow_distance, 75.0);
        assert_eq!(s.texture_quality, TextureQuality::EighthRes);
        assert_eq!(s.msaa, MsaaQuality::Msaa2x);
        assert_eq!(s.shadows, ShadowQuality::HardOnly);
        assert_eq!(s.additional_lights_limit, 3);
        assert_eq!(high.scaled(&QualityOverrides::default()), high);
    }
}
