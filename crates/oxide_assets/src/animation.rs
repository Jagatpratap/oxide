//! Unity `AnimationClip` for transform animation: keyframe tracks bound by hierarchy path.
//! Reference: <https://docs.unity3d.com/Manual/AnimationClips.html>

use crate::handle::Asset;
use oxide_math::{Quaternion, Vector3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct TransformCurves {
    pub path: String,
    pub position: Option<Track<Vector3>>,
    pub rotation: Option<Track<Quaternion>>,
    pub scale: Option<Track<Vector3>>,
}

/// Unity `AnimationEvent`: calls `function_name` on the animated object's scripts (as a
/// `SendMessage`) when playback passes `time` (seconds).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct AnimationEvent {
    pub time: f32,
    pub function_name: String,
    /// Passed as the message argument (`Value::Null` for none).
    pub parameter: oxide_core::reflect::Value,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
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
    /// Curves on any reflected component property (Light intensity, a material colour's
    /// channel, script fields), like Unity's generic float curves.
    #[serde(default)]
    pub properties: Vec<PropertyCurve>,
    /// Keyframes that swap an asset or text field (Unity's object reference curves): a
    /// SpriteRenderer's sprite for sprite animation, a Sprite Resolver's label.
    #[serde(default)]
    pub object_curves: Vec<ObjectCurve>,
}

/// One object-reference key: from `time` on, the field holds `value`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ObjectKey {
    pub time: f32,
    /// An asset path (`Assets/hero.png#run_2`) or text.
    pub value: String,
}

/// Stepped keys on an asset or string field of a component.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ObjectCurve {
    pub path: String,
    pub component: String,
    pub property: String,
    /// Sorted by time.
    pub keys: Vec<ObjectKey>,
}

impl ObjectCurve {
    /// The value at `t` (the last key at or before it; the first key before that).
    pub fn sample(&self, t: f32) -> Option<&str> {
        let i = self.keys.partition_point(|k| k.time <= t + 1e-6);
        self.keys.get(i.saturating_sub(1)).map(|k| k.value.as_str())
    }

    /// Adds or replaces the key at `t`, keeping order.
    pub fn set_key(&mut self, t: f32, value: &str) {
        match self.keys.iter_mut().find(|k| (k.time - t).abs() < 1e-4) {
            Some(k) => k.value = value.to_owned(),
            None => {
                let i = self.keys.partition_point(|k| k.time < t);
                self.keys.insert(
                    i,
                    ObjectKey {
                        time: t,
                        value: value.to_owned(),
                    },
                );
            }
        }
    }
}

/// A float curve on a component property of a transform below the animated root (Unity
/// `EditorCurveBinding` + `AnimationCurve`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PropertyCurve {
    /// Transform path below the animated root; empty = the root itself.
    pub path: String,
    /// Component type name (`Light`, `MeshRenderer`, a script type).
    pub component: String,
    /// Field name, with `.x/.y/.z/.w` or `.r/.g/.b/.a` for one channel of a vector or
    /// colour (`intensity`, `color.r`).
    pub property: String,
    pub curve: oxide_core::AnimationCurve,
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
        let transforms = self
            .curves
            .iter()
            .flat_map(|c| {
                [
                    c.position.as_ref().map(Track::end_time),
                    c.rotation.as_ref().map(Track::end_time),
                    c.scale.as_ref().map(Track::end_time),
                ]
            })
            .flatten();
        let props = self.properties.iter().map(|p| p.curve.time_range().1);
        // A sprite animation's last frame lasts one frame (12 fps, Unity's default).
        let objects = self.object_curves.iter().filter_map(|c| {
            c.keys
                .last()
                .map(|k| k.time + if c.keys.len() > 1 { 1.0 / 12.0 } else { 0.0 })
        });
        self.length = transforms.chain(props).chain(objects).fold(0.0, f32::max);
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
