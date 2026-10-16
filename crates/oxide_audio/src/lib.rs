//! Oxide audio on kira: Unity's `AudioSource` / `AudioListener` with 2D and 3D (spatial)
//! playback, looping, pitch, volume, min/max distance, `PlayOneShot` and
//! `AudioSource.PlayClipAtPoint`. Built-in generated clips (`builtin:engine`,
//! `builtin:gunshot`, `builtin:beep`, `builtin:wind`, `builtin:skid`) work without files.
//!
//! If no audio device is available (servers, CI) everything runs silently.
//! Reference: <https://docs.unity3d.com/Manual/AudioOverview.html>

pub mod mixer;
pub mod synth;

use kira::sound::PlaybackState;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::track::{
    SpatialTrackBuilder, SpatialTrackDistances, SpatialTrackHandle, TrackBuilder, TrackHandle,
};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Tween};
use oxide_assets::{AssetId, AssetServer, AudioClip, Handle};
use oxide_core::{Reflect, debug_log_warning};
use oxide_math::{Quaternion, Vector3};
use oxide_scene::{Camera, Component, ComponentId, Ctx, GameObjectId, System, World};
use std::collections::HashMap;
use std::io::Cursor;

#[derive(Clone, Debug)]
#[doc(hidden)]
pub enum AudioCommand {
    Play,
    Stop,
    Pause,
    UnPause,
    OneShot(Handle<AudioClip>, f32),
}

/// Unity `AudioSource`.
#[derive(Clone, Debug, Reflect)]
pub struct AudioSource {
    pub clip: Option<Handle<AudioClip>>,
    #[reflect(range(0, 1))]
    pub volume: f32,
    #[reflect(range(-3, 3))]
    pub pitch: f32,
    pub looping: bool,
    pub play_on_awake: bool,
    pub mute: bool,
    /// 0 = 2D (no attenuation/panning), 1 = fully 3D.
    #[reflect(range(0, 1))]
    pub spatial_blend: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    /// Audio Mixer group to play through (Unity "Output"): `Assets/Main.mixer#SFX`.
    pub output: Option<Handle<mixer::AudioMixerGroupRef>>,
    #[reflect(skip)]
    #[doc(hidden)]
    pub commands: Vec<AudioCommand>,
    #[reflect(skip)]
    #[doc(hidden)]
    pub playing: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub started: bool,
}

impl Default for AudioSource {
    fn default() -> Self {
        Self {
            clip: None,
            volume: 1.0,
            pitch: 1.0,
            looping: false,
            play_on_awake: true,
            mute: false,
            spatial_blend: 1.0,
            min_distance: 1.0,
            max_distance: 500.0,
            output: None,
            commands: Vec::new(),
            playing: false,
            started: false,
        }
    }
}

impl AudioSource {
    pub fn new(clip: &str) -> Self {
        Self {
            clip: Some(Handle::from_path(clip)),
            ..Default::default()
        }
    }
    pub fn play(&mut self) {
        self.commands.push(AudioCommand::Play);
        self.playing = true;
    }
    pub fn stop(&mut self) {
        self.commands.push(AudioCommand::Stop);
        self.playing = false;
    }
    pub fn pause(&mut self) {
        self.commands.push(AudioCommand::Pause);
    }
    pub fn unpause(&mut self) {
        self.commands.push(AudioCommand::UnPause);
    }
    /// Unity `PlayOneShot`: plays a clip on this source without interrupting it.
    pub fn play_one_shot(&mut self, clip: Handle<AudioClip>, volume_scale: f32) {
        self.commands
            .push(AudioCommand::OneShot(clip, volume_scale));
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
}

impl Component for AudioSource {}

/// Unity `AudioListener` (put one on the main camera).
#[derive(Clone, Debug, Default, Reflect)]
pub struct AudioListener;

impl Component for AudioListener {
    fn allow_multiple(&self) -> bool {
        false
    }
}

enum Output {
    Spatial(SpatialTrackHandle),
    Flat(TrackHandle),
}

struct Voice {
    output: Output,
    sound: Option<StaticSoundHandle>,
    clip: Option<AssetId>,
    spatial: bool,
    volume: f32,
    pitch: f64,
    looping: bool,
    distances: (f32, f32),
    /// Mixer path, group and mixer generation it was routed to.
    route: Option<(String, String, u64)>,
}

/// The audio resource (Unity's `AudioSettings` + mixer).
pub struct Audio {
    manager: Option<AudioManager<DefaultBackend>>,
    listener: Option<kira::listener::ListenerHandle>,
    clips: HashMap<AssetId, StaticSoundData>,
    voices: HashMap<ComponentId, Voice>,
    one_shots: Vec<(Output, StaticSoundHandle)>,
    /// Master volume 0..1 (Unity `AudioListener.volume`).
    pub master_volume: f32,
    /// Unity `AudioListener.pause`.
    pub paused: bool,
    /// The editor's Game view "Mute Audio" (separate from the game's own settings).
    pub mute: bool,
    applied_master: f32,
    /// Running Audio Mixers by asset path.
    mixers: HashMap<String, mixer::MixerRuntime>,
    /// Snapshot and exposed-parameter state per mixer (kept even without a device).
    pub mixer_states: HashMap<String, mixer::MixerState>,
    mixer_generation: u64,
}

fn db(v: f32) -> f32 {
    if v <= 0.0001 { -80.0 } else { 20.0 * v.log10() }
}

impl Audio {
    pub fn new() -> Self {
        let manager = match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(m) => Some(m),
            Err(e) => {
                debug_log_warning!("audio disabled: {e}");
                None
            }
        };
        Self {
            manager,
            listener: None,
            clips: HashMap::new(),
            voices: HashMap::new(),
            one_shots: Vec::new(),
            master_volume: 1.0,
            paused: false,
            mute: false,
            applied_master: 1.0,
            mixers: HashMap::new(),
            mixer_states: HashMap::new(),
            mixer_generation: 0,
        }
    }

    /// Unity `AudioMixer.SetFloat`: sets an exposed parameter (`"MusicVol"`, dB for
    /// volumes) of the mixer at `mixer` (`Assets/Main.mixer`).
    pub fn set_float(&mut self, mixer: &str, name: &str, value: f32) {
        self.mixer_states
            .entry(mixer.to_owned())
            .or_default()
            .exposed
            .insert(name.to_owned(), value);
    }

    /// Unity `AudioMixer.ClearFloat`: the parameter follows the snapshots again.
    pub fn clear_float(&mut self, mixer: &str, name: &str) {
        if let Some(s) = self.mixer_states.get_mut(mixer) {
            s.exposed.remove(name);
        }
    }

    /// Unity `AudioMixer.GetFloat`: the value set by a script, if any.
    pub fn get_float(&self, mixer: &str, name: &str) -> Option<f32> {
        self.mixer_states.get(mixer)?.exposed.get(name).copied()
    }

    /// Unity `AudioMixerSnapshot.TransitionTo`: blends to `snapshot` over `seconds`.
    pub fn transition_to(
        &mut self,
        mixer: &str,
        snapshot: &str,
        seconds: f32,
        assets: &AssetServer,
    ) {
        let h: Handle<mixer::AudioMixer> = Handle::from_path(mixer);
        let current = assets.data.get(&h).map(|m| {
            self.mixer_states
                .get(mixer)
                .cloned()
                .unwrap_or_default()
                .values(m)
        });
        let st = self.mixer_states.entry(mixer.to_owned()).or_default();
        st.from = current.unwrap_or_default();
        st.snapshot = snapshot.to_owned();
        st.transition = Some((0.0, seconds.max(0.0)));
    }

    pub fn is_available(&self) -> bool {
        self.manager.is_some()
    }

    pub fn active_voices(&self) -> usize {
        self.voices
            .values()
            .filter(|v| {
                v.sound
                    .as_ref()
                    .is_some_and(|s| s.state() == PlaybackState::Playing)
            })
            .count()
            + self.one_shots.len()
    }

    fn clip_data(
        &mut self,
        assets: &AssetServer,
        h: &Handle<AudioClip>,
    ) -> Option<StaticSoundData> {
        if let Some(d) = self.clips.get(&h.id()) {
            return Some(d.clone());
        }
        let clip = assets.audio.get(h)?;
        match StaticSoundData::from_cursor(Cursor::new(clip.bytes.clone())) {
            Ok(d) => {
                self.clips.insert(h.id(), d.clone());
                Some(d)
            }
            Err(e) => {
                debug_log_warning!("cannot decode audio clip {}: {e}", h.path());
                None
            }
        }
    }

    fn make_output(
        &mut self,
        spatial: bool,
        pos: Vector3,
        min: f32,
        max: f32,
        persist: bool,
    ) -> Option<Output> {
        self.make_output_in(None, spatial, pos, min, max, persist)
    }

    /// An output track, inside a mixer group's track when `route` names one.
    fn make_output_in(
        &mut self,
        route: Option<(&str, &str)>,
        spatial: bool,
        pos: Vector3,
        min: f32,
        max: f32,
        persist: bool,
    ) -> Option<Output> {
        if let Some((m, g)) = route
            && let Some(group) = self.mixers.get_mut(m).and_then(|r| r.groups.get_mut(g))
        {
            let t = &mut group.track;
            return if spatial {
                let listener = self.listener.as_ref()?.id();
                let b = SpatialTrackBuilder::new()
                    .distances(SpatialTrackDistances {
                        min_distance: min.max(0.01),
                        max_distance: max.max(min + 0.01),
                    })
                    .persist_until_sounds_finish(persist);
                t.add_spatial_sub_track(listener, pos, b)
                    .ok()
                    .map(Output::Spatial)
            } else {
                t.add_sub_track(TrackBuilder::new().persist_until_sounds_finish(persist))
                    .ok()
                    .map(Output::Flat)
            };
        }
        let m = self.manager.as_mut()?;
        if spatial {
            let listener = self.listener.as_ref()?.id();
            let b = SpatialTrackBuilder::new()
                .distances(SpatialTrackDistances {
                    min_distance: min.max(0.01),
                    max_distance: max.max(min + 0.01),
                })
                .persist_until_sounds_finish(persist);
            m.add_spatial_sub_track(listener, pos, b)
                .ok()
                .map(Output::Spatial)
        } else {
            m.add_sub_track(TrackBuilder::new().persist_until_sounds_finish(persist))
                .ok()
                .map(Output::Flat)
        }
    }

    /// Unity `AudioSource.PlayClipAtPoint`: fire-and-forget 3D sound.
    pub fn play_clip_at_point(
        &mut self,
        assets: &AssetServer,
        clip: &Handle<AudioClip>,
        position: Vector3,
        volume: f32,
    ) {
        let Some(data) = self.clip_data(assets, clip) else {
            return;
        };
        let Some(mut out) = self.make_output(true, position, 2.0, 400.0, true) else {
            return;
        };
        let data = data.volume(db(volume));
        let handle = match &mut out {
            Output::Spatial(t) => t.play(data).ok(),
            Output::Flat(t) => t.play(data).ok(),
        };
        if let Some(h) = handle {
            self.one_shots.push((out, h));
        }
    }

    /// 2D one-shot (UI sounds).
    pub fn play_2d(&mut self, assets: &AssetServer, clip: &Handle<AudioClip>, volume: f32) {
        let Some(data) = self.clip_data(assets, clip) else {
            return;
        };
        let Some(mut out) = self.make_output(false, Vector3::ZERO, 1.0, 1.0, true) else {
            return;
        };
        if let Output::Flat(t) = &mut out
            && let Ok(h) = t.play(data.volume(db(volume)))
        {
            self.one_shots.push((out, h));
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

/// Registers the built-in generated clips as assets.
pub fn register_builtin_clips(assets: &mut AssetServer) {
    for (name, bytes) in [
        ("builtin:engine", synth::engine_loop()),
        ("builtin:gunshot", synth::gunshot()),
        ("builtin:beep", synth::beep()),
        ("builtin:wind", synth::wind_loop()),
        ("builtin:skid", synth::skid_loop()),
    ] {
        assets.audio.insert(
            name,
            AudioClip {
                name: name.trim_start_matches("builtin:").into(),
                bytes: bytes.into(),
            },
        );
    }
}

/// Plays sources and moves the listener; runs after `LateUpdate`.
#[derive(Default)]
pub struct AudioSystem;

impl System for AudioSystem {
    fn name(&self) -> &'static str {
        "Audio"
    }

    fn late_update(&mut self, world: &mut World) {
        let Some(mut audio) = world.resources.remove::<Audio>() else {
            return;
        };
        if audio.manager.is_some() {
            sync(world, &mut audio);
        }
        world.resources.insert(audio);
    }
}

fn listener_pose(world: &World) -> Option<(Vector3, Quaternion)> {
    let id = world
        .query::<AudioListener>()
        .map(|(id, _)| id)
        .next()
        .or_else(|| world.find_with_tag("MainCamera").into_iter().next())
        .or_else(|| world.query::<Camera>().map(|(id, _)| id).next())?;
    Some((world.position(id), world.rotation(id)))
}

fn sync(world: &mut World, audio: &mut Audio) {
    let Some((lp, lr)) = listener_pose(world) else {
        return;
    };
    // Kira puts the ears at the listener's local ±X, the same as Unity's right vector, so
    // positions and orientation pass through unchanged.
    let mirror = |v: Vector3| v;
    let lr_k: Quaternion = lr;
    match &mut audio.listener {
        Some(l) => {
            l.set_position(mirror(lp), Tween::default());
            l.set_orientation(lr_k, Tween::default());
        }
        None => {
            if let Some(m) = audio.manager.as_mut() {
                audio.listener = m.add_listener(mirror(lp), lr_k).ok();
            }
        }
    }
    let v = if audio.paused || audio.mute {
        0.0
    } else {
        audio.master_volume
    };
    if (v - audio.applied_master).abs() > 1e-4 {
        if let Some(m) = audio.manager.as_mut() {
            m.main_track().set_volume(db(v), Tween::default());
        }
        audio.applied_master = v;
    }
    audio
        .one_shots
        .retain(|(_, h)| h.state() != PlaybackState::Stopped);

    update_mixers(world, audio);

    let sources: Vec<(GameObjectId, ComponentId)> = world.query_ids::<AudioSource>();
    let mut alive = std::collections::HashSet::new();
    for (obj, cid) in sources {
        alive.insert(cid);
        let pos = world.position(obj);
        let Some(src) = world.component_mut::<AudioSource>(obj, cid) else {
            continue;
        };
        let mut cmds = std::mem::take(&mut src.commands);
        if !src.started {
            src.started = true;
            if src.play_on_awake && src.clip.is_some() {
                cmds.insert(0, AudioCommand::Play);
                src.playing = true;
            }
        }
        let s = src.clone();
        let assets = world.resource::<AssetServer>();
        let spatial = s.spatial_blend >= 0.5;
        // Mixer routing: the group's track (rebuilt mixers re-route their voices).
        let route = s.output.as_ref().and_then(|h| {
            let (m, g) = mixer::split_group_path(h.path());
            let generation = audio.mixers.get(m)?.generation;
            Some((m.to_owned(), g.to_owned(), generation))
        });
        let mixer_pitch = route.as_ref().map_or(1.0, |(m, g, _)| {
            let h: Handle<mixer::AudioMixer> = Handle::from_path(m.as_str());
            match assets.data.get(&h) {
                Some(mx) => {
                    let st = audio
                        .mixer_states
                        .get(m.as_str())
                        .cloned()
                        .unwrap_or_default();
                    mx.pitch(g.as_str(), &st.values(mx))
                }
                None => 1.0,
            }
        });
        let need_new = audio.voices.get(&cid).is_none_or(|v| {
            v.spatial != spatial
                || v.distances != (s.min_distance, s.max_distance)
                || v.route != route
        });
        if need_new {
            if let Some(mut old) = audio.voices.remove(&cid)
                && let Some(snd) = old.sound.as_mut()
            {
                snd.stop(Tween::default());
                // A playing source keeps playing on its new route.
                if src_was_playing(&s) {
                    cmds.insert(0, AudioCommand::Play);
                }
            }
            let Some(out) = audio.make_output_in(
                route.as_ref().map(|(m, g, _)| (m.as_str(), g.as_str())),
                spatial,
                mirror(pos),
                s.min_distance,
                s.max_distance,
                false,
            ) else {
                continue;
            };
            audio.voices.insert(
                cid,
                Voice {
                    output: out,
                    sound: None,
                    clip: None,
                    spatial,
                    volume: -1.0,
                    pitch: -1.0,
                    looping: false,
                    distances: (s.min_distance, s.max_distance),
                    route: route.clone(),
                },
            );
        }
        let vol = if s.mute { 0.0 } else { s.volume };
        let pitch = s.pitch * mixer_pitch;
        for c in cmds {
            match c {
                AudioCommand::Play => {
                    let Some(clip) = &s.clip else { continue };
                    let Some(mut data) = audio.clip_data(assets, clip) else {
                        continue;
                    };
                    if s.looping {
                        data = data.loop_region(..);
                    }
                    data = data.volume(db(vol)).playback_rate(pitch as f64);
                    let v = audio.voices.get_mut(&cid).expect("voice");
                    if let Some(old) = v.sound.as_mut() {
                        old.stop(Tween::default());
                    }
                    v.sound = match &mut v.output {
                        Output::Spatial(t) => t.play(data).ok(),
                        Output::Flat(t) => t.play(data).ok(),
                    };
                    v.clip = Some(clip.id());
                    v.volume = vol;
                    v.pitch = pitch as f64;
                    v.looping = s.looping;
                }
                AudioCommand::Stop => {
                    if let Some(snd) = audio.voices.get_mut(&cid).and_then(|v| v.sound.as_mut()) {
                        snd.stop(Tween::default());
                    }
                }
                AudioCommand::Pause => {
                    if let Some(snd) = audio.voices.get_mut(&cid).and_then(|v| v.sound.as_mut()) {
                        snd.pause(Tween::default());
                    }
                }
                AudioCommand::UnPause => {
                    if let Some(snd) = audio.voices.get_mut(&cid).and_then(|v| v.sound.as_mut()) {
                        snd.resume(Tween::default());
                    }
                }
                AudioCommand::OneShot(clip, scale) => {
                    let Some(data) = audio.clip_data(assets, &clip) else {
                        continue;
                    };
                    let data = data.volume(db(vol * scale)).playback_rate(pitch as f64);
                    let v = audio.voices.get_mut(&cid).expect("voice");
                    let _ = match &mut v.output {
                        Output::Spatial(t) => t.play(data).map(|_| ()),
                        Output::Flat(t) => t.play(data).map(|_| ()),
                    };
                }
            }
        }
        let v = audio.voices.get_mut(&cid).expect("voice");
        if let Output::Spatial(t) = &mut v.output {
            t.set_position(mirror(pos), Tween::default());
        }
        if let Some(snd) = v.sound.as_mut() {
            if (v.volume - vol).abs() > 1e-4 {
                snd.set_volume(db(vol), Tween::default());
                v.volume = vol;
            }
            if (v.pitch - pitch as f64).abs() > 1e-4 {
                snd.set_playback_rate(pitch as f64, Tween::default());
                v.pitch = pitch as f64;
            }
        }
        let playing = v.sound.as_ref().is_some_and(|h| {
            matches!(
                h.state(),
                PlaybackState::Playing | PlaybackState::Pausing | PlaybackState::Resuming
            )
        });
        if let Some(src) = world.component_mut::<AudioSource>(obj, cid) {
            src.playing = playing;
        }
    }
    // Sources that were removed or disabled stop playing.
    audio.voices.retain(|cid, v| {
        let keep = alive.contains(cid);
        if !keep && let Some(s) = v.sound.as_mut() {
            s.stop(Tween::default());
        }
        keep
    });
}

fn src_was_playing(s: &AudioSource) -> bool {
    s.playing
}

/// Builds or rebuilds the kira tracks of every mixer in use (by sources or scripts) and
/// pushes their current values, advancing snapshot transitions.
fn update_mixers(world: &mut World, audio: &mut Audio) {
    let dt = world
        .try_resource::<oxide_core::Time>()
        .map_or(0.0, |t| t.unscaled_delta_time());
    let mut paths: Vec<String> = world
        .query::<AudioSource>()
        .filter_map(|(_, s)| {
            s.output
                .as_ref()
                .map(|h| mixer::split_group_path(h.path()).0.to_owned())
        })
        .collect();
    paths.extend(audio.mixer_states.keys().cloned());
    paths.sort();
    paths.dedup();
    let assets = world.resource_mut::<AssetServer>();
    for path in paths {
        let h: Handle<mixer::AudioMixer> = Handle::from_path(path.clone());
        let Some(m) = assets.data_asset(&h).cloned() else {
            continue;
        };
        let st = audio.mixer_states.entry(path.clone()).or_default();
        if let Some((t, d)) = st.transition.as_mut() {
            *t += dt;
            if *t >= *d {
                st.transition = None;
            }
        }
        let values = st.values(&m);
        let rebuild = audio
            .mixers
            .get(&path)
            .is_none_or(|r| r.structure != m.structure());
        if rebuild && let Some(mgr) = audio.manager.as_mut() {
            audio.mixer_generation += 1;
            match mixer::MixerRuntime::build(mgr, &m, audio.mixer_generation) {
                Some(rt) => {
                    audio.mixers.insert(path.clone(), rt);
                }
                None => continue,
            }
        }
        if let Some(rt) = audio.mixers.get_mut(&path) {
            rt.apply(&m, &values);
        }
    }
}

/// Script helpers: `ctx.play_clip_at_point("builtin:gunshot", pos, 1.0)`, Audio Mixer
/// exposed parameters and snapshots.
pub trait AudioExt {
    fn play_clip_at_point(&mut self, clip: &str, position: Vector3, volume: f32);
    /// Unity `AudioMixer.SetFloat("MusicVol", -10)` on the mixer at `mixer`.
    fn set_mixer_float(&mut self, mixer: &str, name: &str, value: f32);
    /// Unity `mixer.FindSnapshot(name).TransitionTo(seconds)`.
    fn transition_to_snapshot(&mut self, mixer: &str, snapshot: &str, seconds: f32);
}

impl AudioExt for Ctx<'_> {
    fn play_clip_at_point(&mut self, clip: &str, position: Vector3, volume: f32) {
        let h = Handle::<AudioClip>::from_path(clip);
        let Some(mut audio) = self.world.resources.remove::<Audio>() else {
            return;
        };
        audio.play_clip_at_point(self.world.resource::<AssetServer>(), &h, position, volume);
        self.world.resources.insert(audio);
    }

    fn set_mixer_float(&mut self, mixer: &str, name: &str, value: f32) {
        if let Some(a) = self.world.try_resource_mut::<Audio>() {
            a.set_float(mixer, name, value);
        }
    }

    fn transition_to_snapshot(&mut self, mixer: &str, snapshot: &str, seconds: f32) {
        let Some(mut audio) = self.world.resources.remove::<Audio>() else {
            return;
        };
        audio.transition_to(
            mixer,
            snapshot,
            seconds,
            self.world.resource::<AssetServer>(),
        );
        self.world.resources.insert(audio);
    }
}

/// Installs the `Audio` resource, built-in clips and components; returns the system.
pub fn install(world: &mut World) -> Box<dyn System> {
    world.resources.insert(Audio::new());
    if let Some(a) = world.try_resource_mut::<AssetServer>() {
        register_builtin_clips(a);
        a.register_data_asset::<mixer::AudioMixer>(
            oxide_assets::AssetTypeInfo::new("AudioMixer", &["mixer"])
                .menu("Audio Mixer", "New Audio Mixer", 55)
                .color(230, 140, 200),
        );
    }
    world.registry.register::<AudioSource>("Audio/Audio Source");
    world
        .registry
        .register::<AudioListener>("Audio/Audio Listener");
    Box::new(AudioSystem)
}
