//! The Timeline asset (`.playable`, Unity `TimelineAsset`): tracks of clips and markers.
//!
//! Tracks name what they drive; the scene objects they drive are bound on the
//! [`PlayableDirector`](crate::PlayableDirector) (by track name), so one Timeline can be
//! reused on different objects, as in Unity.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.timeline@1.8/manual/index.html>

use oxide_assets::{AnimationClip, Asset, AudioClip, AvatarMask, Handle};
use oxide_core::Reflect;
use oxide_math::Vector3;

/// Unity's track types (Add menu of the Timeline window).
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackType {
    /// Plays Animation Clips on a bound Animator or GameObject.
    #[default]
    Animation,
    /// Sets the bound GameObject active while a clip is under the playhead.
    Activation,
    /// Plays Audio Clips (through the bound AudioSource, if any).
    Audio,
    /// Holds Signal Emitters (markers) sent to the bound object's Signal Receiver.
    Signal,
    /// Activates objects and drives nested Timelines and particle systems.
    Control,
    /// Custom clips: a script's `PlayableBehaviour` (Unity `PlayableTrack`).
    Playable,
    /// A folder of tracks.
    Group,
}

impl TrackType {
    pub const ALL: [TrackType; 7] = [
        TrackType::Activation,
        TrackType::Animation,
        TrackType::Audio,
        TrackType::Control,
        TrackType::Playable,
        TrackType::Signal,
        TrackType::Group,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TrackType::Animation => "Animation Track",
            TrackType::Activation => "Activation Track",
            TrackType::Audio => "Audio Track",
            TrackType::Signal => "Signal Track",
            TrackType::Control => "Control Track",
            TrackType::Playable => "Playable Track",
            TrackType::Group => "Track Group",
        }
    }

    /// Whether objects are bound to tracks of this type.
    pub fn has_binding(self) -> bool {
        !matches!(self, TrackType::Control | TrackType::Group)
    }

    /// Whether the track holds clips (Signal and Group tracks do not).
    pub fn has_clips(self) -> bool {
        !matches!(self, TrackType::Signal | TrackType::Group)
    }
}

/// Unity `TimelineClip.ClipExtrapolation`: what a clip does before it starts or after it
/// ends, up to the neighbouring clip.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClipExtrapolation {
    #[default]
    None,
    /// Holds the first or last frame.
    Hold,
    Loop,
    PingPong,
    /// Keeps the clip's time running (the clip's own looping decides what plays).
    Continue,
}

/// Unity `ActivationTrack.PostPlaybackState`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostPlaybackState {
    Active,
    Inactive,
    /// Back to how it was before the Timeline played.
    Revert,
    #[default]
    LeaveAsIs,
}

/// Unity `AnimationPlayableAsset.LoopMode`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopMode {
    #[default]
    UseSourceAsset,
    On,
    Off,
}

/// Unity `TimelineAsset.DurationMode`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DurationMode {
    #[default]
    BasedOnClips,
    FixedLength,
}

/// One saved field of a custom clip's behaviour (a JSON value).
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct BehaviourField {
    pub name: String,
    pub json: String,
}

/// Unity `TimelineClip` and its playable asset's settings (one struct for every clip type;
/// fields of other types are ignored).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct TimelineClip {
    pub display_name: String,
    /// Seconds on the Timeline.
    pub start: f32,
    pub duration: f32,
    /// Seconds into the source the clip starts at (trimming its start).
    pub clip_in: f32,
    /// Unity "Speed Multiplier".
    pub time_scale: f32,
    /// Fade-in and fade-out lengths (seconds); overlapping clips blend over the overlap.
    pub ease_in_duration: f32,
    pub ease_out_duration: f32,
    pub pre_extrapolation: ClipExtrapolation,
    pub post_extrapolation: ClipExtrapolation,

    // Animation
    pub animation_clip: Option<Handle<AnimationClip>>,
    pub loop_mode: LoopMode,
    /// Moves and turns the clip's root motion (Unity "Clip Transform Offsets").
    pub position_offset: Vector3,
    /// Euler degrees.
    pub rotation_offset: Vector3,

    // Audio
    pub audio_clip: Option<Handle<AudioClip>>,
    #[reflect(range(0, 1))]
    pub volume: f32,
    pub looping: bool,

    // Control
    /// Exposed reference name: the Director's Exposed References map it to an object.
    pub source_game_object: String,
    /// A prefab instantiated under the source object while the clip plays.
    pub prefab: String,
    pub control_activation: bool,
    pub update_directors: bool,
    pub update_particles: bool,
    pub post_playback: PostPlaybackState,

    // Playable
    /// A registered `PlayableBehaviour` type name.
    pub behaviour: String,
    pub behaviour_fields: Vec<BehaviourField>,
}

impl Default for TimelineClip {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            start: 0.0,
            duration: 1.0,
            clip_in: 0.0,
            time_scale: 1.0,
            ease_in_duration: 0.0,
            ease_out_duration: 0.0,
            pre_extrapolation: ClipExtrapolation::None,
            post_extrapolation: ClipExtrapolation::None,
            animation_clip: None,
            loop_mode: LoopMode::UseSourceAsset,
            position_offset: Vector3::ZERO,
            rotation_offset: Vector3::ZERO,
            audio_clip: None,
            volume: 1.0,
            looping: false,
            source_game_object: String::new(),
            prefab: String::new(),
            control_activation: true,
            update_directors: true,
            update_particles: true,
            post_playback: PostPlaybackState::Revert,
            behaviour: String::new(),
            behaviour_fields: Vec::new(),
        }
    }
}

impl TimelineClip {
    pub fn end(&self) -> f32 {
        self.start + self.duration
    }
}

/// Unity `SignalEmitter`: a marker that sends a Signal when the playhead passes it.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct SignalEmitter {
    pub time: f32,
    pub signal: Option<Handle<SignalAsset>>,
    /// Fires when playback starts past it (Unity "Retroactive").
    pub retroactive: bool,
    /// Fires only the first time (not again after looping).
    pub emit_once: bool,
}

impl Default for SignalEmitter {
    fn default() -> Self {
        Self {
            time: 0.0,
            signal: None,
            retroactive: false,
            emit_once: false,
        }
    }
}

/// Unity `TrackAsset`.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct TrackAsset {
    /// Unique within the Timeline: the Director binds objects by this name.
    pub name: String,
    pub track_type: TrackType,
    /// Name of the Track Group this track sits in (empty = top level).
    pub group: String,
    pub muted: bool,
    pub locked: bool,
    /// Editor: a collapsed Track Group hides its tracks.
    pub collapsed: bool,
    pub clips: Vec<TimelineClip>,
    pub markers: Vec<SignalEmitter>,

    // Animation
    /// Unity "Track Offsets": moves and turns everything the track animates.
    pub apply_offsets: bool,
    pub position: Vector3,
    /// Euler degrees.
    pub rotation: Vector3,
    pub avatar_mask: Option<Handle<AvatarMask>>,

    // Activation
    pub post_playback_state: PostPlaybackState,

    // Audio
    #[reflect(range(0, 1))]
    pub volume: f32,
}

impl Default for TrackAsset {
    fn default() -> Self {
        Self {
            name: String::new(),
            track_type: TrackType::Animation,
            group: String::new(),
            muted: false,
            locked: false,
            collapsed: false,
            clips: Vec::new(),
            markers: Vec::new(),
            apply_offsets: false,
            position: Vector3::ZERO,
            rotation: Vector3::ZERO,
            avatar_mask: None,
            post_playback_state: PostPlaybackState::LeaveAsIs,
            volume: 1.0,
        }
    }
}

impl TrackAsset {
    pub fn new(name: &str, track_type: TrackType) -> Self {
        Self {
            name: name.to_owned(),
            track_type,
            ..Default::default()
        }
    }

    /// The end of the last clip or marker.
    pub fn end(&self) -> f32 {
        let clips = self.clips.iter().map(TimelineClip::end);
        let markers = self.markers.iter().map(|m| m.time);
        clips.chain(markers).fold(0.0, f32::max)
    }
}

/// Unity `TimelineAsset` (`.playable`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct TimelineAsset {
    /// Frames per second the editor snaps to.
    pub frame_rate: f32,
    pub duration_mode: DurationMode,
    pub fixed_duration: f32,
    pub tracks: Vec<TrackAsset>,
    /// The Markers row at the top of the Timeline (sent to the Director's object).
    pub markers: Vec<SignalEmitter>,
}

impl Default for TimelineAsset {
    fn default() -> Self {
        Self {
            frame_rate: 60.0,
            duration_mode: DurationMode::BasedOnClips,
            fixed_duration: 10.0,
            tracks: Vec::new(),
            markers: Vec::new(),
        }
    }
}

impl Asset for TimelineAsset {
    const TYPE_NAME: &'static str = "TimelineAsset";
}

impl TimelineAsset {
    /// Unity `TimelineAsset.duration`.
    pub fn duration(&self) -> f32 {
        match self.duration_mode {
            DurationMode::FixedLength => self.fixed_duration.max(0.0),
            DurationMode::BasedOnClips => {
                let markers = self.markers.iter().map(|m| m.time);
                self.tracks
                    .iter()
                    .map(TrackAsset::end)
                    .chain(markers)
                    .fold(0.0, f32::max)
            }
        }
    }

    pub fn track(&self, name: &str) -> Option<&TrackAsset> {
        self.tracks.iter().find(|t| t.name == name)
    }

    pub fn track_mut(&mut self, name: &str) -> Option<&mut TrackAsset> {
        self.tracks.iter_mut().find(|t| t.name == name)
    }

    /// A track name not used yet (`Animation Track`, `Animation Track (1)`, ...).
    pub fn unique_track_name(&self, base: &str) -> String {
        if self.track(base).is_none() {
            return base.to_owned();
        }
        (1..)
            .map(|i| format!("{base} ({i})"))
            .find(|n| self.track(n).is_none())
            .unwrap_or_default()
    }

    /// Whether `track` is muted itself or sits in a muted group.
    pub fn is_muted(&self, track: &TrackAsset) -> bool {
        track.muted
            || (!track.group.is_empty()
                && self
                    .track(&track.group)
                    .is_some_and(|g| g.track_type == TrackType::Group && self.is_muted(g)))
    }

    /// Whether `track` is hidden by a collapsed group above it.
    pub fn is_hidden(&self, track: &TrackAsset) -> bool {
        let mut g = track.group.as_str();
        let mut depth = 0;
        while !g.is_empty() && depth < 16 {
            let Some(p) = self.track(g) else { break };
            if p.collapsed {
                return true;
            }
            g = p.group.as_str();
            depth += 1;
        }
        false
    }

    /// Nesting depth of `track` (0 = top level).
    pub fn depth(&self, track: &TrackAsset) -> usize {
        let mut g = track.group.as_str();
        let mut depth = 0;
        while !g.is_empty() && depth < 16 {
            let Some(p) = self.track(g) else { break };
            g = p.group.as_str();
            depth += 1;
        }
        depth
    }

    /// Tracks in display order: each group followed by its tracks, recursively.
    pub fn display_order(&self) -> Vec<usize> {
        fn walk(a: &TimelineAsset, group: &str, out: &mut Vec<usize>, depth: usize) {
            for (i, t) in a.tracks.iter().enumerate() {
                if t.group == group && !out.contains(&i) {
                    out.push(i);
                    if t.track_type == TrackType::Group && depth < 16 {
                        walk(a, &t.name, out, depth + 1);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(self, "", &mut out, 0);
        // Tracks whose group is missing still show (at the end).
        for i in 0..self.tracks.len() {
            if !out.contains(&i) {
                out.push(i);
            }
        }
        out
    }
}

/// Unity `SignalAsset` (`.signal`): an identity that emitters send and receivers react to.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct SignalAsset {
    /// Notes for the team (Unity has none; shown in the Inspector).
    pub description: String,
}

impl Asset for SignalAsset {
    const TYPE_NAME: &'static str = "SignalAsset";
}
