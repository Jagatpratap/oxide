//! Unity Adaptive Performance: watches the device's thermal state and the CPU and GPU frame
//! times, works out what limits the frame rate, and has an indexer step quality scalers up
//! and down (render scale, LOD, shadows, MSAA, lights, texture quality, frame rate) so the
//! game holds its target frame rate and avoids thermal throttling.
//! Settings live in `ProjectSettings/AdaptivePerformance.yaml`; the scalers act through
//! [`QualityOverrides`], on top of the active Quality level.
//! <https://docs.unity3d.com/Packages/com.unity.adaptiveperformance@5.1/manual/index.html>

use oxide_core::Reflect;
use oxide_scene::{MsaaQuality, QualityOverrides, QualitySettings, ShadowQuality, World};
use serde::{Deserialize, Serialize};

/// Unity `WarningLevel`.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Reflect, Serialize, Deserialize,
)]
pub enum WarningLevel {
    #[default]
    NoWarning,
    /// The device will throttle soon unless it does less work.
    ThrottlingImminent,
    /// The device is throttling (CPU/GPU clocks are lowered).
    Throttling,
}

/// Unity `ThermalMetrics`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
pub struct ThermalMetrics {
    pub warning_level: WarningLevel,
    /// 0 (cool) to 1 (throttling).
    pub temperature_level: f32,
    /// Change of `temperature_level` per second (smoothed): positive while heating up.
    pub temperature_trend: f32,
}

/// Unity `PerformanceBottleneck`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub enum PerformanceBottleneck {
    /// Not enough frames measured yet.
    #[default]
    Unknown,
    /// The CPU work takes longer than the target frame time.
    CPU,
    /// The GPU work takes longer than the target frame time.
    GPU,
    /// Both finish in time; the frame waits for the target frame rate (or vsync).
    TargetFrameRate,
}

/// Unity `FrameTiming` (seconds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameTiming {
    pub current_frame_time: f32,
    pub average_frame_time: f32,
    pub current_cpu_frame_time: f32,
    pub average_cpu_frame_time: f32,
    /// 0 when the GPU can't time its passes.
    pub current_gpu_frame_time: f32,
    pub average_gpu_frame_time: f32,
}

/// Unity `StateAction`: what the indexer decided this frame.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Reflect, Serialize, Deserialize,
)]
pub enum StateAction {
    /// There's headroom: quality can go back up.
    Increase,
    #[default]
    Stale,
    /// Over budget or warm: lower quality.
    Decrease,
    /// Throttling: lower quality two steps at a time.
    FastDecrease,
}

/// Unity `ScalerVisualImpact`: the indexer lowers low-impact scalers first and restores
/// high-impact ones first.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Reflect, Serialize, Deserialize,
)]
pub enum ScalerVisualImpact {
    Low,
    #[default]
    Medium,
    High,
}

/// Unity `ScalerTarget`: which bottleneck a scaler relieves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct ScalerTarget {
    pub cpu: bool,
    pub gpu: bool,
    /// Pixel work (resolution); counts as GPU.
    pub fill_rate: bool,
}

impl ScalerTarget {
    const fn new(cpu: bool, gpu: bool, fill_rate: bool) -> Self {
        Self {
            cpu,
            gpu,
            fill_rate,
        }
    }

    fn relieves(self, b: PerformanceBottleneck) -> bool {
        match b {
            PerformanceBottleneck::CPU => self.cpu,
            PerformanceBottleneck::GPU => self.gpu || self.fill_rate,
            _ => false,
        }
    }
}

/// One scaler's settings (Unity's per-scaler Indexer settings).
#[derive(Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[serde(default)]
pub struct ScalerSettings {
    pub enabled: bool,
    /// Scale at the highest level (most reduced).
    #[reflect(range(0.0, 1.0))]
    pub min_bound: f32,
    /// Scale at level 0 (full quality).
    #[reflect(range(0.0, 1.0))]
    pub max_bound: f32,
    /// Number of steps between `max_bound` and `min_bound`.
    #[reflect(range(1, 10))]
    pub max_level: u32,
    pub visual_impact: ScalerVisualImpact,
    pub target: ScalerTarget,
}

impl Default for ScalerSettings {
    fn default() -> Self {
        Self::new(
            true,
            0.5,
            1.0,
            4,
            ScalerVisualImpact::Medium,
            ScalerTarget::new(false, true, false),
        )
    }
}

impl ScalerSettings {
    const fn new(
        enabled: bool,
        min_bound: f32,
        max_bound: f32,
        max_level: u32,
        visual_impact: ScalerVisualImpact,
        target: ScalerTarget,
    ) -> Self {
        Self {
            enabled,
            min_bound,
            max_bound,
            max_level,
            visual_impact,
            target,
        }
    }

    /// Unity `AdaptivePerformanceScaler.Scale` at `level`.
    pub fn scale(&self, level: u32) -> f32 {
        let t = level.min(self.max_level) as f32 / self.max_level.max(1) as f32;
        self.max_bound + (self.min_bound - self.max_bound) * t
    }
}

/// The scalers, in the order of [`ScalerKind::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
pub enum ScalerKind {
    /// Render Scale of game cameras.
    Resolution,
    /// LOD bias.
    Lod,
    ShadowDistance,
    /// Soft → hard → no realtime shadows.
    ShadowQuality,
    /// MSAA 8× → … → off.
    Msaa,
    /// Additional (point and spot) lights limit.
    Lights,
    /// Extra texture mipmap limit (textures re-upload when it changes).
    TextureQuality,
    /// Application.targetFrameRate (thermal only).
    Framerate,
}

impl ScalerKind {
    pub const ALL: [ScalerKind; 8] = [
        ScalerKind::Resolution,
        ScalerKind::Lod,
        ScalerKind::ShadowDistance,
        ScalerKind::ShadowQuality,
        ScalerKind::Msaa,
        ScalerKind::Lights,
        ScalerKind::TextureQuality,
        ScalerKind::Framerate,
    ];

    /// Unity's scaler class name.
    pub fn name(self) -> &'static str {
        match self {
            ScalerKind::Resolution => "AdaptiveResolution",
            ScalerKind::Lod => "AdaptiveLOD",
            ScalerKind::ShadowDistance => "AdaptiveShadowDistance",
            ScalerKind::ShadowQuality => "AdaptiveShadowQuality",
            ScalerKind::Msaa => "AdaptiveMSAA",
            ScalerKind::Lights => "AdaptiveLights",
            ScalerKind::TextureQuality => "AdaptiveTextureQuality",
            ScalerKind::Framerate => "AdaptiveFramerate",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase();
        Self::ALL.into_iter().find(|k| {
            k.name().to_ascii_lowercase() == n
                || k.name()["Adaptive".len()..].to_ascii_lowercase() == n
                || format!("{k:?}").to_ascii_lowercase() == n.replace('_', "")
        })
    }
}

/// Indexer settings (Unity `AdaptivePerformanceIndexerSettings`).
#[derive(Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexerSettings {
    /// The indexer changes scaler levels by itself.
    pub active: bool,
    /// Seconds between two thermal-driven changes.
    #[reflect(range(0.5, 60.0))]
    pub thermal_action_delay: f32,
    /// Seconds between two performance-driven changes.
    #[reflect(range(0.5, 60.0))]
    pub performance_action_delay: f32,
    /// Quality only goes back up while CPU and GPU both use less than this fraction of the
    /// target frame time.
    #[reflect(range(0.3, 0.95))]
    pub headroom: f32,
}

impl Default for IndexerSettings {
    fn default() -> Self {
        Self {
            active: true,
            thermal_action_delay: 10.0,
            performance_action_delay: 4.0,
            headroom: 0.7,
        }
    }
}

/// Project Settings > Adaptive Performance.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[serde(default)]
pub struct AdaptivePerformanceSettings {
    /// Initialize Adaptive Performance on startup (Play mode in the editor, and players).
    pub enabled: bool,
    /// Log thermal, bottleneck and scaler changes to the Console.
    pub logging: bool,
    /// Frame rate to hold when Application.targetFrameRate isn't set (0: the display's).
    #[reflect(range(0, 240))]
    pub default_target_frame_rate: u32,
    pub indexer: IndexerSettings,
    pub resolution: ScalerSettings,
    pub lod: ScalerSettings,
    pub shadow_distance: ScalerSettings,
    pub shadow_quality: ScalerSettings,
    pub msaa: ScalerSettings,
    pub lights: ScalerSettings,
    pub texture_quality: ScalerSettings,
    /// min/max bound × 120 fps are the lowest and highest target frame rates.
    pub framerate: ScalerSettings,
}

impl Default for AdaptivePerformanceSettings {
    fn default() -> Self {
        use ScalerVisualImpact::*;
        let gpu = ScalerTarget::new(false, true, false);
        Self {
            enabled: true,
            logging: false,
            default_target_frame_rate: 0,
            indexer: IndexerSettings::default(),
            resolution: ScalerSettings::new(
                true,
                0.6,
                1.0,
                4,
                Medium,
                ScalerTarget::new(false, false, true),
            ),
            lod: ScalerSettings::new(true, 0.4, 1.0, 3, Low, ScalerTarget::new(true, true, false)),
            shadow_distance: ScalerSettings::new(true, 0.3, 1.0, 3, Low, gpu),
            shadow_quality: ScalerSettings::new(true, 0.0, 1.0, 2, High, gpu),
            msaa: ScalerSettings::new(
                true,
                0.0,
                1.0,
                3,
                Medium,
                ScalerTarget::new(false, false, true),
            ),
            lights: ScalerSettings::new(
                true,
                0.25,
                1.0,
                3,
                Medium,
                ScalerTarget::new(true, true, false),
            ),
            texture_quality: ScalerSettings::new(false, 0.0, 1.0, 2, High, gpu),
            framerate: ScalerSettings::new(
                false,
                0.25,
                0.5,
                2,
                High,
                ScalerTarget::new(true, true, false),
            ),
        }
    }
}

impl AdaptivePerformanceSettings {
    pub fn scaler(&self, k: ScalerKind) -> &ScalerSettings {
        match k {
            ScalerKind::Resolution => &self.resolution,
            ScalerKind::Lod => &self.lod,
            ScalerKind::ShadowDistance => &self.shadow_distance,
            ScalerKind::ShadowQuality => &self.shadow_quality,
            ScalerKind::Msaa => &self.msaa,
            ScalerKind::Lights => &self.lights,
            ScalerKind::TextureQuality => &self.texture_quality,
            ScalerKind::Framerate => &self.framerate,
        }
    }
}

/// Something that changed this frame (Unity's `ThermalEvent`,
/// `PerformanceBottleneckChangeEvent` and scaler level changes).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum AdaptivePerformanceEvent {
    Thermal(ThermalMetrics),
    BottleneckChanged(PerformanceBottleneck),
    ScalerLevelChanged { scaler: ScalerKind, level: u32 },
}

/// Simulated device state (Unity's Device Simulator Adaptive Performance extension).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Simulation {
    pub thermal: Option<ThermalMetrics>,
    pub bottleneck: Option<PerformanceBottleneck>,
}

const SCALERS: usize = ScalerKind::ALL.len();
/// Frames averaged before a bottleneck is reported.
const WARMUP_FRAMES: u32 = 30;

/// The running Adaptive Performance state (Unity `IAdaptivePerformance`), a World resource.
#[derive(Clone, Debug)]
pub struct AdaptivePerformance {
    /// Running (enabled in settings, and Play mode or a player).
    active: bool,
    thermal: ThermalMetrics,
    bottleneck: PerformanceBottleneck,
    timing: FrameTiming,
    target_frame_rate: f32,
    action: StateAction,
    levels: [u32; SCALERS],
    /// Scaler levels set by hand (Unity `OverrideLevel`); they stay until cleared.
    overrides: [Option<u32>; SCALERS],
    samples: u32,
    last_change: f64,
    last_thermal_sample: f64,
    /// Application.targetFrameRate before the Framerate scaler changed it.
    original_target_frame_rate: Option<i32>,
    events: Vec<AdaptivePerformanceEvent>,
    pub simulation: Simulation,
}

impl Default for AdaptivePerformance {
    fn default() -> Self {
        Self {
            active: false,
            thermal: ThermalMetrics::default(),
            bottleneck: PerformanceBottleneck::Unknown,
            timing: FrameTiming::default(),
            target_frame_rate: 60.0,
            action: StateAction::Stale,
            levels: [0; SCALERS],
            overrides: [None; SCALERS],
            samples: 0,
            last_change: f64::NEG_INFINITY,
            last_thermal_sample: f64::NEG_INFINITY,
            original_target_frame_rate: None,
            events: Vec::new(),
            simulation: Simulation::default(),
        }
    }
}

impl AdaptivePerformance {
    /// Unity `IAdaptivePerformance.Active`.
    pub fn active(&self) -> bool {
        self.active
    }

    /// Unity `ThermalStatus.ThermalMetrics`.
    pub fn thermal_metrics(&self) -> ThermalMetrics {
        self.thermal
    }

    /// Unity `PerformanceStatus.PerformanceMetrics.PerformanceBottleneck`.
    pub fn performance_bottleneck(&self) -> PerformanceBottleneck {
        self.bottleneck
    }

    /// Unity `PerformanceStatus.FrameTiming`.
    pub fn frame_timing(&self) -> FrameTiming {
        self.timing
    }

    /// The frame rate the indexer aims for.
    pub fn target_frame_rate(&self) -> f32 {
        self.target_frame_rate
    }

    /// The indexer's latest decision (Unity `Indexer.PerformanceAction`/`ThermalAction`, the
    /// stronger of the two).
    pub fn action(&self) -> StateAction {
        self.action
    }

    /// Unity `AdaptivePerformanceScaler.CurrentLevel`.
    pub fn level(&self, k: ScalerKind) -> u32 {
        self.overrides[k as usize].unwrap_or(self.levels[k as usize])
    }

    /// Unity `AdaptivePerformanceScaler.OverrideLevel`: pins a level (None hands the scaler
    /// back to the indexer).
    pub fn override_level(&mut self, k: ScalerKind, level: Option<u32>) {
        self.overrides[k as usize] = level;
    }

    pub fn overridden(&self, k: ScalerKind) -> Option<u32> {
        self.overrides[k as usize]
    }

    /// Changes since the previous frame.
    pub fn events(&self) -> &[AdaptivePerformanceEvent] {
        &self.events
    }

    /// Puts every scaler back to full quality.
    pub fn reset(&mut self) {
        self.levels = [0; SCALERS];
        self.samples = 0;
    }

    /// Feeds one frame's measurements and runs the indexer. `cpu`/`gpu` are the frame's
    /// work times and `frame` the whole frame time (seconds); `now` is real time.
    fn step(
        &mut self,
        s: &AdaptivePerformanceSettings,
        now: f64,
        frame: f32,
        cpu: f32,
        gpu: Option<f32>,
        thermal: ThermalMetrics,
    ) {
        self.events.clear();
        // Thermal state, with a smoothed trend.
        let thermal = self.simulation.thermal.unwrap_or(thermal);
        let dt = (now - self.last_thermal_sample).clamp(1e-3, 1.0) as f32;
        let trend = (thermal.temperature_level - self.thermal.temperature_level) / dt;
        self.last_thermal_sample = now;
        let mut thermal = thermal;
        thermal.temperature_trend = self.thermal.temperature_trend * 0.9 + trend * 0.1;
        if thermal.warning_level != self.thermal.warning_level
            || (thermal.temperature_level - self.thermal.temperature_level).abs() > 0.05
        {
            self.events.push(AdaptivePerformanceEvent::Thermal(thermal));
        }
        self.thermal = thermal;

        // Frame times: exponential averages over roughly half a second.
        let t = &mut self.timing;
        let k = if self.samples == 0 { 1.0 } else { 0.06 };
        let avg = |a: f32, x: f32| a + (x - a) * k;
        t.current_frame_time = frame;
        t.current_cpu_frame_time = cpu;
        t.current_gpu_frame_time = gpu.unwrap_or(0.0);
        t.average_frame_time = avg(t.average_frame_time, frame);
        t.average_cpu_frame_time = avg(t.average_cpu_frame_time, cpu);
        if let Some(g) = gpu {
            t.average_gpu_frame_time = avg(t.average_gpu_frame_time, g);
        }
        self.samples = self.samples.saturating_add(1);

        // What limits the frame rate.
        let budget = 1.0 / self.target_frame_rate.max(1.0);
        let (c, g) = (t.average_cpu_frame_time, t.average_gpu_frame_time);
        let measured = if self.samples < WARMUP_FRAMES {
            PerformanceBottleneck::Unknown
        } else if c.max(g) > budget * 0.95 {
            if g > c {
                PerformanceBottleneck::GPU
            } else {
                PerformanceBottleneck::CPU
            }
        } else {
            PerformanceBottleneck::TargetFrameRate
        };
        let bottleneck = self.simulation.bottleneck.unwrap_or(measured);
        if bottleneck != self.bottleneck {
            self.events
                .push(AdaptivePerformanceEvent::BottleneckChanged(bottleneck));
            self.bottleneck = bottleneck;
        }

        // The indexer.
        let thermal_action = match thermal.warning_level {
            WarningLevel::Throttling => StateAction::FastDecrease,
            WarningLevel::ThrottlingImminent => StateAction::Decrease,
            WarningLevel::NoWarning if thermal.temperature_level > 0.6 => StateAction::Stale,
            WarningLevel::NoWarning => StateAction::Increase,
        };
        let headroom = c.max(g) < budget * s.indexer.headroom;
        let perf_action = match bottleneck {
            PerformanceBottleneck::CPU | PerformanceBottleneck::GPU => StateAction::Decrease,
            PerformanceBottleneck::TargetFrameRate
                if headroom || self.simulation.bottleneck.is_some() =>
            {
                StateAction::Increase
            }
            _ => StateAction::Stale,
        };
        self.action = thermal_action.max(perf_action);
        if !s.indexer.active {
            return;
        }
        let thermal_driven = thermal_action >= perf_action && thermal_action > StateAction::Stale;
        let delay = if thermal_driven {
            s.indexer.thermal_action_delay
        } else {
            s.indexer.performance_action_delay
        } as f64;
        let since = now - self.last_change;
        let changed = match self.action {
            StateAction::FastDecrease if since >= delay * 0.5 => {
                let a = self.lower(s, None);
                let b = self.lower(s, None);
                a || b
            }
            StateAction::Decrease if since >= delay => {
                let relieve = (!thermal_driven).then_some(bottleneck);
                self.lower(s, relieve)
            }
            // Raising quality waits longer so it doesn't bounce.
            StateAction::Increase if since >= delay * 2.0 => self.raise(s),
            _ => false,
        };
        if changed {
            self.last_change = now;
            // New settings: measure afresh before judging them.
            self.samples = self.samples.min(WARMUP_FRAMES / 2);
        }
    }

    fn adjustable(&self, s: &AdaptivePerformanceSettings, k: ScalerKind) -> bool {
        s.scaler(k).enabled && self.overrides[k as usize].is_none()
    }

    /// Raises one scaler's level (less quality): the lowest visual impact first, among the
    /// scalers that relieve `bottleneck` (any scaler for thermal pressure).
    fn lower(
        &mut self,
        s: &AdaptivePerformanceSettings,
        bottleneck: Option<PerformanceBottleneck>,
    ) -> bool {
        let pick = ScalerKind::ALL
            .into_iter()
            .filter(|&k| self.adjustable(s, k) && self.levels[k as usize] < s.scaler(k).max_level)
            .filter(|&k| match bottleneck {
                Some(b) => s.scaler(k).target.relieves(b),
                // The frame rate only drops for heat (Unity's Adaptive Framerate).
                None => true,
            })
            .filter(|&k| k != ScalerKind::Framerate || bottleneck.is_none())
            .min_by(|&a, &b| {
                let ra = self.levels[a as usize] as f32 / s.scaler(a).max_level.max(1) as f32;
                let rb = self.levels[b as usize] as f32 / s.scaler(b).max_level.max(1) as f32;
                (s.scaler(a).visual_impact, ra)
                    .partial_cmp(&(s.scaler(b).visual_impact, rb))
                    .unwrap()
            });
        self.change(pick, 1)
    }

    /// Lowers one scaler's level (more quality): the highest visual impact first.
    fn raise(&mut self, s: &AdaptivePerformanceSettings) -> bool {
        let pick = ScalerKind::ALL
            .into_iter()
            .filter(|&k| self.overrides[k as usize].is_none() && self.levels[k as usize] > 0)
            .max_by(|&a, &b| {
                let ra = self.levels[a as usize] as f32 / s.scaler(a).max_level.max(1) as f32;
                let rb = self.levels[b as usize] as f32 / s.scaler(b).max_level.max(1) as f32;
                (s.scaler(a).visual_impact, ra)
                    .partial_cmp(&(s.scaler(b).visual_impact, rb))
                    .unwrap()
            });
        self.change(pick, -1)
    }

    fn change(&mut self, k: Option<ScalerKind>, by: i32) -> bool {
        let Some(k) = k else { return false };
        let l = &mut self.levels[k as usize];
        *l = l.saturating_add_signed(by);
        self.events
            .push(AdaptivePerformanceEvent::ScalerLevelChanged {
                scaler: k,
                level: *l,
            });
        true
    }

    /// The quality changes the current scaler levels make.
    pub fn overrides(&self, s: &AdaptivePerformanceSettings, base_lights: u32) -> QualityOverrides {
        let mut o = QualityOverrides::default();
        for k in ScalerKind::ALL {
            let cfg = s.scaler(k);
            let level = self.level(k).min(cfg.max_level);
            if !cfg.enabled || level == 0 {
                continue;
            }
            let scale = cfg.scale(level);
            let frac = level as f32 / cfg.max_level.max(1) as f32;
            match k {
                ScalerKind::Resolution => o.render_scale = scale,
                ScalerKind::Lod => o.lod_bias = scale,
                ScalerKind::ShadowDistance => o.shadow_distance = scale,
                ScalerKind::ShadowQuality => {
                    o.max_shadows = Some(if frac >= 1.0 {
                        ShadowQuality::Disable
                    } else {
                        ShadowQuality::HardOnly
                    })
                }
                ScalerKind::Msaa => {
                    let order = [
                        MsaaQuality::Msaa8x,
                        MsaaQuality::Msaa4x,
                        MsaaQuality::Msaa2x,
                        MsaaQuality::Disabled,
                    ];
                    let i = ((frac * 3.0).round() as usize).min(3);
                    o.max_msaa = Some(order[i]);
                }
                ScalerKind::Lights => {
                    o.additional_lights_limit =
                        ((base_lights.min(64) as f32 * scale).round() as u32).max(1)
                }
                ScalerKind::TextureQuality => o.extra_mip_limit = level,
                ScalerKind::Framerate => {}
            }
        }
        o
    }
}

/// The device's thermal state: macOS/iOS `NSProcessInfo.thermalState` (nominal, fair,
/// serious, critical); other platforms report none.
pub fn device_thermal() -> ThermalMetrics {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState as S};
        let state = NSProcessInfo::processInfo().thermalState();
        let (warning_level, temperature_level) = if state == S::Critical {
            (WarningLevel::Throttling, 1.0)
        } else if state == S::Serious {
            (WarningLevel::ThrottlingImminent, 0.8)
        } else if state == S::Fair {
            (WarningLevel::NoWarning, 0.5)
        } else {
            (WarningLevel::NoWarning, 0.2)
        };
        ThermalMetrics {
            warning_level,
            temperature_level,
            temperature_trend: 0.0,
        }
    }
    #[cfg(not(target_os = "macos"))]
    ThermalMetrics::default()
}

/// Once per frame, after rendering: measures, runs the indexer and applies the scalers.
/// `running` is false in Edit mode (scalers go back to full quality).
pub fn update(
    world: &mut World,
    running: bool,
    now: f64,
    frame: f32,
    cpu: f32,
    gpu: Option<f32>,
    display_hz: f32,
) {
    let settings = world
        .try_resource::<AdaptivePerformanceSettings>()
        .cloned()
        .unwrap_or_default();
    if world.try_resource::<AdaptivePerformance>().is_none() {
        world.resources.insert(AdaptivePerformance::default());
    }
    let app_target = world.resource::<crate::AppControl>().target_frame_rate;
    let base_lights = world
        .try_resource::<QualitySettings>()
        .map_or(256, |q| q.current().additional_lights_limit);
    let ap = world.resource_mut::<AdaptivePerformance>();
    let on = running && settings.enabled;
    if !on {
        let was = ap.active;
        ap.active = false;
        ap.events.clear();
        if was {
            ap.reset();
            ap.overrides = [None; SCALERS];
            let original = ap.original_target_frame_rate.take();
            world.resources.remove::<QualityOverrides>();
            if let Some(fps) = original {
                world.resource_mut::<crate::AppControl>().target_frame_rate = fps;
            }
        }
        return;
    }
    ap.active = true;
    let own_target = ap.original_target_frame_rate.unwrap_or(app_target);
    ap.target_frame_rate = if own_target > 0 {
        own_target as f32
    } else if settings.default_target_frame_rate > 0 {
        settings.default_target_frame_rate as f32
    } else {
        display_hz.max(30.0)
    };
    let thermal = device_thermal();
    ap.step(&settings, now, frame, cpu, gpu, thermal);
    if settings.logging {
        for e in &ap.events {
            oxide_core::debug_log!("Adaptive Performance: {e:?}");
        }
    }
    let overrides = ap.overrides(&settings, base_lights);
    // Adaptive Framerate: Application.targetFrameRate between the bounds (× 120 fps).
    let fr = settings.framerate;
    let fr_level = ap.level(ScalerKind::Framerate).min(fr.max_level);
    let fps = if fr.enabled && fr_level > 0 {
        if ap.original_target_frame_rate.is_none() {
            ap.original_target_frame_rate = Some(app_target);
        }
        Some((fr.scale(fr_level) * 120.0).round().max(1.0) as i32)
    } else {
        ap.original_target_frame_rate.take()
    };
    if let Some(fps) = fps {
        world.resource_mut::<crate::AppControl>().target_frame_rate = fps;
    }
    if overrides == QualityOverrides::default() {
        world.resources.remove::<QualityOverrides>();
    } else {
        world.resources.insert(overrides);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cool() -> ThermalMetrics {
        ThermalMetrics::default()
    }

    /// Runs `secs` of 60 Hz frames with the given CPU/GPU times.
    fn run(
        ap: &mut AdaptivePerformance,
        s: &AdaptivePerformanceSettings,
        t: &mut f64,
        secs: f64,
        cpu: f32,
        gpu: f32,
        th: ThermalMetrics,
    ) {
        let end = *t + secs;
        while *t < end {
            *t += 1.0 / 60.0;
            ap.step(s, *t, 1.0 / 60.0, cpu, Some(gpu), th);
        }
    }

    #[test]
    fn gpu_bound_lowers_gpu_scalers_then_recovers() {
        let s = AdaptivePerformanceSettings::default();
        let mut ap = AdaptivePerformance {
            active: true,
            ..Default::default()
        };
        let mut t = 0.0;
        // 25 ms of GPU work against a 16.7 ms budget.
        run(&mut ap, &s, &mut t, 1.0, 0.005, 0.025, cool());
        assert_eq!(ap.performance_bottleneck(), PerformanceBottleneck::GPU);
        run(&mut ap, &s, &mut t, 20.0, 0.005, 0.025, cool());
        // Low visual impact GPU scalers go first; CPU-only scalers don't move.
        assert!(ap.level(ScalerKind::ShadowDistance) > 0 || ap.level(ScalerKind::Lod) > 0);
        assert_eq!(ap.level(ScalerKind::Framerate), 0);
        let lowered: u32 = ScalerKind::ALL.iter().map(|&k| ap.level(k)).sum();
        assert!(lowered >= 3, "{lowered}");
        let o = ap.overrides(&s, 256);
        assert!(o.shadow_distance < 1.0 || o.lod_bias < 1.0);
        // Plenty of headroom: quality comes back, one step at a time.
        run(&mut ap, &s, &mut t, 120.0, 0.003, 0.004, cool());
        assert_eq!(
            ap.performance_bottleneck(),
            PerformanceBottleneck::TargetFrameRate
        );
        let left: u32 = ScalerKind::ALL.iter().map(|&k| ap.level(k)).sum();
        assert_eq!(left, 0);
        assert_eq!(ap.overrides(&s, 256), QualityOverrides::default());
    }

    #[test]
    fn cpu_bound_only_touches_cpu_scalers() {
        let s = AdaptivePerformanceSettings::default();
        let mut ap = AdaptivePerformance::default();
        let mut t = 0.0;
        run(&mut ap, &s, &mut t, 30.0, 0.030, 0.004, cool());
        assert_eq!(ap.performance_bottleneck(), PerformanceBottleneck::CPU);
        assert_eq!(ap.level(ScalerKind::Resolution), 0);
        assert_eq!(ap.level(ScalerKind::Msaa), 0);
        assert!(ap.level(ScalerKind::Lod) + ap.level(ScalerKind::Lights) > 0);
    }

    #[test]
    fn throttling_fast_decreases_and_overrides_stick() {
        let mut s = AdaptivePerformanceSettings::default();
        s.framerate.enabled = true;
        let mut ap = AdaptivePerformance::default();
        ap.override_level(ScalerKind::Resolution, Some(2));
        let hot = ThermalMetrics {
            warning_level: WarningLevel::Throttling,
            temperature_level: 1.0,
            temperature_trend: 0.0,
        };
        let mut t = 0.0;
        run(&mut ap, &s, &mut t, 6.0, 0.004, 0.004, hot);
        assert_eq!(ap.action(), StateAction::FastDecrease);
        // Two steps per change, every half thermal delay: 6 s → at least 2 changes.
        let lowered: u32 = ScalerKind::ALL
            .iter()
            .filter(|&&k| k != ScalerKind::Resolution)
            .map(|&k| ap.level(k))
            .sum();
        assert!(lowered >= 4, "{lowered}");
        assert_eq!(ap.level(ScalerKind::Resolution), 2);
        assert!((ap.overrides(&s, 256).render_scale - s.resolution.scale(2)).abs() < 1e-6);
    }

    #[test]
    fn simulation_and_events() {
        let s = AdaptivePerformanceSettings::default();
        let mut ap = AdaptivePerformance::default();
        ap.simulation.bottleneck = Some(PerformanceBottleneck::GPU);
        ap.step(&s, 0.0, 0.016, 0.001, Some(0.001), cool());
        assert_eq!(ap.performance_bottleneck(), PerformanceBottleneck::GPU);
        assert!(
            ap.events()
                .contains(&AdaptivePerformanceEvent::BottleneckChanged(
                    PerformanceBottleneck::GPU
                ))
        );
        assert_eq!(
            ScalerKind::from_name("resolution"),
            Some(ScalerKind::Resolution)
        );
        assert_eq!(
            ScalerKind::from_name("AdaptiveMSAA"),
            Some(ScalerKind::Msaa)
        );
        assert_eq!(
            ScalerKind::from_name("shadow_distance"),
            Some(ScalerKind::ShadowDistance)
        );
    }

    #[test]
    fn world_update_applies_and_clears_overrides() {
        let mut w = World::new();
        w.resources.insert(crate::AppControl::default());
        w.resources.insert(QualitySettings::default());
        w.resources.insert(AdaptivePerformanceSettings::default());
        update(&mut w, true, 0.0, 0.016, 0.001, Some(0.001), 60.0);
        w.resource_mut::<AdaptivePerformance>()
            .override_level(ScalerKind::Resolution, Some(4));
        update(&mut w, true, 0.02, 0.016, 0.001, Some(0.001), 60.0);
        assert_eq!(QualitySettings::of(&w).render_scale, 0.6);
        // Leaving Play mode puts quality back.
        update(&mut w, false, 0.04, 0.016, 0.001, Some(0.001), 60.0);
        assert!(w.try_resource::<QualityOverrides>().is_none());
        assert_eq!(QualitySettings::of(&w).render_scale, 1.0);
    }
}

#[cfg(test)]
mod light_limit_tests {
    use oxide_math::Vector3;
    use oxide_render::PunctualLight;

    #[test]
    fn keeps_the_most_important_lights() {
        let light = |x: f32, intensity: f32| PunctualLight {
            position: Vector3::new(x, 0.0, 0.0),
            range: 5.0,
            color: [intensity; 3],
            ..Default::default()
        };
        let mut lights = vec![
            light(50.0, 1.0),
            light(2.0, 1.0),
            light(30.0, 100.0),
            light(80.0, 1.0),
        ];
        crate::extract::limit_lights(&mut lights, Vector3::ZERO, 2);
        let xs: Vec<f32> = lights.iter().map(|l| l.position.x).collect();
        // The one around the camera, then the bright distant one.
        assert_eq!(xs, vec![2.0, 30.0]);
    }
}
