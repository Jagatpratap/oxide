//! Unity Animator Controllers (`.controller`): parameters, layers, state machines with
//! sub-state machines, blend trees, transitions and state machine behaviours; plus Avatar
//! Masks (`.mask`) and Animator Override Controllers (`.overrideController`).
//! Reference: <https://docs.unity3d.com/Manual/class-AnimatorController.html>

use crate::animation::AnimationClip;
use crate::handle::{Asset, Handle};
use oxide_core::reflect::Value;
use oxide_math::Vector2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Unity `AnimatorControllerParameterType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParameterType {
    Float,
    Int,
    Bool,
    Trigger,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub kind: ParameterType,
    pub default: f32,
}

/// Unity `AnimatorConditionMode`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConditionMode {
    /// Bool true / trigger set.
    If,
    IfNot,
    Greater(f32),
    Less(f32),
    Equals(f32),
    NotEqual(f32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub parameter: String,
    pub mode: ConditionMode,
}

impl Condition {
    pub fn met(&self, params: &std::collections::HashMap<String, f32>) -> bool {
        let v = params.get(&self.parameter).copied().unwrap_or(0.0);
        match self.mode {
            ConditionMode::If => v != 0.0,
            ConditionMode::IfNot => v == 0.0,
            ConditionMode::Greater(x) => v > x,
            ConditionMode::Less(x) => v < x,
            ConditionMode::Equals(x) => (v - x).abs() < 1e-4,
            ConditionMode::NotEqual(x) => (v - x).abs() >= 1e-4,
        }
    }
}

/// The 2D blend types of the old blend tree format (kept so old files load).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlendTree2DType {
    SimpleDirectional,
    FreeformDirectional,
    #[default]
    FreeformCartesian,
}

/// Unity `BlendTreeType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlendTreeType {
    /// One parameter; children at thresholds.
    #[default]
    Simple1D,
    /// Two parameters; one motion per direction (strafe sets). Gradient-band interpolation
    /// in polar space, so speed and direction blend separately.
    SimpleDirectional2D,
    /// Two parameters; several motions per direction (walk and run forward, ...).
    FreeformDirectional2D,
    /// Two parameters; motions that are not directions (lean, aim offsets).
    FreeformCartesian2D,
    /// Each child's weight is its own parameter (facial expressions, additive layers).
    Direct,
}

impl BlendTreeType {
    pub const ALL: [Self; 5] = [
        Self::Simple1D,
        Self::SimpleDirectional2D,
        Self::FreeformDirectional2D,
        Self::FreeformCartesian2D,
        Self::Direct,
    ];

    pub fn is_2d(self) -> bool {
        matches!(
            self,
            Self::SimpleDirectional2D | Self::FreeformDirectional2D | Self::FreeformCartesian2D
        )
    }

    /// Unity's Blend Type menu text.
    pub fn label(self) -> &'static str {
        match self {
            Self::Simple1D => "1D",
            Self::SimpleDirectional2D => "2D Simple Directional",
            Self::FreeformDirectional2D => "2D Freeform Directional",
            Self::FreeformCartesian2D => "2D Freeform Cartesian",
            Self::Direct => "Direct",
        }
    }

    /// The matching old 2D type, for the interpolation code.
    pub fn as_2d(self) -> BlendTree2DType {
        match self {
            Self::SimpleDirectional2D => BlendTree2DType::SimpleDirectional,
            Self::FreeformDirectional2D => BlendTree2DType::FreeformDirectional,
            _ => BlendTree2DType::FreeformCartesian,
        }
    }
}

/// Unity `ChildMotion`: one entry of a blend tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChildMotion {
    pub motion: Motion,
    /// 1D trees.
    pub threshold: f32,
    /// 2D trees.
    pub position: Vector2,
    /// Playback speed of this child relative to the tree.
    pub time_scale: f32,
    /// Normalized time offset.
    pub cycle_offset: f32,
    /// Plays the motion mirrored left/right.
    pub mirror: bool,
    /// Direct trees: the parameter giving this child's weight.
    pub direct_blend_parameter: String,
}

impl Default for ChildMotion {
    fn default() -> Self {
        Self {
            motion: Motion::Empty,
            threshold: 0.0,
            position: Vector2::ZERO,
            time_scale: 1.0,
            cycle_offset: 0.0,
            mirror: false,
            direct_blend_parameter: String::new(),
        }
    }
}

/// Unity `BlendTree`: blends child motions (clips or nested trees) by parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlendTree {
    pub name: String,
    pub blend_type: BlendTreeType,
    pub blend_parameter: String,
    pub blend_parameter_y: String,
    pub children: Vec<ChildMotion>,
    /// 1D: space thresholds evenly between `min_threshold` and `max_threshold`.
    pub use_automatic_thresholds: bool,
    pub min_threshold: f32,
    pub max_threshold: f32,
    /// Direct: divide the weights by their sum.
    pub normalized_blend_values: bool,
}

impl Default for BlendTree {
    fn default() -> Self {
        Self {
            name: "Blend Tree".into(),
            blend_type: BlendTreeType::Simple1D,
            blend_parameter: String::new(),
            blend_parameter_y: String::new(),
            children: vec![],
            use_automatic_thresholds: true,
            min_threshold: 0.0,
            max_threshold: 1.0,
            normalized_blend_values: false,
        }
    }
}

impl BlendTree {
    /// The thresholds 1D blending uses (automatic ones spread evenly, as Unity shows them).
    pub fn effective_thresholds(&self) -> Vec<f32> {
        let n = self.children.len();
        if !self.use_automatic_thresholds || n == 0 {
            return self.children.iter().map(|c| c.threshold).collect();
        }
        if n == 1 {
            return vec![self.min_threshold];
        }
        (0..n)
            .map(|i| {
                self.min_threshold
                    + (self.max_threshold - self.min_threshold) * i as f32 / (n - 1) as f32
            })
            .collect()
    }

    /// Writes automatic thresholds into the children (when switching them off, Unity keeps
    /// the values they had).
    pub fn bake_thresholds(&mut self) {
        let t = self.effective_thresholds();
        for (c, t) in self.children.iter_mut().zip(t) {
            c.threshold = t;
        }
    }

    /// Unity's Add Motion Field / New Blend Tree: appends a child at a sensible spot.
    pub fn add_child(&mut self, motion: Motion) -> &mut ChildMotion {
        let n = self.children.len();
        let threshold = self.children.last().map_or(0.0, |c| c.threshold + 1.0);
        // 2D: first at the centre, then around a unit circle.
        let position = if n == 0 {
            Vector2::ZERO
        } else {
            let a = (n - 1) as f32 * std::f32::consts::FRAC_PI_4;
            Vector2::new(a.sin(), a.cos())
        };
        self.children.push(ChildMotion {
            motion,
            threshold,
            position,
            ..Default::default()
        });
        if self.use_automatic_thresholds {
            self.max_threshold = self.max_threshold.max(self.min_threshold + 1.0);
        }
        self.children.last_mut().expect("pushed")
    }
}

/// What a state (or blend tree child) plays.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Motion {
    Clip(Handle<AnimationClip>),
    BlendTree(Box<BlendTree>),
    /// Old 1D format (converted to `BlendTree` on load).
    BlendTree1D {
        parameter: String,
        children: Vec<(f32, Handle<AnimationClip>)>,
    },
    /// Old 2D format (converted to `BlendTree` on load).
    BlendTree2D {
        parameter_x: String,
        parameter_y: String,
        kind: BlendTree2DType,
        children: Vec<(Vector2, Handle<AnimationClip>)>,
    },
    /// No motion (holds the current pose).
    #[default]
    Empty,
}

impl Motion {
    pub fn tree(t: BlendTree) -> Self {
        Self::BlendTree(Box::new(t))
    }

    /// Converts the old blend tree formats to [`BlendTree`] (recursively).
    pub fn upgrade(&mut self) {
        let new = match std::mem::take(self) {
            Motion::BlendTree1D {
                parameter,
                children,
            } => Motion::tree(BlendTree {
                blend_parameter: parameter,
                use_automatic_thresholds: false,
                children: children
                    .into_iter()
                    .map(|(t, h)| ChildMotion {
                        motion: Motion::Clip(h),
                        threshold: t,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }),
            Motion::BlendTree2D {
                parameter_x,
                parameter_y,
                kind,
                children,
            } => Motion::tree(BlendTree {
                blend_type: match kind {
                    BlendTree2DType::SimpleDirectional => BlendTreeType::SimpleDirectional2D,
                    BlendTree2DType::FreeformDirectional => BlendTreeType::FreeformDirectional2D,
                    BlendTree2DType::FreeformCartesian => BlendTreeType::FreeformCartesian2D,
                },
                blend_parameter: parameter_x,
                blend_parameter_y: parameter_y,
                use_automatic_thresholds: false,
                children: children
                    .into_iter()
                    .map(|(p, h)| ChildMotion {
                        motion: Motion::Clip(h),
                        position: p,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }),
            Motion::BlendTree(mut t) => {
                for c in &mut t.children {
                    c.motion.upgrade();
                }
                Motion::BlendTree(t)
            }
            m => m,
        };
        *self = new;
    }

    pub fn as_tree(&self) -> Option<&BlendTree> {
        match self {
            Motion::BlendTree(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_tree_mut(&mut self) -> Option<&mut BlendTree> {
        match self {
            Motion::BlendTree(t) => Some(t),
            _ => None,
        }
    }

    /// The nested motion at `path` (child indices from this tree down).
    pub fn at(&self, path: &[usize]) -> Option<&Motion> {
        match path.split_first() {
            None => Some(self),
            Some((i, rest)) => self.as_tree()?.children.get(*i)?.motion.at(rest),
        }
    }

    pub fn at_mut(&mut self, path: &[usize]) -> Option<&mut Motion> {
        match path.split_first() {
            None => Some(self),
            Some((i, rest)) => self
                .as_tree_mut()?
                .children
                .get_mut(*i)?
                .motion
                .at_mut(rest),
        }
    }

    /// Every clip, depth first.
    pub fn for_each_clip(&self, f: &mut dyn FnMut(&Handle<AnimationClip>)) {
        match self {
            Motion::Clip(h) => f(h),
            Motion::BlendTree(t) => t.children.iter().for_each(|c| c.motion.for_each_clip(f)),
            Motion::BlendTree1D { children, .. } => children.iter().for_each(|(_, h)| f(h)),
            Motion::BlendTree2D { children, .. } => children.iter().for_each(|(_, h)| f(h)),
            Motion::Empty => {}
        }
    }

    pub fn for_each_clip_mut(&mut self, f: &mut dyn FnMut(&mut Handle<AnimationClip>)) {
        match self {
            Motion::Clip(h) => f(h),
            Motion::BlendTree(t) => t
                .children
                .iter_mut()
                .for_each(|c| c.motion.for_each_clip_mut(f)),
            Motion::BlendTree1D { children, .. } => children.iter_mut().for_each(|(_, h)| f(h)),
            Motion::BlendTree2D { children, .. } => children.iter_mut().for_each(|(_, h)| f(h)),
            Motion::Empty => {}
        }
    }

    /// Display name: the clip's file name, the tree's name or "None".
    pub fn label(&self) -> String {
        match self {
            Motion::Clip(h) => {
                let p = h.path();
                let p = p.rsplit('/').next().unwrap_or(p);
                p.trim_end_matches(".anim").to_owned()
            }
            Motion::BlendTree(t) => t.name.clone(),
            Motion::BlendTree1D { .. } | Motion::BlendTree2D { .. } => "Blend Tree".into(),
            Motion::Empty => "None".into(),
        }
    }
}

/// Unity `StateMachineBehaviour` attached to a state or state machine: a registered script
/// type and its saved field values. Its callbacks (`OnStateEnter`, `OnStateUpdate`,
/// `OnStateExit`, `OnStateMove`, `OnStateIK`, `OnStateMachineEnter/Exit`) run while the
/// Animator is in that state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StateBehaviour {
    pub type_name: String,
    pub fields: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimatorState {
    pub name: String,
    /// Where the Animator window draws the state.
    pub position: Vector2,
    /// Unity state tag (`AnimatorStateInfo.IsTag`).
    pub tag: String,
    pub motion: Motion,
    pub speed: f32,
    /// Multiplies `speed` by a float parameter.
    pub speed_parameter_active: bool,
    pub speed_parameter: String,
    /// Drives normalized time from a float parameter instead of playing (aim poses, scrubbing).
    pub time_parameter_active: bool,
    pub time_parameter: String,
    pub mirror: bool,
    pub mirror_parameter_active: bool,
    pub mirror_parameter: String,
    /// Normalized start offset.
    pub cycle_offset: f32,
    pub cycle_offset_parameter_active: bool,
    pub cycle_offset_parameter: String,
    /// Humanoid foot IK (keeps the feet on the clip's foot positions).
    pub foot_ik: bool,
    /// Properties this state does not animate return to their default values.
    pub write_defaults: bool,
    pub looping: bool,
    pub behaviours: Vec<StateBehaviour>,
}

impl Default for AnimatorState {
    fn default() -> Self {
        Self {
            name: "New State".into(),
            position: Vector2::ZERO,
            tag: String::new(),
            motion: Motion::Empty,
            speed: 1.0,
            speed_parameter_active: false,
            speed_parameter: String::new(),
            time_parameter_active: false,
            time_parameter: String::new(),
            mirror: false,
            mirror_parameter_active: false,
            mirror_parameter: String::new(),
            cycle_offset: 0.0,
            cycle_offset_parameter_active: false,
            cycle_offset_parameter: String::new(),
            foot_ik: false,
            write_defaults: true,
            looping: true,
            behaviours: vec![],
        }
    }
}

/// What a transition starts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionSource {
    /// A state (`from` = its name), or Any State when `from` is `None`.
    #[default]
    State,
    /// The Entry node of the machine holding the transition (Unity entry transitions).
    Entry,
    /// A sub-state machine (`from` = its name): taken when it exits through its Exit node.
    StateMachine,
}

/// Where a transition goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionTarget {
    #[default]
    State,
    /// A sub-state machine: enters it through its Entry node.
    StateMachine,
    /// The Exit node: leaves the machine holding the transition.
    Exit,
}

/// Unity `TransitionInterruptionSource`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptionSource {
    #[default]
    None,
    Source,
    Destination,
    SourceThenDestination,
    DestinationThenSource,
}

impl InterruptionSource {
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::Source,
        Self::Destination,
        Self::SourceThenDestination,
        Self::DestinationThenSource,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Source => "Current State",
            Self::Destination => "Next State",
            Self::SourceThenDestination => "Current State Then Next State",
            Self::DestinationThenSource => "Next State Then Current State",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    /// Source name; `None` = Any State (or the Entry node, see `source`).
    pub from: Option<String>,
    pub source: TransitionSource,
    /// Destination name (empty for Exit).
    pub to: String,
    pub target: TransitionTarget,
    pub conditions: Vec<Condition>,
    /// Crossfade duration: seconds when `fixed_duration`, else a fraction of the source state.
    pub duration: f32,
    pub fixed_duration: bool,
    /// Only transition after `exit_time` (normalized) of the source state.
    pub has_exit_time: bool,
    pub exit_time: f32,
    /// Normalized time the destination starts at.
    pub offset: f32,
    pub interruption_source: InterruptionSource,
    /// Only transitions listed before this one may interrupt it.
    pub ordered_interruption: bool,
    /// Any State transitions: allow going to the state already playing.
    pub can_transition_to_self: bool,
    pub mute: bool,
    pub solo: bool,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            from: None,
            source: TransitionSource::State,
            to: String::new(),
            target: TransitionTarget::State,
            conditions: vec![],
            duration: 0.25,
            fixed_duration: true,
            has_exit_time: false,
            exit_time: 0.75,
            offset: 0.0,
            interruption_source: InterruptionSource::None,
            ordered_interruption: true,
            can_transition_to_self: true,
            mute: false,
            solo: false,
        }
    }
}

impl Transition {
    pub fn is_any_state(&self) -> bool {
        self.from.is_none() && self.source == TransitionSource::State
    }

    pub fn is_entry(&self) -> bool {
        self.source == TransitionSource::Entry
    }
}

fn v2(x: f32, y: f32) -> Vector2 {
    Vector2::new(x, y)
}

/// Unity `AnimatorStateMachine`: states, sub-state machines and transitions of one layer
/// (or of a sub-state machine inside it). State and sub-machine names are unique within a
/// layer, so transitions refer to them by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimatorStateMachine {
    /// Sub-state machines: the name shown on its node.
    pub name: String,
    /// Sub-state machines: where the parent graph draws its node.
    pub position: Vector2,
    pub states: Vec<AnimatorState>,
    pub state_machines: Vec<AnimatorStateMachine>,
    /// Index into `states` (a machine without states enters its first sub-machine).
    pub default_state: usize,
    pub transitions: Vec<Transition>,
    pub entry_position: Vector2,
    pub any_state_position: Vector2,
    pub exit_position: Vector2,
    pub parent_position: Vector2,
    /// Behaviours that get `OnStateMachineEnter/Exit` (and the state callbacks of every
    /// state inside).
    pub behaviours: Vec<StateBehaviour>,
}

impl Default for AnimatorStateMachine {
    fn default() -> Self {
        Self {
            name: String::new(),
            position: Vector2::ZERO,
            states: vec![],
            state_machines: vec![],
            default_state: 0,
            transitions: vec![],
            entry_position: v2(50.0, 120.0),
            any_state_position: v2(50.0, 40.0),
            exit_position: v2(700.0, 120.0),
            parent_position: v2(700.0, 40.0),
            behaviours: vec![],
        }
    }
}

/// A state's place in a layer: the sub-machine path (child indices from the layer's root
/// machine) and its index in that machine.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct StatePath {
    pub machine: Vec<usize>,
    pub index: usize,
}

impl AnimatorStateMachine {
    /// Index of a state of this machine (not nested ones).
    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name == name)
    }

    pub fn machine(&self, path: &[usize]) -> Option<&Self> {
        match path.split_first() {
            None => Some(self),
            Some((i, rest)) => self.state_machines.get(*i)?.machine(rest),
        }
    }

    pub fn machine_mut(&mut self, path: &[usize]) -> Option<&mut Self> {
        match path.split_first() {
            None => Some(self),
            Some((i, rest)) => self.state_machines.get_mut(*i)?.machine_mut(rest),
        }
    }

    /// Every state, depth first (own states, then each sub-machine's): the layer's flat
    /// state order the runtime uses.
    pub fn flatten(&self) -> Vec<(StatePath, &AnimatorState)> {
        let mut out = vec![];
        self.flatten_into(&mut vec![], &mut out);
        out
    }

    fn flatten_into<'a>(
        &'a self,
        path: &mut Vec<usize>,
        out: &mut Vec<(StatePath, &'a AnimatorState)>,
    ) {
        for (i, s) in self.states.iter().enumerate() {
            out.push((
                StatePath {
                    machine: path.clone(),
                    index: i,
                },
                s,
            ));
        }
        for (i, m) in self.state_machines.iter().enumerate() {
            path.push(i);
            m.flatten_into(path, out);
            path.pop();
        }
    }

    /// Finds a state anywhere in this machine. Accepts `Name` or a dotted Unity path
    /// (`Base Layer.Locomotion.Run`): the last part is the name.
    pub fn find_state(&self, name: &str) -> Option<StatePath> {
        let name = name.rsplit('.').next().unwrap_or(name);
        self.flatten()
            .into_iter()
            .find(|(_, s)| s.name == name)
            .map(|(p, _)| p)
    }

    pub fn state_at(&self, p: &StatePath) -> Option<&AnimatorState> {
        self.machine(&p.machine)?.states.get(p.index)
    }

    pub fn state_at_mut(&mut self, p: &StatePath) -> Option<&mut AnimatorState> {
        self.machine_mut(&p.machine)?.states.get_mut(p.index)
    }

    /// Path of a sub-machine by name, searching the whole tree.
    pub fn find_machine(&self, name: &str) -> Option<Vec<usize>> {
        for (i, m) in self.state_machines.iter().enumerate() {
            if m.name == name {
                return Some(vec![i]);
            }
            if let Some(mut p) = m.find_machine(name) {
                p.insert(0, i);
                return Some(p);
            }
        }
        None
    }

    /// Every state and sub-machine name (for keeping new names unique).
    pub fn all_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.states.iter().map(|s| s.name.clone()).collect();
        for m in &self.state_machines {
            out.push(m.name.clone());
            out.extend(m.all_names());
        }
        out
    }

    /// Every transition in the tree, with the path of the machine holding it.
    pub fn all_transitions(&self) -> Vec<(Vec<usize>, &Transition)> {
        let mut out = vec![];
        fn walk<'a>(
            m: &'a AnimatorStateMachine,
            path: &mut Vec<usize>,
            out: &mut Vec<(Vec<usize>, &'a Transition)>,
        ) {
            out.extend(m.transitions.iter().map(|t| (path.clone(), t)));
            for (i, c) in m.state_machines.iter().enumerate() {
                path.push(i);
                walk(c, path, out);
                path.pop();
            }
        }
        walk(self, &mut vec![], &mut out);
        out
    }

    /// Renames a state or sub-machine everywhere transitions mention it.
    pub fn rename_references(&mut self, old: &str, new: &str) {
        for t in &mut self.transitions {
            if t.to == old {
                t.to = new.to_owned();
            }
            if t.from.as_deref() == Some(old) {
                t.from = Some(new.to_owned());
            }
        }
        for m in &mut self.state_machines {
            m.rename_references(old, new);
        }
    }

    /// Removes every transition into or out of `name`.
    pub fn remove_references(&mut self, name: &str) {
        self.transitions.retain(|t| {
            !(t.to == name && t.target != TransitionTarget::Exit) && t.from.as_deref() != Some(name)
        });
        for m in &mut self.state_machines {
            m.remove_references(name);
        }
    }

    /// Unity's Layer Default State: the state the machine enters with no entry transition.
    pub fn default_state_path(&self) -> Option<StatePath> {
        if !self.states.is_empty() {
            return Some(StatePath {
                machine: vec![],
                index: self.default_state.min(self.states.len() - 1),
            });
        }
        self.state_machines.iter().enumerate().find_map(|(i, m)| {
            let mut p = m.default_state_path()?;
            p.machine.insert(0, i);
            Some(p)
        })
    }

    pub fn add_state(&mut self, name: &str, motion: Motion) -> &mut AnimatorState {
        let n = self.states.len() as f32;
        self.states.push(AnimatorState {
            name: name.into(),
            // Laid out in a column until the Animator window moves them.
            position: Vector2::new(260.0, 60.0 + 70.0 * n),
            motion,
            ..Default::default()
        });
        self.states.last_mut().expect("pushed")
    }

    pub fn add_state_machine(&mut self, name: &str) -> &mut AnimatorStateMachine {
        let n = self.state_machines.len() as f32;
        self.state_machines.push(AnimatorStateMachine {
            name: name.into(),
            position: Vector2::new(480.0, 60.0 + 70.0 * n),
            ..Default::default()
        });
        self.state_machines.last_mut().expect("pushed")
    }

    pub fn add_transition(
        &mut self,
        from: Option<&str>,
        to: &str,
        conditions: Vec<Condition>,
        duration: f32,
    ) -> &mut Transition {
        self.transitions.push(Transition {
            from: from.map(str::to_owned),
            to: to.into(),
            conditions,
            duration,
            ..Default::default()
        });
        self.transitions.last_mut().expect("pushed")
    }

    /// Converts old blend trees in every state (recursively).
    pub fn upgrade(&mut self) {
        for s in &mut self.states {
            s.motion.upgrade();
        }
        for m in &mut self.state_machines {
            m.upgrade();
        }
    }
}

/// Unity `AvatarMaskBodyPart` (humanoid part of an Avatar Mask).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AvatarMaskBodyPart {
    Root,
    Body,
    Head,
    LeftLeg,
    RightLeg,
    LeftArm,
    RightArm,
    LeftFingers,
    RightFingers,
    LeftFootIK,
    RightFootIK,
    LeftHandIK,
    RightHandIK,
}

impl AvatarMaskBodyPart {
    pub const ALL: [Self; 13] = [
        Self::Root,
        Self::Body,
        Self::Head,
        Self::LeftLeg,
        Self::RightLeg,
        Self::LeftArm,
        Self::RightArm,
        Self::LeftFingers,
        Self::RightFingers,
        Self::LeftFootIK,
        Self::RightFootIK,
        Self::LeftHandIK,
        Self::RightHandIK,
    ];
}

/// Unity `AvatarMask`: which body parts (humanoid rigs) and transforms a layer animates.
/// A transform is included when its path equals, or sits below, one of `transform_paths`;
/// an empty list includes everything. Humanoid parts in `disabled_body_parts` are left out.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AvatarMask {
    pub name: String,
    pub transform_paths: Vec<String>,
    pub disabled_body_parts: Vec<AvatarMaskBodyPart>,
}

impl Asset for AvatarMask {
    const TYPE_NAME: &'static str = "AvatarMask";
}

impl AvatarMask {
    pub fn includes(&self, path: &str) -> bool {
        self.transform_paths.is_empty()
            || self.transform_paths.iter().any(|m| {
                path == m
                    || path.ends_with(&format!("/{m}"))
                    || path.starts_with(&format!("{m}/"))
                    || path.contains(&format!("/{m}/"))
            })
    }

    pub fn body_part_active(&self, part: AvatarMaskBodyPart) -> bool {
        !self.disabled_body_parts.contains(&part)
    }

    pub fn set_body_part_active(&mut self, part: AvatarMaskBodyPart, on: bool) {
        self.disabled_body_parts.retain(|p| *p != part);
        if !on {
            self.disabled_body_parts.push(part);
        }
    }
}

/// Unity `AnimatorLayerBlendingMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AnimatorLayerBlendingMode {
    /// Replaces the layers above it (by `weight`).
    #[default]
    Override,
    /// Adds the motion relative to each clip's first frame.
    Additive,
}

/// A synced layer's motion for one state of its source layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncedMotion {
    pub state: String,
    pub motion: Motion,
}

/// Unity `AnimatorControllerLayer`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimatorControllerLayer {
    pub name: String,
    pub state_machine: AnimatorStateMachine,
    /// Starting weight (the base layer is always 1).
    pub default_weight: f32,
    pub blending_mode: AnimatorLayerBlendingMode,
    pub avatar_mask: Option<AvatarMask>,
    /// The `.mask` asset `avatar_mask` was loaded from (saved instead of the inline mask).
    pub avatar_mask_asset: Option<Handle<AvatarMask>>,
    /// Calls `OnAnimatorIK(layer)` on scripts after this layer is evaluated.
    pub ik_pass: bool,
    /// Sync: reuse another layer's state machine (same states and transitions) with
    /// different motions (`synced_motions`), e.g. an injured walk cycle.
    pub synced_layer_index: Option<usize>,
    /// Sync: this layer's motion lengths drive the timing (else the source layer's do).
    pub synced_layer_affects_timing: bool,
    pub synced_motions: Vec<SyncedMotion>,
}

impl Default for AnimatorControllerLayer {
    fn default() -> Self {
        Self::new("New Layer")
    }
}

impl AnimatorControllerLayer {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            state_machine: AnimatorStateMachine::default(),
            default_weight: 1.0,
            blending_mode: AnimatorLayerBlendingMode::Override,
            avatar_mask: None,
            avatar_mask_asset: None,
            ik_pass: false,
            synced_layer_index: None,
            synced_layer_affects_timing: false,
            synced_motions: vec![],
        }
    }

    /// A synced layer's motion for `state` (`Empty` when none was set).
    pub fn synced_motion(&self, state: &str) -> Option<&Motion> {
        self.synced_motions
            .iter()
            .find(|m| m.state == state)
            .map(|m| &m.motion)
    }

    pub fn synced_motion_mut(&mut self, state: &str) -> &mut Motion {
        if let Some(i) = self.synced_motions.iter().position(|m| m.state == state) {
            return &mut self.synced_motions[i].motion;
        }
        self.synced_motions.push(SyncedMotion {
            state: state.to_owned(),
            motion: Motion::Empty,
        });
        &mut self.synced_motions.last_mut().expect("pushed").motion
    }
}

/// Unity `AnimatorController`: parameters and layers of state machines.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimatorController {
    pub name: String,
    pub parameters: Vec<Parameter>,
    /// Layer 0 is the Base Layer.
    pub layers: Vec<AnimatorControllerLayer>,
}

impl Asset for AnimatorController {
    const TYPE_NAME: &'static str = "AnimatorController";
}

impl AnimatorController {
    /// One looping state per clip (named after the clip), first clip as default — what
    /// Unity sets up when you drag an animated model into a scene.
    pub fn from_clips(name: &str, clips: &[(String, Handle<AnimationClip>)]) -> Self {
        let mut c = Self {
            name: name.into(),
            ..Default::default()
        };
        for (n, h) in clips {
            c.add_state(n, Motion::Clip(h.clone()));
        }
        c
    }

    /// Converts old file formats (blend trees) in place.
    pub fn upgrade(&mut self) {
        for l in &mut self.layers {
            l.state_machine.upgrade();
            for m in &mut l.synced_motions {
                m.motion.upgrade();
            }
        }
    }

    /// The Base Layer's state machine (created on first use).
    pub fn base(&mut self) -> &mut AnimatorStateMachine {
        if self.layers.is_empty() {
            self.layers.push(AnimatorControllerLayer::new("Base Layer"));
        }
        &mut self.layers[0].state_machine
    }

    /// States of the Base Layer's top machine.
    pub fn states(&self) -> &[AnimatorState] {
        self.layers
            .first()
            .map_or(&[], |l| l.state_machine.states.as_slice())
    }

    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.layers.first()?.state_machine.state_index(name)
    }

    pub fn layer_index(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == name)
    }

    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.name == name)
    }

    pub fn add_parameter(&mut self, name: &str, kind: ParameterType) -> &mut Self {
        self.parameters.push(Parameter {
            name: name.into(),
            kind,
            default: 0.0,
        });
        self
    }

    /// Adds a state to the Base Layer.
    pub fn add_state(&mut self, name: &str, motion: Motion) -> &mut AnimatorState {
        self.base().add_state(name, motion)
    }

    /// Adds a transition to the Base Layer.
    pub fn add_transition(
        &mut self,
        from: Option<&str>,
        to: &str,
        conditions: Vec<Condition>,
        duration: f32,
    ) -> &mut Transition {
        self.base().add_transition(from, to, conditions, duration)
    }

    /// Unity `AddLayer`; returns the new layer to fill in.
    pub fn add_layer(&mut self, name: &str) -> &mut AnimatorControllerLayer {
        self.base();
        self.layers.push(AnimatorControllerLayer::new(name));
        self.layers.last_mut().expect("pushed")
    }

    /// Every clip the controller's states use, in order, without repeats.
    pub fn clips(&self) -> Vec<Handle<AnimationClip>> {
        let mut out: Vec<Handle<AnimationClip>> = Vec::new();
        let mut add = |h: &Handle<AnimationClip>| {
            if !out.contains(h) {
                out.push(h.clone());
            }
        };
        for l in &self.layers {
            for (_, s) in l.state_machine.flatten() {
                s.motion.for_each_clip(&mut add);
            }
            for m in &l.synced_motions {
                m.motion.for_each_clip(&mut add);
            }
        }
        out
    }

    /// Calls `f` on every clip reference (override controllers swap them).
    pub fn for_each_clip_mut(&mut self, f: &mut dyn FnMut(&mut Handle<AnimationClip>)) {
        fn walk(m: &mut AnimatorStateMachine, f: &mut dyn FnMut(&mut Handle<AnimationClip>)) {
            for s in &mut m.states {
                s.motion.for_each_clip_mut(f);
            }
            for c in &mut m.state_machines {
                walk(c, f);
            }
        }
        for l in &mut self.layers {
            walk(&mut l.state_machine, f);
            for m in &mut l.synced_motions {
                m.motion.for_each_clip_mut(f);
            }
        }
    }
}

/// Unity's Animator Override Controller (`.overrideController`): a controller's state
/// machines with some clips swapped (a different character's run, a pistol idle). It loads
/// as a ready-to-use [`AnimatorController`] under its own path.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnimatorOverrideController {
    pub controller: Option<Handle<AnimatorController>>,
    pub overrides: Vec<ClipOverride>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipOverride {
    pub original: Handle<AnimationClip>,
    /// `None` keeps the original.
    pub replacement: Option<Handle<AnimationClip>>,
}

impl Asset for AnimatorOverrideController {
    const TYPE_NAME: &'static str = "AnimatorOverrideController";
}

impl AnimatorOverrideController {
    /// `base` with this controller's clips swapped in.
    pub fn resolve(&self, base: &AnimatorController, name: &str) -> AnimatorController {
        let mut c = base.clone();
        c.name = name.to_owned();
        c.for_each_clip_mut(&mut |h| {
            if let Some(r) = self
                .overrides
                .iter()
                .find(|o| &o.original == h)
                .and_then(|o| o.replacement.clone())
            {
                *h = r;
            }
        });
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_includes_subtrees() {
        let m = AvatarMask {
            name: "Upper".into(),
            transform_paths: vec!["Hips/Spine".into()],
            ..Default::default()
        };
        assert!(m.includes("Hips/Spine"));
        assert!(m.includes("Hips/Spine/Chest/Head"));
        assert!(m.includes("Armature/Hips/Spine/Chest"));
        assert!(!m.includes("Hips"));
        assert!(!m.includes("Hips/LeftUpLeg"));
        assert!(!m.includes("Hips/Spine2"));
    }

    #[test]
    fn old_files_load_and_upgrade() {
        let old = "
name: Old
parameters:
- name: Speed
  kind: Float
  default: 0.0
layers:
- name: Base Layer
  state_machine:
    states:
    - name: Move
      position: [240.0, 60.0]
      tag: ''
      motion: !BlendTree1D
        parameter: Speed
        children:
        - [0.0, Assets/Walk.anim]
        - [1.0, Assets/Run.anim]
      speed: 1.0
      looping: true
    default_state: 0
    transitions:
    - from: null
      to: Move
      conditions: []
      duration: 0.25
      has_exit_time: false
      exit_time: 0.75
  default_weight: 1.0
  blending_mode: Override
  avatar_mask: null
  avatar_mask_asset: null
  ik_pass: false
";
        let mut c: AnimatorController = serde_yaml_ng::from_str(old).unwrap();
        c.upgrade();
        let t = c.layers[0].state_machine.states[0]
            .motion
            .as_tree()
            .unwrap();
        assert_eq!(t.blend_parameter, "Speed");
        assert_eq!(t.effective_thresholds(), vec![0.0, 1.0]);
        assert!(c.layers[0].state_machine.states[0].write_defaults);
        assert!(c.layers[0].state_machine.transitions[0].is_any_state());
        assert!(c.layers[0].state_machine.transitions[0].fixed_duration);
        assert_eq!(c.clips().len(), 2);
    }

    #[test]
    fn nested_machines_flatten_and_find() {
        let mut sm = AnimatorStateMachine::default();
        sm.add_state("Idle", Motion::Empty);
        let loco = sm.add_state_machine("Locomotion");
        loco.add_state("Walk", Motion::Empty);
        loco.add_state_machine("Air")
            .add_state("Fall", Motion::Empty);
        let names: Vec<_> = sm.flatten().iter().map(|(_, s)| s.name.clone()).collect();
        assert_eq!(names, ["Idle", "Walk", "Fall"]);
        let p = sm.find_state("Base Layer.Locomotion.Air.Fall").unwrap();
        assert_eq!(p.machine, vec![0, 0]);
        assert_eq!(sm.find_machine("Air"), Some(vec![0, 0]));
        let mut empty = AnimatorStateMachine::default();
        empty
            .add_state_machine("Sub")
            .add_state("Deep", Motion::Empty);
        assert_eq!(empty.default_state_path().unwrap().machine, vec![0]);
    }

    #[test]
    fn automatic_thresholds_spread_evenly() {
        let mut t = BlendTree {
            max_threshold: 2.0,
            ..Default::default()
        };
        for _ in 0..3 {
            t.add_child(Motion::Empty);
        }
        assert_eq!(t.effective_thresholds(), vec![0.0, 1.0, 2.0]);
    }
}
