//! Unity's Audio Mixer: a `.mixer` asset of groups (Master → Music, SFX, ...) with
//! volume, pitch, mute/solo and effects (lowpass, highpass, echo, reverb, distortion,
//! compressor), snapshots that store alternative values (Paused, Underwater) to
//! transition between, and exposed parameters scripts set by name. Audio Sources route
//! into a group with `output: "Assets/Main.mixer#SFX"`.
//! Reference: <https://docs.unity3d.com/Manual/AudioMixer.html>

use kira::effect::compressor::{CompressorBuilder, CompressorHandle};
use kira::effect::delay::{DelayBuilder, DelayHandle};
use kira::effect::distortion::{DistortionBuilder, DistortionHandle};
use kira::effect::filter::{FilterBuilder, FilterHandle, FilterMode};
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{Mix, Tween};
use oxide_assets::Asset;
use oxide_core::Reflect;
use oxide_core::reflect::Reflect as _;
use std::collections::HashMap;
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
            wet_mix: 1.0,
        };
        match kind {
            AudioEffectType::Highpass => e.cutoff = 500.0,
            AudioEffectType::Reverb => {
                e.feedback = 0.8;
                e.wet_mix = 0.35;
            }
            AudioEffectType::Echo => e.wet_mix = 0.5,
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
                    .map(|e| format!("{:?}{}", e.kind, e.delay.round()))
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

enum EffectHandle {
    Filter(FilterHandle),
    Echo(DelayHandle),
    Reverb(ReverbHandle),
    Distortion(DistortionHandle),
    Compressor(CompressorHandle),
}

pub(crate) struct GroupRuntime {
    pub track: TrackHandle,
    effects: Vec<EffectHandle>,
}

/// A mixer's kira tracks.
pub(crate) struct MixerRuntime {
    pub structure: String,
    pub groups: HashMap<String, GroupRuntime>,
    applied: HashMap<String, f32>,
    pub generation: u64,
}

fn add_effects(b: &mut TrackBuilder, g: &AudioMixerGroup) -> Vec<EffectHandle> {
    g.effects
        .iter()
        .map(|e| match e.kind {
            AudioEffectType::Lowpass | AudioEffectType::Highpass => EffectHandle::Filter(
                b.add_effect(
                    FilterBuilder::new()
                        .mode(if e.kind == AudioEffectType::Lowpass {
                            FilterMode::LowPass
                        } else {
                            FilterMode::HighPass
                        })
                        .cutoff(e.cutoff as f64)
                        .resonance(e.resonance as f64),
                ),
            ),
            AudioEffectType::Echo => EffectHandle::Echo(
                b.add_effect(
                    DelayBuilder::new()
                        .delay_time(Duration::from_secs_f32(
                            (e.delay / 1000.0).clamp(0.001, 5.0),
                        ))
                        .feedback(e.feedback.min(-0.1)),
                ),
            ),
            AudioEffectType::Reverb => EffectHandle::Reverb(
                b.add_effect(
                    ReverbBuilder::new()
                        .feedback(e.feedback as f64)
                        .damping(e.damping as f64),
                ),
            ),
            AudioEffectType::Distortion => {
                EffectHandle::Distortion(b.add_effect(DistortionBuilder::new().drive(e.drive)))
            }
            AudioEffectType::Compressor => EffectHandle::Compressor(
                b.add_effect(
                    CompressorBuilder::new()
                        .threshold(e.threshold as f64)
                        .ratio(e.ratio as f64),
                ),
            ),
        })
        .collect()
}

impl MixerRuntime {
    /// Builds a track per group under `master` (the kira main track's sub-tracks), parents
    /// first.
    pub fn build(
        manager: &mut kira::AudioManager<kira::DefaultBackend>,
        mixer: &AudioMixer,
        generation: u64,
    ) -> Option<Self> {
        let mut groups: HashMap<String, GroupRuntime> = HashMap::new();
        let mut pending: Vec<&AudioMixerGroup> = mixer.groups.iter().collect();
        while !pending.is_empty() {
            let before = pending.len();
            let mut rest = Vec::new();
            for g in pending {
                let mut b = TrackBuilder::new();
                let effects = add_effects(&mut b, g);
                let track = if g.parent.is_empty() || mixer.group(&g.parent).is_none() {
                    manager.add_sub_track(b).ok()
                } else if let Some(p) = groups.get_mut(&g.parent) {
                    p.track.add_sub_track(b).ok()
                } else {
                    rest.push(g);
                    continue;
                };
                if let Some(track) = track {
                    groups.insert(g.name.clone(), GroupRuntime { track, effects });
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
            for (i, (e, h)) in g.effects.iter().zip(rt.effects.iter_mut()).enumerate() {
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
                let key = format!("{}/{i}", g.name);
                let sig: f32 = e.parameters().iter().map(|p| get(p, 0.0)).sum::<f32>() + mix * 1e3;
                if self.applied.get(&key) == Some(&sig) {
                    continue;
                }
                self.applied.insert(key, sig);
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
    fn mixer_round_trips_through_reflection() {
        use oxide_core::reflect::FieldValue;
        let m = mixer();
        let back = Vec::<AudioMixerGroup>::from_value(m.groups.to_value()).unwrap();
        assert_eq!(back, m.groups);
    }
}
