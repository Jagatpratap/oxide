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

#[derive(Clone, Debug, Default)]
pub struct AnimationClip {
    pub name: String,
    pub length: f32,
    pub looping: bool,
    pub curves: Vec<TransformCurves>,
}

impl Asset for AnimationClip {
    const TYPE_NAME: &'static str = "AnimationClip";
}

impl AnimationClip {
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
