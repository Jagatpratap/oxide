//! Oxide Timeline (Unity's Timeline package): sequenced animation, activation, audio,
//! signals, nested Timelines and particles, and script clips, played by a
//! [`PlayableDirector`].
//!
//! The Timeline asset (`.playable`) holds tracks; the Director binds each track to a scene
//! object by track name and plays the asset. Every frame the Director evaluates the
//! Timeline at its time: animation tracks blend their clips over the Animator's pose
//! (eases and overlapping clips blend), activation tracks switch objects on and off, audio
//! tracks keep their sounds at the right position, control tracks drive nested Directors
//! and particle systems, signal emitters send messages through Signal Receivers, and
//! Playable tracks run script `PlayableBehaviour`s. The editor's Timeline window scrubs the
//! same evaluation in Edit mode.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.timeline@1.8/manual/index.html>

pub mod asset;
pub mod mix;
pub mod playables;

pub use asset::*;
pub use mix::{ActiveClip, evaluate_track};
pub use playables::{FrameInfo, PlayableBehaviour, PlayableBehaviours, save_fields};

use oxide_animation::{Animator, ClipSample, RestPose, play_clips};
use oxide_assets::{AssetServer, Handle};
use oxide_audio::{Audio, AudioSource};
use oxide_core::reflect::Value;
use oxide_core::{Reflect, Time};
use oxide_math::{Quaternion, QuaternionExt, Vector3};
use oxide_particles::ParticleSystem;
use oxide_scene::{Component, ComponentId, GameObjectId, System, World};
use std::collections::{HashMap, HashSet};

/// Unity `DirectorUpdateMode`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectorUpdateMode {
    #[default]
    GameTime,
    UnscaledGameTime,
    /// Only advances when a script sets the time and calls `evaluate`.
    Manual,
}

/// Unity `DirectorWrapMode`: what happens at the end of the Timeline.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectorWrapMode {
    /// Stays on the last frame.
    Hold,
    Loop,
    /// Stops (and resets).
    #[default]
    None,
}

/// Unity `PlayState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// A track bound to a scene object (Unity's Bindings list on the Director).
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct TrackBinding {
    pub track: String,
    pub object: Option<GameObjectId>,
}

/// An exposed reference: a name a clip uses (Control clips' Source Game Object) mapped to
/// a scene object.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct ExposedReference {
    pub name: String,
    pub object: Option<GameObjectId>,
}

#[derive(Clone, Debug)]
#[doc(hidden)]
pub enum DirectorCommand {
    Play,
    Pause,
    Resume,
    Stop,
    Evaluate,
}

/// Playback state that is not saved.
#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct DirectorRuntime {
    pub state: PlayState,
    pub time: f64,
    /// Time of the last evaluation (signals fire between it and the next).
    pub last: Option<f64>,
    pub started: bool,
    pub duration: f64,
    /// Driven by a parent Timeline's Control clip this frame.
    pub controlled: bool,
    pub commands: Vec<DirectorCommand>,
    /// Active state of objects before the Timeline first switched them (Revert).
    pub activation_backup: HashMap<GameObjectId, bool>,
    /// Markers that already fired (`emit_once`): (track index or MAX, marker index).
    pub emitted: HashSet<(usize, usize)>,
    /// Animation binding paths per track.
    pub anim_bindings: HashMap<String, HashMap<String, Option<GameObjectId>>>,
    /// Pose of animated objects before the Timeline moved them (eases blend toward it
    /// when no Animator Controller supplies a pose).
    pub rest: HashMap<String, RestPose>,
    /// Control clips under the playhead last frame, and the prefab instances they made.
    pub control_active: HashSet<(usize, usize)>,
    pub spawned: HashMap<(usize, usize), GameObjectId>,
    /// Playable clips under the playhead last frame.
    pub playables_active: HashSet<(usize, usize)>,
    /// Activation tracks touched (post-playback state applies on stop).
    pub activated: HashSet<usize>,
}

/// Unity `PlayableDirector`.
#[derive(Clone, Debug, Reflect)]
pub struct PlayableDirector {
    /// Unity "Playable": the Timeline played.
    pub playable_asset: Option<Handle<TimelineAsset>>,
    pub update_method: DirectorUpdateMode,
    pub play_on_awake: bool,
    pub wrap_mode: DirectorWrapMode,
    pub initial_time: f32,
    pub bindings: Vec<TrackBinding>,
    pub exposed_references: Vec<ExposedReference>,
    #[reflect(skip)]
    #[doc(hidden)]
    pub rt: DirectorRuntime,
}

impl Default for PlayableDirector {
    fn default() -> Self {
        Self {
            playable_asset: None,
            update_method: DirectorUpdateMode::GameTime,
            play_on_awake: true,
            wrap_mode: DirectorWrapMode::None,
            initial_time: 0.0,
            bindings: Vec::new(),
            exposed_references: Vec::new(),
            rt: DirectorRuntime::default(),
        }
    }
}

impl Component for PlayableDirector {}

impl PlayableDirector {
    pub fn new(asset: &str) -> Self {
        Self {
            playable_asset: Some(Handle::from_path(asset)),
            ..Default::default()
        }
    }

    pub fn play(&mut self) {
        self.rt.commands.push(DirectorCommand::Play);
    }
    pub fn pause(&mut self) {
        self.rt.commands.push(DirectorCommand::Pause);
    }
    pub fn resume(&mut self) {
        self.rt.commands.push(DirectorCommand::Resume);
    }
    pub fn stop(&mut self) {
        self.rt.commands.push(DirectorCommand::Stop);
    }
    /// Applies the Timeline at the current time this frame (also while paused or stopped).
    pub fn evaluate(&mut self) {
        self.rt.commands.push(DirectorCommand::Evaluate);
    }
    pub fn time(&self) -> f64 {
        self.rt.time
    }
    /// Unity `PlayableDirector.time` setter.
    pub fn set_time(&mut self, t: f64) {
        self.rt.time = t.max(0.0);
    }
    pub fn state(&self) -> PlayState {
        self.rt.state
    }
    /// The Timeline's length (known after the first evaluation).
    pub fn duration(&self) -> f64 {
        self.rt.duration
    }

    /// Unity `GetGenericBinding`.
    pub fn binding(&self, track: &str) -> Option<GameObjectId> {
        self.bindings
            .iter()
            .find(|b| b.track == track)
            .and_then(|b| b.object)
    }

    /// Unity `SetGenericBinding`.
    pub fn set_binding(&mut self, track: &str, object: Option<GameObjectId>) {
        match self.bindings.iter_mut().find(|b| b.track == track) {
            Some(b) => b.object = object,
            None => self.bindings.push(TrackBinding {
                track: track.to_owned(),
                object,
            }),
        }
    }

    /// Unity `GetReferenceValue`.
    pub fn reference(&self, name: &str) -> Option<GameObjectId> {
        self.exposed_references
            .iter()
            .find(|r| r.name == name)
            .and_then(|r| r.object)
    }

    /// Unity `SetReferenceValue`.
    pub fn set_reference(&mut self, name: &str, object: Option<GameObjectId>) {
        match self.exposed_references.iter_mut().find(|r| r.name == name) {
            Some(r) => r.object = object,
            None => self.exposed_references.push(ExposedReference {
                name: name.to_owned(),
                object,
            }),
        }
    }
}

/// One reaction of a Signal Receiver: the message sent when its signal arrives.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct SignalReaction {
    pub signal: Option<Handle<SignalAsset>>,
    /// Receives the message (default: the receiver's own object).
    pub target: Option<GameObjectId>,
    /// Message name (`SendMessage`): a script method such as `OnDoorOpen`.
    pub method: String,
    /// Sent as the message's argument (empty = none).
    pub parameter: String,
}

/// Unity `SignalReceiver`: reacts to signals from Signal Emitters on tracks bound to this
/// object (or Timeline markers of a Director on it).
#[derive(Clone, Debug, Default, Reflect)]
pub struct SignalReceiver {
    pub reactions: Vec<SignalReaction>,
}

impl Component for SignalReceiver {}

/// How an evaluation runs.
#[derive(Clone, Copy, Debug)]
pub struct EvalMode {
    /// Game time is moving (audio plays, behaviours see `playing`).
    pub playing: bool,
    pub dt: f32,
    /// Send signals (Play mode only, as in Unity).
    pub signals: bool,
    pub audio: bool,
}

impl EvalMode {
    /// The editor scrubbing or previewing in Edit mode.
    pub fn preview(playing: bool, dt: f32) -> Self {
        Self {
            playing,
            dt,
            signals: false,
            audio: playing,
        }
    }
}

fn director_key(id: GameObjectId) -> u64 {
    let b = id.to_bits();
    (b ^ (b >> 29) ^ (b >> 47)) & 0xffff_ffff
}

fn audio_key(id: GameObjectId, track: usize, clip: usize) -> u64 {
    (director_key(id) << 32) | ((track as u64 & 0xffff) << 16) | (clip as u64 & 0xffff)
}

fn euler(v: Vector3) -> Quaternion {
    Quaternion::euler_v(v)
}

fn find_director(world: &World, id: GameObjectId) -> Option<ComponentId> {
    world
        .query_ids::<PlayableDirector>()
        .into_iter()
        .find(|(o, _)| *o == id)
        .map(|(_, c)| c)
}

/// Loads the Director's Timeline (a copy, so evaluation can change the world freely).
pub fn timeline_of(world: &mut World, id: GameObjectId, cid: ComponentId) -> Option<TimelineAsset> {
    let h = world
        .component::<PlayableDirector>(id, cid)?
        .playable_asset
        .clone()?;
    world
        .try_resource_mut::<AssetServer>()?
        .data_asset(&h)
        .cloned()
}

/// Evaluates the Director at its current time (Unity `PlayableDirector.Evaluate`).
pub fn evaluate(world: &mut World, id: GameObjectId, cid: ComponentId, mode: EvalMode) {
    let Some(mut assets) = world.resources.remove::<AssetServer>() else {
        return;
    };
    evaluate_with(world, &mut assets, id, cid, mode, 0);
    world.resources.insert(assets);
}

/// Sets the Director's time and evaluates it (editor scrubbing; the Timeline window).
pub fn preview(world: &mut World, id: GameObjectId, time: f64, playing: bool, dt: f32) {
    let Some(cid) = find_director(world, id) else {
        return;
    };
    if let Some(d) = world.component_mut::<PlayableDirector>(id, cid) {
        d.rt.time = time.max(0.0);
    }
    evaluate(world, id, cid, EvalMode::preview(playing, dt));
}

/// Undoes what playing did (activation post-playback states, control clips, spawned
/// prefabs, sounds, behaviours): the Director stopped or the editor preview ended.
pub fn release(world: &mut World, id: GameObjectId, cid: ComponentId) {
    let Some(timeline) = timeline_of(world, id, cid) else {
        if let Some(d) = world.component_mut::<PlayableDirector>(id, cid) {
            d.rt.last = None;
        }
        return;
    };
    let Some(mut rt) = world
        .component_mut::<PlayableDirector>(id, cid)
        .map(|d| std::mem::take(&mut d.rt))
    else {
        return;
    };
    let bindings = world
        .component::<PlayableDirector>(id, cid)
        .cloned()
        .unwrap_or_default();
    for &ti in &rt.activated {
        let Some(track) = timeline.tracks.get(ti) else {
            continue;
        };
        let Some(obj) = bindings.binding(&track.name) else {
            continue;
        };
        apply_post_playback(world, &rt, obj, track.post_playback_state);
    }
    let active: Vec<(usize, usize)> = rt.control_active.drain().collect();
    for (ti, ci) in active {
        if let Some(c) = timeline.tracks.get(ti).and_then(|t| t.clips.get(ci)) {
            exit_control(world, &mut rt, &bindings, c, (ti, ci));
        }
    }
    if let Some(mut pb) = world.resources.remove::<PlayableBehaviours>() {
        for (ti, ci) in rt.playables_active.drain() {
            if let Some(mut b) = pb.instances.remove(&(id, ti, ci)) {
                let info = FrameInfo {
                    director: id,
                    binding: timeline
                        .tracks
                        .get(ti)
                        .and_then(|t| bindings.binding(&t.name)),
                    time: 0.0,
                    duration: 0.0,
                    weight: 0.0,
                    playing: false,
                    delta_time: 0.0,
                };
                b.on_behaviour_pause(world, &info);
            }
        }
        world.resources.insert(pb);
    }
    if let Some(a) = world.try_resource_mut::<Audio>() {
        let dk = director_key(id);
        a.retain_timed_voices(|k| k >> 32 != dk);
    }
    if let Some(d) = world.component_mut::<PlayableDirector>(id, cid) {
        let keep_time = rt.time;
        d.rt = DirectorRuntime {
            time: keep_time,
            started: rt.started,
            ..Default::default()
        };
    }
}

fn apply_post_playback(
    world: &mut World,
    rt: &DirectorRuntime,
    obj: GameObjectId,
    s: PostPlaybackState,
) {
    match s {
        PostPlaybackState::Active => world.set_active(obj, true),
        PostPlaybackState::Inactive => world.set_active(obj, false),
        PostPlaybackState::Revert => {
            if let Some(&a) = rt.activation_backup.get(&obj) {
                world.set_active(obj, a);
            }
        }
        PostPlaybackState::LeaveAsIs => {}
    }
}

fn set_active_backed(world: &mut World, rt: &mut DirectorRuntime, obj: GameObjectId, on: bool) {
    let Some(cur) = world.get(obj).map(|o| o.active_self()) else {
        return;
    };
    rt.activation_backup.entry(obj).or_insert(cur);
    if cur != on {
        world.set_active(obj, on);
    }
}

fn particles_under(world: &World, root: GameObjectId) -> Vec<(GameObjectId, ComponentId)> {
    let ids: HashSet<GameObjectId> = world.descendants(root).into_iter().collect();
    world
        .query_ids::<ParticleSystem>()
        .into_iter()
        .filter(|(o, _)| ids.contains(o))
        .collect()
}

fn directors_under(
    world: &World,
    root: GameObjectId,
    except: GameObjectId,
) -> Vec<(GameObjectId, ComponentId)> {
    let ids: HashSet<GameObjectId> = world.descendants(root).into_iter().collect();
    world
        .query_ids::<PlayableDirector>()
        .into_iter()
        .filter(|(o, _)| *o != except && ids.contains(o))
        .collect()
}

fn exit_control(
    world: &mut World,
    rt: &mut DirectorRuntime,
    dir: &PlayableDirector,
    c: &TimelineClip,
    key: (usize, usize),
) {
    if let Some(inst) = rt.spawned.remove(&key) {
        world.destroy_immediate(inst);
    }
    let Some(src) = dir.reference(&c.source_game_object) else {
        return;
    };
    if c.update_particles {
        for (o, pc) in particles_under(world, src) {
            if let Some(p) = world.component_mut::<ParticleSystem>(o, pc) {
                p.stop();
            }
        }
    }
    if c.update_directors {
        for (o, dc) in directors_under(world, src, src) {
            if let Some(d) = world.component_mut::<PlayableDirector>(o, dc) {
                d.rt.controlled = false;
            }
            release(world, o, dc);
        }
    }
    if c.control_activation {
        apply_post_playback(world, rt, src, c.post_playback);
    }
}

fn send_signal(world: &mut World, receiver: GameObjectId, signal: &str) {
    let reactions: Vec<SignalReaction> = world
        .get_component::<SignalReceiver>(receiver)
        .map(|r| r.reactions.clone())
        .unwrap_or_default();
    for r in reactions {
        if r.signal.as_ref().map(|h| h.path()) != Some(signal) || r.method.is_empty() {
            continue;
        }
        let arg = if r.parameter.is_empty() {
            Value::Null
        } else {
            Value::String(r.parameter.clone())
        };
        world.send_message(r.target.unwrap_or(receiver), &r.method, arg);
    }
}

fn evaluate_with(
    world: &mut World,
    assets: &mut AssetServer,
    id: GameObjectId,
    cid: ComponentId,
    mode: EvalMode,
    depth: usize,
) {
    let Some(dir) = world.component::<PlayableDirector>(id, cid).cloned() else {
        return;
    };
    let Some(timeline) = dir
        .playable_asset
        .as_ref()
        .and_then(|h| assets.data_asset(h))
        .cloned()
    else {
        return;
    };
    let mut rt = dir.rt.clone();
    let duration = f64::from(timeline.duration());
    rt.duration = duration;
    let t = rt.time as f32;
    let prev = rt.last;
    let tracks = &timeline.tracks;

    // Activation first, so objects switched on this frame are animated this frame.
    for (ti, track) in tracks.iter().enumerate() {
        if track.track_type != TrackType::Activation || timeline.is_muted(track) {
            continue;
        }
        let Some(obj) = dir.binding(&track.name) else {
            continue;
        };
        let on = track.clips.iter().any(|c| t >= c.start && t < c.end());
        set_active_backed(world, &mut rt, obj, on);
        rt.activated.insert(ti);
    }

    // Animation.
    for track in tracks {
        if track.track_type != TrackType::Animation || timeline.is_muted(track) {
            continue;
        }
        let Some(root) = dir.binding(&track.name) else {
            continue;
        };
        // An Animator on the bound object or its children animates its own hierarchy.
        let root = if world.get_component::<Animator>(root).is_some() {
            root
        } else {
            world
                .descendants(root)
                .into_iter()
                .find(|o| world.get_component::<Animator>(*o).is_some())
                .unwrap_or(root)
        };
        let active = evaluate_track(track, t);
        let mut samples = Vec::new();
        let mut total = 0.0;
        let mut heaviest: Option<(&TimelineClip, f32)> = None;
        for a in &active {
            let c = &track.clips[a.index];
            let Some(clip) = c
                .animation_clip
                .as_ref()
                .and_then(|h| assets.animations.get(h))
            else {
                continue;
            };
            let looping = match c.loop_mode {
                LoopMode::UseSourceAsset => clip.looping,
                LoopMode::On => true,
                LoopMode::Off => false,
            };
            let len = clip.length.max(1e-4);
            let time = if looping {
                a.source_time.rem_euclid(len)
            } else {
                a.source_time.clamp(0.0, len)
            };
            if a.weight > 0.0 {
                samples.push(ClipSample {
                    clip,
                    time,
                    weight: a.weight,
                });
                total += a.weight;
                if heaviest.is_none_or(|(_, w)| a.weight > w) {
                    heaviest = Some((c, a.weight));
                }
            }
        }
        if samples.is_empty() {
            continue;
        }
        let mut offset = None;
        if let Some((c, _)) = heaviest
            && (c.position_offset != Vector3::ZERO || c.rotation_offset != Vector3::ZERO)
        {
            offset = Some((c.position_offset, euler(c.rotation_offset)));
        }
        if track.apply_offsets {
            let (tp, tr) = (track.position, euler(track.rotation));
            offset = Some(match offset {
                Some((p, r)) => (tr * p + tp, tr * r),
                None => (tp, tr),
            });
        }
        let mask = track
            .avatar_mask
            .as_ref()
            .and_then(|h| assets.masks.get(h))
            .cloned();
        let has_controller = world
            .get_component::<Animator>(root)
            .is_some_and(|a| a.controller.is_some());
        let bindings = rt.anim_bindings.entry(track.name.clone()).or_default();
        let rest = (!has_controller).then(|| rt.rest.entry(track.name.clone()).or_default());
        play_clips(
            world,
            assets,
            root,
            bindings,
            &samples,
            total.min(1.0),
            offset,
            mask.as_ref(),
            rest,
        );
    }

    // Audio.
    let mut voices: HashSet<u64> = HashSet::new();
    if mode.audio {
        for (ti, track) in tracks.iter().enumerate() {
            if track.track_type != TrackType::Audio || timeline.is_muted(track) {
                continue;
            }
            let binding = dir.binding(&track.name);
            let at = binding.and_then(|b| {
                let src = world.get_component::<AudioSource>(b)?;
                (src.spatial_blend > 0.5).then(|| world.position(b))
            });
            for a in evaluate_track(track, t) {
                let c = &track.clips[a.index];
                let Some(h) = c.audio_clip.clone() else {
                    continue;
                };
                if a.weight <= 0.0 {
                    continue;
                }
                let key = audio_key(id, ti, a.index);
                voices.insert(key);
                if let Some(audio) = world.try_resource_mut::<Audio>() {
                    audio.sync_timed_voice(
                        assets,
                        key,
                        &h,
                        f64::from(a.source_time.max(0.0)),
                        c.volume * track.volume * a.weight,
                        c.time_scale,
                        c.looping,
                        mode.playing,
                        at,
                    );
                }
            }
        }
    }
    if let Some(audio) = world.try_resource_mut::<Audio>() {
        let dk = director_key(id);
        audio.retain_timed_voices(|k| k >> 32 != dk || voices.contains(&k));
    }

    // Control.
    let mut control_now: HashSet<(usize, usize)> = HashSet::new();
    for (ti, track) in tracks.iter().enumerate() {
        if track.track_type != TrackType::Control || timeline.is_muted(track) {
            continue;
        }
        for a in evaluate_track(track, t) {
            let c = &track.clips[a.index];
            let key = (ti, a.index);
            control_now.insert(key);
            let src = dir.reference(&c.source_game_object);
            let entering = !rt.control_active.contains(&key);
            if entering {
                if !c.prefab.is_empty() {
                    let parent = src.or_else(|| world.parent(id));
                    match world.instantiate_prefab(&c.prefab, parent) {
                        Ok(inst) => {
                            rt.spawned.insert(key, inst);
                        }
                        Err(e) => oxide_core::debug_log_warning!("Control clip: {e}"),
                    }
                }
                if let Some(src) = src
                    && c.update_particles
                {
                    for (o, pc) in particles_under(world, src) {
                        if let Some(p) = world.component_mut::<ParticleSystem>(o, pc) {
                            p.clear();
                            p.play();
                        }
                    }
                }
            }
            let Some(src) = src.or_else(|| rt.spawned.get(&key).copied()) else {
                continue;
            };
            if c.control_activation {
                set_active_backed(world, &mut rt, src, true);
            }
            if c.update_directors && depth < 8 {
                for (o, dc) in directors_under(world, src, id) {
                    if let Some(d) = world.component_mut::<PlayableDirector>(o, dc) {
                        d.rt.controlled = true;
                        d.rt.time = f64::from(a.source_time.max(0.0));
                        if d.rt.state == PlayState::Stopped {
                            d.rt.state = PlayState::Playing;
                        }
                    }
                    evaluate_with(world, assets, o, dc, mode, depth + 1);
                }
            }
        }
    }
    let leaving: Vec<(usize, usize)> = rt
        .control_active
        .difference(&control_now)
        .copied()
        .collect();
    for key in leaving {
        if let Some(c) = tracks.get(key.0).and_then(|t| t.clips.get(key.1)) {
            exit_control(world, &mut rt, &dir, c, key);
        }
    }
    rt.control_active = control_now;

    // Playable (script) clips.
    if let Some(mut pb) = world.resources.remove::<PlayableBehaviours>() {
        let mut now: HashSet<(usize, usize)> = HashSet::new();
        for (ti, track) in tracks.iter().enumerate() {
            if track.track_type != TrackType::Playable || timeline.is_muted(track) {
                continue;
            }
            let binding = dir.binding(&track.name);
            for a in evaluate_track(track, t) {
                let c = &track.clips[a.index];
                let key = (id, ti, a.index);
                if !pb.instances.contains_key(&key) {
                    let Some(b) = pb.instantiate(&c.behaviour, &c.behaviour_fields) else {
                        continue;
                    };
                    pb.instances.insert(key, b);
                }
                let info = FrameInfo {
                    director: id,
                    binding,
                    time: a.source_time,
                    duration: c.duration,
                    weight: a.weight,
                    playing: mode.playing,
                    delta_time: mode.dt,
                };
                let b = pb.instances.get_mut(&key).expect("just inserted");
                if !rt.playables_active.contains(&(ti, a.index)) {
                    b.on_behaviour_play(world, &info);
                }
                b.process_frame(world, &info);
                now.insert((ti, a.index));
            }
        }
        for &(ti, ci) in rt.playables_active.difference(&now) {
            if let Some(mut b) = pb.instances.remove(&(id, ti, ci)) {
                let info = FrameInfo {
                    director: id,
                    binding: tracks.get(ti).and_then(|tr| dir.binding(&tr.name)),
                    time: 0.0,
                    duration: 0.0,
                    weight: 0.0,
                    playing: mode.playing,
                    delta_time: mode.dt,
                };
                b.on_behaviour_pause(world, &info);
            }
        }
        rt.playables_active = now;
        world.resources.insert(pb);
    }

    // Signals.
    if mode.signals && world.run_scripts {
        let dur = duration as f32;
        let (from, first) = match prev {
            Some(p) => (p as f32, false),
            None => (-1e-4, true),
        };
        let mut fire: Vec<(GameObjectId, String, (usize, usize))> = Vec::new();
        let mut collect = |markers: &[SignalEmitter], receiver: Option<GameObjectId>, ti: usize| {
            let Some(recv) = receiver else { return };
            let mut hit = mix::crossed(markers.iter().map(|m| m.time), from, t, dur);
            if first {
                for (i, m) in markers.iter().enumerate() {
                    if m.retroactive && m.time < t && !hit.contains(&i) {
                        hit.push(i);
                    }
                }
            }
            for i in hit {
                let m = &markers[i];
                if m.emit_once && rt.emitted.contains(&(ti, i)) {
                    continue;
                }
                if let Some(s) = &m.signal {
                    fire.push((recv, s.path().to_owned(), (ti, i)));
                }
            }
        };
        collect(&timeline.markers, Some(id), usize::MAX);
        for (ti, track) in tracks.iter().enumerate() {
            if timeline.is_muted(track) {
                continue;
            }
            collect(&track.markers, dir.binding(&track.name), ti);
        }
        for (recv, signal, key) in fire {
            rt.emitted.insert(key);
            send_signal(world, recv, &signal);
        }
    }

    rt.last = Some(rt.time);
    if let Some(d) = world.component_mut::<PlayableDirector>(id, cid) {
        let commands = std::mem::take(&mut d.rt.commands);
        d.rt = rt;
        d.rt.commands = commands;
    }
}

/// Plays every PlayableDirector (after the Animator, so animation tracks override it).
#[derive(Default)]
pub struct TimelineSystem;

impl TimelineSystem {
    fn run(world: &mut World) {
        let time = world.resource::<Time>();
        let (dt, unscaled) = (time.delta_time(), time.unscaled_delta_time());
        for (id, cid) in world.query_ids::<PlayableDirector>() {
            let enabled = world.is_component_enabled(id, cid)
                && world.get(id).is_some_and(|o| o.active_in_hierarchy());
            let Some(mut d) = world.component::<PlayableDirector>(id, cid).cloned() else {
                continue;
            };
            if !enabled {
                if d.rt.state == PlayState::Playing {
                    release(world, id, cid);
                    if let Some(d) = world.component_mut::<PlayableDirector>(id, cid) {
                        d.rt.state = PlayState::Stopped;
                    }
                }
                continue;
            }
            if d.rt.controlled {
                continue;
            }
            let mut evaluate_now = false;
            if !d.rt.started {
                d.rt.started = true;
                if d.play_on_awake {
                    d.rt.commands.insert(0, DirectorCommand::Play);
                }
            }
            let mut stop = false;
            for c in std::mem::take(&mut d.rt.commands) {
                match c {
                    DirectorCommand::Play => {
                        if d.rt.state == PlayState::Stopped {
                            d.rt.time = f64::from(d.initial_time);
                            d.rt.last = None;
                            d.rt.emitted.clear();
                        }
                        d.rt.state = PlayState::Playing;
                        evaluate_now = true;
                    }
                    DirectorCommand::Pause => {
                        if d.rt.state == PlayState::Playing {
                            d.rt.state = PlayState::Paused;
                        }
                    }
                    DirectorCommand::Resume => {
                        if d.rt.state == PlayState::Paused {
                            d.rt.state = PlayState::Playing;
                        }
                    }
                    DirectorCommand::Stop => stop = true,
                    DirectorCommand::Evaluate => evaluate_now = true,
                }
            }
            let playing = d.rt.state == PlayState::Playing;
            let step = match d.update_method {
                DirectorUpdateMode::GameTime => dt,
                DirectorUpdateMode::UnscaledGameTime => unscaled,
                DirectorUpdateMode::Manual => 0.0,
            };
            let started_now = evaluate_now && d.rt.last.is_none();
            if playing && !started_now {
                d.rt.time += f64::from(step);
            }
            let duration = d.rt.duration;
            let mut finished = false;
            if playing && d.rt.last.is_some() && duration > 0.0 && d.rt.time >= duration {
                match d.wrap_mode {
                    DirectorWrapMode::Hold => d.rt.time = duration,
                    DirectorWrapMode::Loop => d.rt.time = d.rt.time.rem_euclid(duration),
                    DirectorWrapMode::None => {
                        d.rt.time = duration;
                        finished = true;
                    }
                }
            }
            let state = d.rt.state;
            if let Some(c) = world.component_mut::<PlayableDirector>(id, cid) {
                let keep = std::mem::take(&mut c.rt.commands);
                c.rt = d.rt;
                c.rt.commands = keep;
            }
            if playing || evaluate_now {
                evaluate(
                    world,
                    id,
                    cid,
                    EvalMode {
                        playing,
                        dt: if playing { step } else { 0.0 },
                        signals: playing,
                        audio: playing,
                    },
                );
            } else if state == PlayState::Paused {
                // Paused sounds hold their place.
                evaluate(
                    world,
                    id,
                    cid,
                    EvalMode {
                        playing: false,
                        dt: 0.0,
                        signals: false,
                        audio: true,
                    },
                );
            }
            if stop || finished {
                release(world, id, cid);
                if let Some(c) = world.component_mut::<PlayableDirector>(id, cid) {
                    c.rt.state = PlayState::Stopped;
                    c.rt.time = f64::from(c.initial_time);
                    c.rt.last = None;
                }
            }
        }
    }
}

impl System for TimelineSystem {
    fn name(&self) -> &'static str {
        "Timeline"
    }

    fn update(&mut self, world: &mut World) {
        if world.run_scripts {
            Self::run(world);
        }
    }
}

/// Registers the Timeline asset types (`.playable`, `.signal`).
pub fn register_assets(assets: &mut AssetServer) {
    use oxide_assets::AssetTypeInfo;
    assets.register_data_asset::<TimelineAsset>(
        AssetTypeInfo::new("TimelineAsset", &["playable"])
            .menu("Timeline", "New Timeline", 74)
            .color(120, 200, 240),
    );
    assets.register_data_asset::<SignalAsset>(
        AssetTypeInfo::new("SignalAsset", &["signal"])
            .menu("Timeline/Signal", "New Signal", 75)
            .color(240, 200, 100),
    );
}

/// Installs the Director and Signal Receiver components; returns the system.
pub fn install(world: &mut World) -> Box<dyn System> {
    world
        .registry
        .register::<PlayableDirector>("Miscellaneous/Playable Director");
    world
        .registry
        .register::<SignalReceiver>("Miscellaneous/Signal Receiver");
    if !world.resources.contains::<PlayableBehaviours>() {
        world.resources.insert(PlayableBehaviours::default());
    }
    Box::new(TimelineSystem)
}

#[cfg(test)]
mod tests;
