//! Unity's Audio Mixer: a `.mixer` asset of groups (Master → Music, SFX, ...) with
//! volume, pitch, mute/solo and effects (lowpass, highpass, echo, reverb, distortion,
//! compressor), sends to Receive groups (aux buses: one reverb shared by many groups),
//! sidechain ducking (Duck Volume: music dips while dialogue plays), snapshots that store alternative values (Paused, Underwater) to
//! transition between, and exposed parameters scripts set by name. Audio Sources route
//! into a group with `output: "Assets/Main.mixer#SFX"`.
//! Reference: <https://docs.unity3d.com/Manual/AudioMixer.html>

use kira::effect::compressor::{CompressorBuilder, CompressorHandle};
use kira::effect::delay::{DelayBuilder, DelayHandle};
use kira::effect::distortion::{DistortionBuilder, DistortionHandle};
use kira::effect::filter::{FilterBuilder, FilterHandle, FilterMode};
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::track::{SendTrackBuilder, SendTrackHandle, SendTrackId, TrackBuilder, TrackHandle};
use kira::{Frame, Mix, Tween};
use oxide_assets::Asset;
use oxide_core::Reflect;
use oxide_core::reflect::Reflect as _;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Unity's audio effect types (the ones kira can run).
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioEffectType {
    #[default]
    Lowpass,
    Highpass,
    Echo,
    Reverb,
    Distortion,
    Compressor,
    /// Sends a copy of the group's signal (at `send_level`) to `target`'s Receive, or
    /// sidechains `target`'s Duck Volume.
    Send,
    /// Mixes in what other groups send here; the effects after it process the sends.
    Receive,
    /// Lowers the group's volume while the groups sending to it are above `threshold`.
    DuckVolume,
}

/// One effect in a group's chain. Each type uses some of the fields (the Audio Mixer
/// window shows only those).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct AudioMixerEffect {
    pub kind: AudioEffectType,
    /// Lowpass/Highpass cutoff (Hz).
    pub cutoff: f32,
    /// Lowpass/Highpass resonance (0..1).
    pub resonance: f32,
    /// Echo delay (ms). Changing it rebuilds the group.
    pub delay: f32,
    /// Echo feedback (dB, below 0) / Reverb room size (0..1).
    pub feedback: f32,
    /// Reverb damping (0..1).
    pub damping: f32,
    /// Distortion drive (dB).
    pub drive: f32,
    /// Compressor threshold (dB), ratio, attack/release (ms), makeup gain (dB).
    pub threshold: f32,
    pub ratio: f32,
    pub attack: f32,
    pub release: f32,
    pub makeup_gain: f32,
    /// Send: the group (with a Receive or Duck Volume) it sends to.
    pub target: String,
    /// Send level (dB).
    pub send_level: f32,
    /// Wet/dry mix (0 = dry, 1 = fully processed).
    pub wet_mix: f32,
}

impl Default for AudioMixerEffect {
    fn default() -> Self {
        Self::new(AudioEffectType::Lowpass)
    }
}

impl AudioMixerEffect {
    /// Unity's defaults for each effect.
    pub fn new(kind: AudioEffectType) -> Self {
        let mut e = Self {
            kind,
            cutoff: 5000.0,
            resonance: 0.1,
            delay: 500.0,
            feedback: -6.0,
            damping: 0.5,
            drive: 12.0,
            threshold: -20.0,
            ratio: 4.0,
            attack: 10.0,
            release: 100.0,
            makeup_gain: 0.0,
            target: String::new(),
            send_level: 0.0,
            wet_mix: 1.0,
        };
        match kind {
            AudioEffectType::Highpass => e.cutoff = 500.0,
            AudioEffectType::Reverb => {
                e.feedback = 0.8;
                e.wet_mix = 0.35;
            }
            AudioEffectType::Echo => e.wet_mix = 0.5,
            // Unity's Duck Volume defaults.
            AudioEffectType::DuckVolume => {
                e.threshold = -25.0;
                e.ratio = 2.0;
                e.attack = 100.0;
                e.release = 300.0;
            }
            _ => {}
        }
        e
    }

    /// Names of the parameters this effect type uses.
    pub fn parameters(&self) -> &'static [&'static str] {
        match self.kind {
            AudioEffectType::Lowpass | AudioEffectType::Highpass => {
                &["cutoff", "resonance", "wet_mix"]
            }
            AudioEffectType::Echo => &["delay", "feedback", "wet_mix"],
            AudioEffectType::Reverb => &["feedback", "damping", "wet_mix"],
            AudioEffectType::Distortion => &["drive", "wet_mix"],
            AudioEffectType::Compressor => &[
                "threshold",
                "ratio",
                "attack",
                "release",
                "makeup_gain",
                "wet_mix",
            ],
            AudioEffectType::Send => &["send_level"],
            AudioEffectType::Receive => &[],
            AudioEffectType::DuckVolume => &[
                "threshold",
                "ratio",
                "attack",
                "release",
                "makeup_gain",
                "wet_mix",
            ],
        }
    }
}

/// A mixer group (Unity `AudioMixerGroup`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct AudioMixerGroup {
    pub name: String,
    /// Parent group's name; empty for Master.
    pub parent: String,
    /// Attenuation (dB, -80..20).
    pub volume: f32,
    /// Multiplies the pitch of everything routed here.
    pub pitch: f32,
    pub mute: bool,
    pub solo: bool,
    /// Skip this group's effects.
    pub bypass_effects: bool,
    pub effects: Vec<AudioMixerEffect>,
}

impl Default for AudioMixerGroup {
    fn default() -> Self {
        Self::new("Master", "")
    }
}

impl AudioMixerGroup {
    pub fn new(name: &str, parent: &str) -> Self {
        Self {
            name: name.into(),
            parent: parent.into(),
            volume: 0.0,
            pitch: 1.0,
            mute: false,
            solo: false,
            bypass_effects: false,
            effects: Vec::new(),
        }
    }
}

/// A stored parameter value (`Music/volume`, `SFX/0/cutoff`).
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct AudioMixerValue {
    pub parameter: String,
    pub value: f32,
}

/// Unity `AudioMixerSnapshot`: values that differ from the groups' own.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct AudioMixerSnapshot {
    pub name: String,
    pub values: Vec<AudioMixerValue>,
}

/// A parameter scripts can set by name (Unity "Expose to script").
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct ExposedParameter {
    pub name: String,
    pub parameter: String,
}

/// Unity's Audio Mixer asset (`.mixer`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct AudioMixer {
    /// The first group is Master.
    pub groups: Vec<AudioMixerGroup>,
    /// The first snapshot is the start snapshot (the groups' own values).
    pub snapshots: Vec<AudioMixerSnapshot>,
    pub exposed_parameters: Vec<ExposedParameter>,
}

impl Default for AudioMixer {
    /// A Master group and the default "Snapshot", like a new Unity mixer.
    fn default() -> Self {
        Self {
            groups: vec![AudioMixerGroup::new("Master", "")],
            snapshots: vec![AudioMixerSnapshot {
                name: "Snapshot".into(),
                values: vec![],
            }],
            exposed_parameters: vec![],
        }
    }
}

impl Asset for AudioMixer {
    const TYPE_NAME: &'static str = "AudioMixer";
}

/// An Audio Source's output group, by path: `Assets/Main.mixer#SFX`.
#[derive(Clone, Debug)]
pub struct AudioMixerGroupRef;

impl Asset for AudioMixerGroupRef {
    const TYPE_NAME: &'static str = "AudioMixerGroup";
}

/// Splits `Assets/Main.mixer#SFX` into the mixer path and the group name (Master when
/// there is no `#`).
pub fn split_group_path(path: &str) -> (&str, &str) {
    path.split_once('#').unwrap_or((path, "Master"))
}

impl AudioMixer {
    pub fn group(&self, name: &str) -> Option<&AudioMixerGroup> {
        self.groups.iter().find(|g| g.name == name)
    }

    /// Group names from Master down to `name`.
    pub fn chain(&self, name: &str) -> Vec<&AudioMixerGroup> {
        let mut out = Vec::new();
        let mut cur = self.group(name);
        while let Some(g) = cur {
            if out.iter().any(|o: &&AudioMixerGroup| o.name == g.name) {
                break;
            }
            out.push(g);
            cur = if g.parent.is_empty() {
                None
            } else {
                self.group(&g.parent)
            };
        }
        out.reverse();
        out
    }

    /// Every parameter's base value (the groups' own values).
    pub fn base_values(&self) -> HashMap<String, f32> {
        let mut v = HashMap::new();
        for g in &self.groups {
            v.insert(format!("{}/volume", g.name), g.volume);
            v.insert(format!("{}/pitch", g.name), g.pitch);
            for (i, e) in g.effects.iter().enumerate() {
                for p in e.parameters() {
                    if let Some(oxide_core::reflect::Value::Float(x)) = e.get_field(p) {
                        v.insert(format!("{}/{i}/{p}", g.name), x as f32);
                    }
                }
            }
        }
        v
    }

    /// Values with `snapshot`'s overrides applied.
    pub fn snapshot_values(&self, snapshot: &str) -> HashMap<String, f32> {
        let mut v = self.base_values();
        if let Some(s) = self.snapshots.iter().find(|s| s.name == snapshot) {
            for x in &s.values {
                if v.contains_key(&x.parameter) {
                    v.insert(x.parameter.clone(), x.value);
                }
            }
        }
        v
    }

    /// Volume (dB) a group actually plays at, with mute and solo: soloing any group
    /// silences groups that are neither soloed, nor above or below a soloed group.
    pub fn effective_volume(&self, group: &str, values: &HashMap<String, f32>) -> f32 {
        let Some(g) = self.group(group) else {
            return -80.0;
        };
        if g.mute {
            return -80.0;
        }
        let soloed: Vec<&str> = self
            .groups
            .iter()
            .filter(|g| g.solo)
            .map(|g| g.name.as_str())
            .collect();
        if !soloed.is_empty() {
            let related = soloed.iter().any(|s| {
                *s == group
                    || self.chain(s).iter().any(|a| a.name == group)
                    || self.chain(group).iter().any(|a| a.name == *s)
            });
            if !related {
                return -80.0;
            }
        }
        values
            .get(&format!("{group}/volume"))
            .copied()
            .unwrap_or(g.volume)
    }

    /// The first effect of type `kind` in `group`.
    pub fn effect_index(&self, group: &str, kind: AudioEffectType) -> Option<usize> {
        self.group(group)?
            .effects
            .iter()
            .position(|e| e.kind == kind)
    }

    /// Sum of the faders (dB) from Master down to `group` (what a Receive's signal plays at).
    pub fn chain_volume(&self, group: &str, values: &HashMap<String, f32>) -> f32 {
        let mut sum = 0.0;
        for g in self.chain(group) {
            let v = self.effective_volume(&g.name, values);
            if v <= -79.9 {
                return -80.0;
            }
            sum += v;
        }
        sum.max(-80.0)
    }

    /// Product of group pitches from Master down to `group`.
    pub fn pitch(&self, group: &str, values: &HashMap<String, f32>) -> f32 {
        self.chain(group)
            .iter()
            .map(|g| {
                values
                    .get(&format!("{}/pitch", g.name))
                    .copied()
                    .unwrap_or(g.pitch)
            })
            .product()
    }

    /// What forces a rebuild of the kira tracks: groups, parents, effect types, delays.
    pub fn structure(&self) -> String {
        self.groups
            .iter()
            .map(|g| {
                let fx: Vec<String> = g
                    .effects
                    .iter()
                    .map(|e| format!("{:?}{}>{}", e.kind, e.delay.round(), e.target))
                    .collect();
                format!("{}<{}[{}]", g.name, g.parent, fx.join(","))
            })
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// Per-mixer playback state: active snapshot, transition and script-set exposed values.
#[derive(Clone, Debug, Default)]
pub struct MixerState {
    pub snapshot: String,
    /// Values when the transition started, and (elapsed, duration).
    pub from: HashMap<String, f32>,
    pub transition: Option<(f32, f32)>,
    /// Exposed parameter values set by scripts (Unity `SetFloat`), by exposed name.
    pub exposed: HashMap<String, f32>,
}

impl MixerState {
    /// The values the mixer plays at now: the snapshot (blended from the previous one while
    /// transitioning), then exposed overrides.
    pub fn values(&self, mixer: &AudioMixer) -> HashMap<String, f32> {
        let snap = if self.snapshot.is_empty() {
            mixer
                .snapshots
                .first()
                .map(|s| s.name.clone())
                .unwrap_or_default()
        } else {
            self.snapshot.clone()
        };
        let mut v = mixer.snapshot_values(&snap);
        if let Some((t, d)) = self.transition {
            let k = if d > 0.0 {
                (t / d).clamp(0.0, 1.0)
            } else {
                1.0
            };
            for (p, x) in v.iter_mut() {
                if let Some(a) = self.from.get(p) {
                    *x = a + (*x - a) * k;
                }
            }
        }
        for e in &mixer.exposed_parameters {
            if let Some(x) = self.exposed.get(&e.name)
                && v.contains_key(&e.parameter)
            {
                v.insert(e.parameter.clone(), *x);
            }
        }
        v
    }
}

/// An `f32` shared with the audio thread.
#[derive(Debug, Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// What a Duck Volume shares with the audio thread: the loudest sidechain peak since it last
/// looked, and its settings.
#[derive(Debug, Default)]
pub struct DuckShared {
    /// Peak amplitude as `f32` bits (non-negative floats order like their bits, so senders
    /// combine with `fetch_max`).
    sidechain: AtomicU32,
    pub threshold: AtomicF32,
    pub ratio: AtomicF32,
    /// Attack / release (ms).
    pub attack: AtomicF32,
    pub release: AtomicF32,
    pub makeup_gain: AtomicF32,
    pub mix: AtomicF32,
}

impl DuckShared {
    fn set(&self, e: &AudioMixerEffect, get: impl Fn(&str, f32) -> f32, mix: f32) {
        self.threshold.set(get("threshold", e.threshold));
        self.ratio.set(get("ratio", e.ratio).max(1.0));
        self.attack.set(get("attack", e.attack).max(0.1));
        self.release.set(get("release", e.release).max(1.0));
        self.makeup_gain.set(get("makeup_gain", e.makeup_gain));
        self.mix.set(mix);
    }
}

/// Gain reduction (dB) a Duck Volume applies for a sidechain at `level` dB.
pub fn duck_reduction(level: f32, threshold: f32, ratio: f32) -> f32 {
    if level > threshold {
        (level - threshold) * (1.0 - 1.0 / ratio.max(1.0))
    } else {
        0.0
    }
}

fn amplitude(db: f32) -> f32 {
    if db <= -79.9 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// A Send to a Duck Volume: measures the signal's peak (times the send level) and passes the
/// audio through untouched.
pub struct SidechainMeter {
    duck: Arc<DuckShared>,
    /// Send level (amplitude).
    level: Arc<AtomicF32>,
}

impl SidechainMeter {
    pub fn run(&mut self, input: &[Frame]) {
        let peak = input
            .iter()
            .map(|f| f.left.abs().max(f.right.abs()))
            .fold(0.0f32, f32::max)
            * self.level.get();
        self.duck
            .sidechain
            .fetch_max(peak.max(0.0).to_bits(), Ordering::Relaxed);
    }
}

impl kira::effect::Effect for SidechainMeter {
    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &kira::info::Info) {
        self.run(input);
    }
}

/// Unity's Duck Volume: a compressor keyed by what other groups send to it.
pub struct Ducker {
    duck: Arc<DuckShared>,
    /// Current gain reduction (dB), smoothed by attack/release.
    reduction: f32,
}

impl Ducker {
    pub fn run(&mut self, input: &mut [Frame], dt: f64) {
        let d = &self.duck;
        let peak = f32::from_bits(d.sidechain.swap(0, Ordering::Relaxed));
        let level = if peak > 1e-6 {
            20.0 * peak.log10()
        } else {
            -120.0
        };
        let target = duck_reduction(level, d.threshold.get(), d.ratio.get());
        let (attack, release) = (d.attack.get() / 1000.0, d.release.get() / 1000.0);
        let (makeup, mix) = (d.makeup_gain.get(), d.mix.get());
        for f in input {
            let tau = if target > self.reduction {
                attack
            } else {
                release
            };
            self.reduction += (target - self.reduction) * (1.0 - (-(dt as f32) / tau).exp());
            let gain = 1.0 - mix + mix * amplitude(makeup - self.reduction);
            *f *= gain;
        }
    }
}

impl kira::effect::Effect for Ducker {
    fn process(&mut self, input: &mut [Frame], dt: f64, _info: &kira::info::Info) {
        self.run(input, dt);
    }
}

enum EffectHandle {
    Filter(FilterHandle),
    Echo(DelayHandle),
    Reverb(ReverbHandle),
    Distortion(DistortionHandle),
    Compressor(CompressorHandle),
    /// A Send: to a Receive's send track, or a Duck Volume's sidechain meter.
    Send {
        track: Option<SendTrackId>,
        meter: Option<Arc<AtomicF32>>,
    },
    Duck(Arc<DuckShared>),
    /// Receive (the routing is the group's send track), or an effect that can't run here.
    None,
}

/// Kira's sub- and send-track builders.
trait FxBuilder {
    fn fx<B: kira::effect::EffectBuilder>(&mut self, b: B) -> B::Handle;
    fn built(&mut self, e: Box<dyn kira::effect::Effect>);
}

impl FxBuilder for TrackBuilder {
    fn fx<B: kira::effect::EffectBuilder>(&mut self, b: B) -> B::Handle {
        self.add_effect(b)
    }
    fn built(&mut self, e: Box<dyn kira::effect::Effect>) {
        self.add_built_effect(e);
    }
}

impl FxBuilder for SendTrackBuilder {
    fn fx<B: kira::effect::EffectBuilder>(&mut self, b: B) -> B::Handle {
        self.add_effect(b)
    }
    fn built(&mut self, e: Box<dyn kira::effect::Effect>) {
        self.add_built_effect(e);
    }
}

/// Where Sends go: Receives' send tracks and Duck Volumes' shared state, by group.
#[derive(Default)]
struct Links {
    receives: HashMap<String, SendTrackId>,
    ducks: HashMap<String, Arc<DuckShared>>,
}

/// The Receive's send track and the handles of the effects after it.
struct ReceiveRuntime {
    track: SendTrackHandle,
    start: usize,
    effects: Vec<EffectHandle>,
}

pub(crate) struct GroupRuntime {
    pub track: TrackHandle,
    effects: Vec<EffectHandle>,
    receive: Option<ReceiveRuntime>,
}

/// A mixer's kira tracks.
pub(crate) struct MixerRuntime {
    pub structure: String,
    pub groups: HashMap<String, GroupRuntime>,
    applied: HashMap<String, f32>,
    pub generation: u64,
}

/// Adds `effects` to a track; returns a handle per effect and the send tracks to route to
/// (only sub-tracks can send; `on_send_track` adds meters and skips the rest). `duck` is the
/// group's Duck Volume state its senders meter into.
fn add_effects(
    b: &mut impl FxBuilder,
    effects: &[AudioMixerEffect],
    links: &Links,
    on_send_track: bool,
    duck: Option<&Arc<DuckShared>>,
) -> (Vec<EffectHandle>, Vec<(SendTrackId, f32)>) {
    let mut sends = Vec::new();
    let handles = effects
        .iter()
        .map(|e| match e.kind {
            AudioEffectType::Lowpass | AudioEffectType::Highpass => EffectHandle::Filter(
                b.fx(FilterBuilder::new()
                    .mode(if e.kind == AudioEffectType::Lowpass {
                        FilterMode::LowPass
                    } else {
                        FilterMode::HighPass
                    })
                    .cutoff(e.cutoff as f64)
                    .resonance(e.resonance as f64)),
            ),
            AudioEffectType::Echo => EffectHandle::Echo(
                b.fx(DelayBuilder::new()
                    .delay_time(Duration::from_secs_f32(
                        (e.delay / 1000.0).clamp(0.001, 5.0),
                    ))
                    .feedback(e.feedback.min(-0.1))),
            ),
            AudioEffectType::Reverb => EffectHandle::Reverb(
                b.fx(ReverbBuilder::new()
                    .feedback(e.feedback as f64)
                    .damping(e.damping as f64)),
            ),
            AudioEffectType::Distortion => {
                EffectHandle::Distortion(b.fx(DistortionBuilder::new().drive(e.drive)))
            }
            AudioEffectType::Compressor => EffectHandle::Compressor(
                b.fx(CompressorBuilder::new()
                    .threshold(e.threshold as f64)
                    .ratio(e.ratio as f64)),
            ),
            AudioEffectType::Send => {
                if let Some(id) = links.receives.get(&e.target).filter(|_| !on_send_track) {
                    sends.push((*id, e.send_level));
                    EffectHandle::Send {
                        track: Some(*id),
                        meter: None,
                    }
                } else if let Some(duck) = links.ducks.get(&e.target) {
                    let level = Arc::new(AtomicF32::new(amplitude(e.send_level)));
                    b.built(Box::new(SidechainMeter {
                        duck: duck.clone(),
                        level: level.clone(),
                    }));
                    EffectHandle::Send {
                        track: None,
                        meter: Some(level),
                    }
                } else {
                    EffectHandle::None
                }
            }
            AudioEffectType::Receive => EffectHandle::None,
            // A Duck Volume runs on the group's own track only (one reader per sidechain).
            AudioEffectType::DuckVolume => match duck.filter(|_| !on_send_track) {
                Some(duck) => {
                    b.built(Box::new(Ducker {
                        duck: duck.clone(),
                        reduction: 0.0,
                    }));
                    EffectHandle::Duck(duck.clone())
                }
                None => EffectHandle::None,
            },
        })
        .collect();
    (handles, sends)
}

/// Pushes one effect's current values to its handle.
fn apply_effect(
    h: &mut EffectHandle,
    e: &AudioMixerEffect,
    get: &dyn Fn(&str, f32) -> f32,
    mix: f32,
    track: Option<&mut TrackHandle>,
) {
    let t = Tween::default();
    match h {
        EffectHandle::Filter(f) => {
            f.set_cutoff(get("cutoff", e.cutoff).clamp(10.0, 22_000.0) as f64, t);
            f.set_resonance(get("resonance", e.resonance).clamp(0.0, 1.0) as f64, t);
            f.set_mix(Mix(mix), t);
        }
        EffectHandle::Echo(d) => {
            d.set_feedback(get("feedback", e.feedback).min(-0.1), t);
            d.set_mix(Mix(mix), t);
        }
        EffectHandle::Reverb(r) => {
            r.set_feedback(get("feedback", e.feedback).clamp(0.0, 1.0) as f64, t);
            r.set_damping(get("damping", e.damping).clamp(0.0, 1.0) as f64, t);
            r.set_mix(Mix(mix), t);
        }
        EffectHandle::Distortion(d) => {
            d.set_drive(get("drive", e.drive), t);
            d.set_mix(Mix(mix), t);
        }
        EffectHandle::Compressor(c) => {
            c.set_threshold(get("threshold", e.threshold) as f64, t);
            c.set_ratio(get("ratio", e.ratio).max(1.0) as f64, t);
            c.set_attack_duration(
                Duration::from_secs_f32(get("attack", e.attack).max(0.1) / 1000.0),
                t,
            );
            c.set_release_duration(
                Duration::from_secs_f32(get("release", e.release).max(1.0) / 1000.0),
                t,
            );
            c.set_makeup_gain(get("makeup_gain", e.makeup_gain), t);
            c.set_mix(Mix(mix), t);
        }
        EffectHandle::Send { track: id, meter } => {
            // Bypassed effects mute the send.
            let level = if mix > 0.0 {
                get("send_level", e.send_level)
            } else {
                -80.0
            };
            if let (Some(id), Some(track)) = (id, track) {
                // Fails only when the send track is gone (the mixer is being rebuilt).
                let _ = track.set_send(
                    *id,
                    if level <= -79.9 {
                        f32::NEG_INFINITY
                    } else {
                        level
                    },
                    t,
                );
            }
            if let Some(m) = meter {
                m.set(amplitude(level));
            }
        }
        EffectHandle::Duck(d) => d.set(e, get, mix),
        EffectHandle::None => {}
    }
}

impl MixerRuntime {
    /// Builds a track per group under `master` (the kira main track's sub-tracks), parents
    /// first, after the Receives' send tracks and Duck Volumes' sidechains they route to.
    pub fn build<B: kira::backend::Backend>(
        manager: &mut kira::AudioManager<B>,
        mixer: &AudioMixer,
        generation: u64,
    ) -> Option<Self> {
        let mut links = Links::default();
        for g in &mixer.groups {
            if let Some(i) = mixer.effect_index(&g.name, AudioEffectType::DuckVolume) {
                let duck = Arc::new(DuckShared::default());
                duck.set(&g.effects[i], |_, d| d, g.effects[i].wet_mix);
                links.ducks.insert(g.name.clone(), duck);
            }
        }
        let mut receives: HashMap<String, ReceiveRuntime> = HashMap::new();
        for g in &mixer.groups {
            let Some(i) = mixer.effect_index(&g.name, AudioEffectType::Receive) else {
                continue;
            };
            let mut b = SendTrackBuilder::new();
            let (effects, _) = add_effects(&mut b, &g.effects[i + 1..], &links, true, None);
            if let Ok(track) = manager.add_send_track(b) {
                links.receives.insert(g.name.clone(), track.id());
                receives.insert(
                    g.name.clone(),
                    ReceiveRuntime {
                        track,
                        start: i + 1,
                        effects,
                    },
                );
            }
        }
        let mut groups: HashMap<String, GroupRuntime> = HashMap::new();
        let mut pending: Vec<&AudioMixerGroup> = mixer.groups.iter().collect();
        while !pending.is_empty() {
            let before = pending.len();
            let mut rest = Vec::new();
            for g in pending {
                let mut b = TrackBuilder::new();
                let duck = links.ducks.get(&g.name);
                let (effects, sends) = add_effects(&mut b, &g.effects, &links, false, duck);
                for (id, level) in sends {
                    b = b.with_send(
                        id,
                        if level <= -79.9 {
                            f32::NEG_INFINITY
                        } else {
                            level
                        },
                    );
                }
                let track = if g.parent.is_empty() || mixer.group(&g.parent).is_none() {
                    manager.add_sub_track(b).ok()
                } else if let Some(p) = groups.get_mut(&g.parent) {
                    p.track.add_sub_track(b).ok()
                } else {
                    rest.push(g);
                    continue;
                };
                if let Some(track) = track {
                    groups.insert(
                        g.name.clone(),
                        GroupRuntime {
                            track,
                            effects,
                            receive: receives.remove(&g.name),
                        },
                    );
                }
            }
            if rest.len() == before {
                break;
            }
            pending = rest;
        }
        Some(Self {
            structure: mixer.structure(),
            groups,
            applied: HashMap::new(),
            generation,
        })
    }

    /// Pushes changed values to the tracks and effects.
    pub fn apply(&mut self, mixer: &AudioMixer, values: &HashMap<String, f32>) {
        let t = Tween::default();
        for g in &mixer.groups {
            let Some(rt) = self.groups.get_mut(&g.name) else {
                continue;
            };
            let vol = mixer.effective_volume(&g.name, values);
            let key = format!("{}/volume!", g.name);
            if self.applied.get(&key) != Some(&vol) {
                rt.track
                    .set_volume(if vol <= -79.9 { f32::NEG_INFINITY } else { vol }, t);
                self.applied.insert(key, vol);
            }
            let GroupRuntime {
                track,
                effects,
                receive,
            } = rt;
            if let Some(r) = receive.as_mut() {
                // The Receive plays at the group's and its parents' faders.
                let vol = mixer.chain_volume(&g.name, values);
                let key = format!("{}/receive!", g.name);
                if self.applied.get(&key) != Some(&vol) {
                    r.track
                        .set_volume(if vol <= -79.9 { f32::NEG_INFINITY } else { vol }, t);
                    self.applied.insert(key, vol);
                }
            }
            let mut chains: Vec<(
                usize,
                &mut Vec<EffectHandle>,
                Option<&mut TrackHandle>,
                &str,
            )> = vec![(0, effects, Some(track), "")];
            if let Some(r) = receive.as_mut() {
                chains.push((r.start, &mut r.effects, None, "r"));
            }
            for (start, handles, mut track, suffix) in chains {
                for (k, h) in handles.iter_mut().enumerate() {
                    let i = start + k;
                    let Some(e) = g.effects.get(i) else { break };
                    let get = |p: &str, d: f32| {
                        values
                            .get(&format!("{}/{i}/{p}", g.name))
                            .copied()
                            .unwrap_or(d)
                    };
                    let mix = if g.bypass_effects {
                        0.0
                    } else {
                        get("wet_mix", e.wet_mix).clamp(0.0, 1.0)
                    };
                    let key = format!("{}/{i}{suffix}", g.name);
                    let sig: f32 =
                        e.parameters().iter().map(|p| get(p, 0.0)).sum::<f32>() + mix * 1e3;
                    if self.applied.get(&key) == Some(&sig) {
                        continue;
                    }
                    self.applied.insert(key, sig);
                    apply_effect(h, e, &get, mix, track.as_deref_mut());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixer() -> AudioMixer {
        let mut m = AudioMixer::default();
        m.groups.push(AudioMixerGroup::new("Music", "Master"));
        m.groups.push(AudioMixerGroup::new("SFX", "Master"));
        m.groups.push(AudioMixerGroup::new("Guns", "SFX"));
        m.groups[3].pitch = 1.5;
        m.groups[2].pitch = 2.0;
        m.groups[1]
            .effects
            .push(AudioMixerEffect::new(AudioEffectType::Lowpass));
        m.snapshots.push(AudioMixerSnapshot {
            name: "Paused".into(),
            values: vec![
                AudioMixerValue {
                    parameter: "Music/volume".into(),
                    value: -20.0,
                },
                AudioMixerValue {
                    parameter: "Music/0/cutoff".into(),
                    value: 800.0,
                },
            ],
        });
        m.exposed_parameters.push(ExposedParameter {
            name: "MusicVol".into(),
            parameter: "Music/volume".into(),
        });
        m
    }

    #[test]
    fn chains_pitch_and_paths() {
        let m = mixer();
        let names: Vec<&str> = m.chain("Guns").iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["Master", "SFX", "Guns"]);
        assert_eq!(m.pitch("Guns", &m.base_values()), 3.0);
        assert_eq!(
            split_group_path("Assets/Main.mixer#SFX"),
            ("Assets/Main.mixer", "SFX")
        );
        assert_eq!(
            split_group_path("Assets/Main.mixer"),
            ("Assets/Main.mixer", "Master")
        );
        assert_eq!(m.base_values()["Music/0/cutoff"], 5000.0);
    }

    #[test]
    fn mute_and_solo() {
        let mut m = mixer();
        let v = m.base_values();
        assert_eq!(m.effective_volume("Music", &v), 0.0);
        m.groups[2].solo = true; // SFX
        assert_eq!(
            m.effective_volume("Music", &v),
            -80.0,
            "not related to the soloed group"
        );
        assert_eq!(
            m.effective_volume("Guns", &v),
            0.0,
            "below the soloed group"
        );
        assert_eq!(
            m.effective_volume("Master", &v),
            0.0,
            "above the soloed group"
        );
        m.groups[3].mute = true;
        assert_eq!(m.effective_volume("Guns", &v), -80.0);
    }

    #[test]
    fn snapshot_transitions_and_exposed_parameters() {
        let m = mixer();
        let mut s = MixerState::default();
        assert_eq!(s.values(&m)["Music/volume"], 0.0, "start snapshot");
        s.from = s.values(&m);
        s.snapshot = "Paused".into();
        s.transition = Some((0.5, 1.0));
        let v = s.values(&m);
        assert_eq!(v["Music/volume"], -10.0, "halfway to Paused");
        assert_eq!(v["Music/0/cutoff"], 2900.0);
        s.transition = None;
        assert_eq!(s.values(&m)["Music/volume"], -20.0);
        s.exposed.insert("MusicVol".into(), -3.0);
        assert_eq!(
            s.values(&m)["Music/volume"],
            -3.0,
            "scripts override the snapshot"
        );
    }

    #[test]
    fn sends_receives_and_duck_parameters() {
        let mut m = mixer();
        let mut fx = AudioMixer::default().groups.remove(0);
        fx.name = "Reverb Bus".into();
        fx.parent = "Master".into();
        fx.effects
            .push(AudioMixerEffect::new(AudioEffectType::Receive));
        fx.effects
            .push(AudioMixerEffect::new(AudioEffectType::Reverb));
        m.groups.push(fx);
        let mut send = AudioMixerEffect::new(AudioEffectType::Send);
        send.target = "Reverb Bus".into();
        send.send_level = -6.0;
        m.groups[3].effects.push(send.clone()); // Guns -> Reverb Bus
        send.target = "Music".into();
        m.groups[2].effects.push(send); // SFX ducks Music
        m.groups[1]
            .effects
            .push(AudioMixerEffect::new(AudioEffectType::DuckVolume));
        let v = m.base_values();
        assert_eq!(v["Guns/0/send_level"], -6.0);
        assert_eq!(v["Music/1/threshold"], -25.0, "Unity's Duck Volume default");
        assert!(m.structure().contains("Send500>Reverb Bus"));
        assert_eq!(
            m.effect_index("Reverb Bus", AudioEffectType::Receive),
            Some(0)
        );
        let mut v = v;
        v.insert("Master/volume".into(), -3.0);
        v.insert("Reverb Bus/volume".into(), -2.0);
        assert_eq!(m.chain_volume("Reverb Bus", &v), -5.0);
        m.groups[0].mute = true;
        assert_eq!(m.chain_volume("Reverb Bus", &v), -80.0, "muted parent");
    }

    #[test]
    fn ducking_follows_the_sidechain() {
        assert_eq!(duck_reduction(-30.0, -25.0, 2.0), 0.0, "below threshold");
        assert_eq!(duck_reduction(-5.0, -25.0, 2.0), 10.0);
        assert_eq!(duck_reduction(-5.0, -25.0, 1.0), 0.0, "ratio 1 never ducks");
        let duck = Arc::new(DuckShared::default());
        let mut e = AudioMixerEffect::new(AudioEffectType::DuckVolume);
        e.attack = 1.0;
        e.release = 50.0;
        duck.set(&e, |_, d| d, 1.0);
        let mut meter = SidechainMeter {
            duck: duck.clone(),
            level: Arc::new(AtomicF32::new(1.0)),
        };
        let mut ducker = Ducker {
            duck: duck.clone(),
            reduction: 0.0,
        };
        let dt = 1.0 / 48_000.0;
        let loud = vec![Frame::new(0.5, -0.5); 480]; // -6 dB peak
        let mut music = vec![Frame::new(1.0, 1.0); 480];
        meter.run(&loud);
        ducker.run(&mut music, dt);
        // -6 dB at a -25 dB threshold, 2:1 -> ~9.5 dB of reduction once attacked.
        let g = music[479].left;
        assert!((g - amplitude(-9.53)).abs() < 0.02, "{g}");
        // Silence: releases back towards unity gain over the release time.
        for _ in 0..20 {
            meter.run(&[Frame::ZERO; 480]);
            music = vec![Frame::new(1.0, 1.0); 480];
            ducker.run(&mut music, dt);
        }
        assert!(music[479].left > 0.97, "{}", music[479].left);
        // A bypassed duck (mix 0) leaves the signal alone.
        duck.mix.set(0.0);
        meter.run(&loud);
        let mut music = vec![Frame::new(1.0, 1.0); 480];
        ducker.run(&mut music, dt);
        assert_eq!(music[479].left, 1.0);
    }

    #[test]
    fn builds_sends_receives_and_ducks_on_kira() {
        use kira::backend::mock::{MockBackend, MockBackendSettings};
        let mut m = mixer();
        m.groups.push(AudioMixerGroup::new("Bus", "Master"));
        m.groups[4]
            .effects
            .push(AudioMixerEffect::new(AudioEffectType::Receive));
        m.groups[4]
            .effects
            .push(AudioMixerEffect::new(AudioEffectType::Reverb));
        m.groups[1]
            .effects
            .push(AudioMixerEffect::new(AudioEffectType::DuckVolume));
        for (target, from) in [("Bus", 3), ("Music", 2)] {
            let mut send = AudioMixerEffect::new(AudioEffectType::Send);
            send.target = target.into();
            m.groups[from].effects.push(send);
        }
        let mut manager = kira::AudioManager::<MockBackend>::new(kira::AudioManagerSettings {
            backend_settings: MockBackendSettings::default(),
            ..Default::default()
        })
        .unwrap();
        let mut rt = MixerRuntime::build(&mut manager, &m, 1).unwrap();
        assert_eq!(rt.groups.len(), 5);
        let bus = &rt.groups["Bus"];
        let r = bus.receive.as_ref().expect("Bus has a send track");
        assert_eq!((r.start, r.effects.len()), (1, 1));
        assert!(matches!(
            rt.groups["Guns"].effects[0],
            EffectHandle::Send {
                track: Some(_),
                meter: None
            }
        ));
        assert!(matches!(
            rt.groups["SFX"].effects[0],
            EffectHandle::Send {
                track: None,
                meter: Some(_)
            }
        ));
        let EffectHandle::Duck(d) = &rt.groups["Music"].effects[1] else {
            panic!("Music has a Duck Volume");
        };
        let d = d.clone();
        let mut values = m.base_values();
        values.insert("Music/1/threshold".into(), -40.0);
        values.insert("SFX/0/send_level".into(), -12.0);
        rt.apply(&m, &values);
        assert_eq!(
            d.threshold.get(),
            -40.0,
            "snapshot values reach the audio thread"
        );
        let EffectHandle::Send {
            meter: Some(level), ..
        } = &rt.groups["SFX"].effects[0]
        else {
            unreachable!()
        };
        assert!((level.get() - amplitude(-12.0)).abs() < 1e-4);
    }

    #[test]
    fn mixer_round_trips_through_reflection() {
        use oxide_core::reflect::FieldValue;
        let m = mixer();
        let back = Vec::<AudioMixerGroup>::from_value(m.groups.to_value()).unwrap();
        assert_eq!(back, m.groups);
    }
}
