//! Unity `AnimationClip` for transform animation: keyframe tracks bound by hierarchy path.
//! Reference: <https://docs.unity3d.com/Manual/AnimationClips.html>

use crate::handle::Asset;
use oxide_math::{Quaternion, Vector3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
    /// glTF cubic spline: `values` holds `[in_tangent, value, out_tangent]` per key.
    CubicSpline,
}

pub trait Animatable: Copy {
    fn lerp(a: Self, b: Self, t: f32) -> Self;
    fn hermite(v0: Self, m0: Self, v1: Self, m1: Self, t: f32, dt: f32) -> Self;
}

impl Animatable for Vector3 {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        a.lerp(b, t)
    }
    fn hermite(v0: Self, m0: Self, v1: Self, m1: Self, t: f32, dt: f32) -> Self {
        let (t2, t3) = (t * t, t * t * t);
        v0 * (2.0 * t3 - 3.0 * t2 + 1.0)
            + m0 * dt * (t3 - 2.0 * t2 + t)
            + v1 * (-2.0 * t3 + 3.0 * t2)
            + m1 * dt * (t3 - t2)
    }
}

impl Animatable for Quaternion {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        a.slerp(b, t)
    }
    fn hermite(v0: Self, m0: Self, v1: Self, m1: Self, t: f32, dt: f32) -> Self {
        let (t2, t3) = (t * t, t * t * t);
        let v = v0 * (2.0 * t3 - 3.0 * t2 + 1.0)
            + m0 * (dt * (t3 - 2.0 * t2 + t))
            + v1 * (-2.0 * t3 + 3.0 * t2)
            + m1 * (dt * (t3 - t2));
        v.normalize()
    }
}

#[derive(Clone, Debug)]
pub struct Track<T> {
    pub times: Vec<f32>,
    pub values: Vec<T>,
    pub interpolation: Interpolation,
}

impl<T: Animatable> Track<T> {
    pub fn sample(&self, t: f32) -> Option<T> {
        let n = self.times.len();
        if n == 0 {
            return None;
        }
        let cubic = self.interpolation == Interpolation::CubicSpline;
        let val = |i: usize| {
            if cubic {
                self.values[i * 3 + 1]
            } else {
                self.values[i]
            }
        };
        if t <= self.times[0] || n == 1 {
            return Some(val(0));
        }
        if t >= self.times[n - 1] {
            return Some(val(n - 1));
        }
        let i = self.times.partition_point(|&k| k <= t) - 1;
        let (t0, t1) = (self.times[i], self.times[i + 1]);
        let dt = t1 - t0;
        let f = if dt > 0.0 { (t - t0) / dt } else { 0.0 };
        Some(match self.interpolation {
            Interpolation::Step => val(i),
            Interpolation::Linear => T::lerp(val(i), val(i + 1), f),
            Interpolation::CubicSpline => T::hermite(
                val(i),
                self.values[i * 3 + 2],
                val(i + 1),
                self.values[(i + 1) * 3],
                f,
                dt,
            ),
        })
    }

    pub fn end_time(&self) -> f32 {
        self.times.last().copied().unwrap_or(0.0)
    }
}

/// Animation of one transform, found by its path below the animated root
/// (`"Armature/Hips/Spine"`, like Unity's binding paths).
#[derive(Clone, Debug, Default)]
pub struct TransformCurves {
    pub path: String,
    pub position: Option<Track<Vector3>>,
    pub rotation: Option<Track<Quaternion>>,
    pub scale: Option<Track<Vector3>>,
}

/// Unity `AnimationEvent`: calls `function_name` on the animated object's scripts (as a
/// `SendMessage`) when playback passes `time` (seconds).
#[derive(Clone, Debug, Default)]
pub struct AnimationEvent {
    pub time: f32,
    pub function_name: String,
    /// Passed as the message argument (`Value::Null` for none).
    pub parameter: oxide_core::reflect::Value,
}

#[derive(Clone, Debug, Default)]
pub struct AnimationClip {
    pub name: String,
    pub length: f32,
    pub looping: bool,
    pub curves: Vec<TransformCurves>,
    /// Sorted by time.
    pub events: Vec<AnimationEvent>,
    /// Path of the transform whose motion becomes root motion (Unity's import setting
    /// "Root node"). `None` picks the shallowest transform with a position curve.
    pub root_node: Option<String>,
}

impl Asset for AnimationClip {
    const TYPE_NAME: &'static str = "AnimationClip";
}

impl AnimationClip {
    /// Unity `AnimationClip.AddEvent`.
    pub fn add_event(
        &mut self,
        time: f32,
        function_name: &str,
        parameter: oxide_core::reflect::Value,
    ) {
        self.events.push(AnimationEvent {
            time,
            function_name: function_name.into(),
            parameter,
        });
        self.events.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    /// Index into `curves` of the root-motion transform.
    pub fn root_curve(&self) -> Option<usize> {
        match &self.root_node {
            Some(p) => self.curves.iter().position(|c| &c.path == p),
            None => self
                .curves
                .iter()
                .enumerate()
                .filter(|(_, c)| c.position.is_some())
                .min_by_key(|(_, c)| c.path.matches('/').count())
                .map(|(i, _)| i),
        }
    }

    pub fn recalculate_length(&mut self) {
        self.length = self
            .curves
            .iter()
            .flat_map(|c| {
                [
                    c.position.as_ref().map(Track::end_time),
                    c.rotation.as_ref().map(Track::end_time),
                    c.scale.as_ref().map(Track::end_time),
                ]
            })
            .flatten()
            .fold(0.0, f32::max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_and_step() {
        let mut tr = Track {
            times: vec![0.0, 1.0],
            values: vec![Vector3::ZERO, Vector3::X * 2.0],
            interpolation: Interpolation::Linear,
        };
        assert_eq!(tr.sample(0.5), Some(Vector3::X));
        assert_eq!(tr.sample(5.0), Some(Vector3::X * 2.0));
        tr.interpolation = Interpolation::Step;
        assert_eq!(tr.sample(0.99), Some(Vector3::ZERO));
    }
}

/// Unity `AnimatorControllerParameterType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterType {
    Float,
    Int,
    Bool,
    Trigger,
}

#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: String,
    pub kind: ParameterType,
    pub default: f32,
}

/// Unity `AnimatorConditionMode`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConditionMode {
    /// Bool true / trigger set.
    If,
    IfNot,
    Greater(f32),
    Less(f32),
    Equals(f32),
    NotEqual(f32),
}

#[derive(Clone, Debug)]
pub struct Condition {
    pub parameter: String,
    pub mode: ConditionMode,
}

/// Unity `BlendTreeType` for two-parameter blend trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlendTree2DType {
    /// Motions at different directions (strafe walk/run sets). Gradient-band interpolation in
    /// polar space, so speed and direction blend separately.
    SimpleDirectional,
    /// Several motions per direction (walk and run forward, ...); same interpolation.
    FreeformDirectional,
    /// Motions that are not directions (e.g. lean / aim offsets). Cartesian gradient bands.
    #[default]
    FreeformCartesian,
}

#[derive(Clone, Debug)]
pub enum Motion {
    Clip(crate::handle::Handle<AnimationClip>),
    /// 1D blend tree: children sorted by threshold, blended on `parameter`.
    BlendTree1D {
        parameter: String,
        children: Vec<(f32, crate::handle::Handle<AnimationClip>)>,
    },
    /// 2D blend tree: each child sits at a position in (`parameter_x`, `parameter_y`).
    BlendTree2D {
        parameter_x: String,
        parameter_y: String,
        kind: BlendTree2DType,
        children: Vec<(oxide_math::Vector2, crate::handle::Handle<AnimationClip>)>,
    },
    /// No motion (holds the current pose).
    Empty,
}

#[derive(Clone, Debug)]
pub struct AnimatorState {
    pub name: String,
    /// Unity state tag (`AnimatorStateInfo.IsTag`).
    pub tag: String,
    pub motion: Motion,
    pub speed: f32,
    pub looping: bool,
}

#[derive(Clone, Debug)]
pub struct Transition {
    /// Source state name; `None` = Any State.
    pub from: Option<String>,
    pub to: String,
    pub conditions: Vec<Condition>,
    /// Crossfade duration in seconds.
    pub duration: f32,
    /// Only transition after `exit_time` (normalized) of the source state.
    pub has_exit_time: bool,
    pub exit_time: f32,
}

/// Unity `AnimatorStateMachine`: the states and transitions of one layer.
#[derive(Clone, Debug, Default)]
pub struct AnimatorStateMachine {
    pub states: Vec<AnimatorState>,
    pub default_state: usize,
    pub transitions: Vec<Transition>,
}

impl AnimatorStateMachine {
    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name == name)
    }

    pub fn add_state(&mut self, name: &str, motion: Motion) -> &mut AnimatorState {
        self.states.push(AnimatorState {
            name: name.into(),
            tag: String::new(),
            motion,
            speed: 1.0,
            looping: true,
        });
        self.states.last_mut().expect("pushed")
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
            has_exit_time: false,
            exit_time: 1.0,
        });
        self.transitions.last_mut().expect("pushed")
    }
}

/// Unity `AvatarMask` (transform part): which transforms a layer animates. A transform is
/// included when its path equals, or sits below, one of `transform_paths`; an empty list
/// includes everything.
#[derive(Clone, Debug, Default)]
pub struct AvatarMask {
    pub name: String,
    pub transform_paths: Vec<String>,
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
}

/// Unity `AnimatorLayerBlendingMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AnimatorLayerBlendingMode {
    /// Replaces the layers above it (by `weight`).
    #[default]
    Override,
    /// Adds the motion relative to each clip's first frame.
    Additive,
}

/// Unity `AnimatorControllerLayer`.
#[derive(Clone, Debug)]
pub struct AnimatorControllerLayer {
    pub name: String,
    pub state_machine: AnimatorStateMachine,
    /// Starting weight (the base layer is always 1).
    pub default_weight: f32,
    pub blending_mode: AnimatorLayerBlendingMode,
    pub avatar_mask: Option<AvatarMask>,
    /// Calls `OnAnimatorIK(layer)` on scripts after this layer is evaluated.
    pub ik_pass: bool,
}

impl AnimatorControllerLayer {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            state_machine: AnimatorStateMachine::default(),
            default_weight: 1.0,
            blending_mode: AnimatorLayerBlendingMode::Override,
            avatar_mask: None,
            ik_pass: false,
        }
    }
}

/// Unity `AnimatorController`: parameters and layers of state machines.
#[derive(Clone, Debug, Default)]
pub struct AnimatorController {
    pub name: String,
    pub parameters: Vec<Parameter>,
    /// Layer 0 is the Base Layer.
    pub layers: Vec<AnimatorControllerLayer>,
}

impl crate::handle::Asset for AnimatorController {
    const TYPE_NAME: &'static str = "AnimatorController";
}

impl AnimatorController {
    /// One looping state per clip (named after the clip), first clip as default — what
    /// Unity sets up when you drag an animated model into a scene.
    pub fn from_clips(
        name: &str,
        clips: &[(String, crate::handle::Handle<AnimationClip>)],
    ) -> Self {
        let mut c = Self {
            name: name.into(),
            ..Default::default()
        };
        for (n, h) in clips {
            c.add_state(n, Motion::Clip(h.clone()));
        }
        c
    }

    /// The Base Layer's state machine (created on first use).
    pub fn base(&mut self) -> &mut AnimatorStateMachine {
        if self.layers.is_empty() {
            self.layers.push(AnimatorControllerLayer::new("Base Layer"));
        }
        &mut self.layers[0].state_machine
    }

    /// States of the Base Layer.
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
}

#[cfg(test)]
mod mask_tests {
    use super::*;

    #[test]
    fn mask_includes_subtrees() {
        let m = AvatarMask {
            name: "Upper".into(),
            transform_paths: vec!["Hips/Spine".into()],
        };
        assert!(m.includes("Hips/Spine"));
        assert!(m.includes("Hips/Spine/Chest/Head"));
        assert!(m.includes("Armature/Hips/Spine/Chest"));
        assert!(!m.includes("Hips"));
        assert!(!m.includes("Hips/LeftUpLeg"));
        assert!(!m.includes("Hips/Spine2"));
    }
}
