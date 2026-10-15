//! Unity `Vector2` / `Vector3` statics and helpers.
//! Reference: <https://docs.unity3d.com/ScriptReference/Vector3.html>
//!
//! glam already provides Unity-equivalent `lerp`, `move_towards`, `distance`, `dot`, `cross`,
//! `normalize`, `project_onto` and `reflect`; only the missing ones are added here.

use crate::{Mathf, Vector2, Vector3};

/// Unity-named constants and static functions for [`Vector3`].
pub trait Vector3Ext: Sized {
    const UP: Self;
    const DOWN: Self;
    const LEFT: Self;
    const RIGHT: Self;
    /// +Z, Unity's forward axis.
    const FORWARD: Self;
    const BACK: Self;

    /// Unsigned angle in degrees between two vectors.
    fn angle(from: Self, to: Self) -> f32;
    /// Signed angle in degrees from `from` to `to` around `axis`.
    fn signed_angle(from: Self, to: Self, axis: Self) -> f32;
    fn clamp_magnitude(v: Self, max_length: f32) -> Self;
    fn project_on_plane(v: Self, plane_normal: Self) -> Self;
    fn smooth_damp(
        current: Self,
        target: Self,
        current_velocity: &mut Self,
        smooth_time: f32,
        max_speed: f32,
        delta_time: f32,
    ) -> Self;
}

impl Vector3Ext for Vector3 {
    const UP: Self = Vector3::Y;
    const DOWN: Self = Vector3::NEG_Y;
    const LEFT: Self = Vector3::NEG_X;
    const RIGHT: Self = Vector3::X;
    const FORWARD: Self = Vector3::Z;
    const BACK: Self = Vector3::NEG_Z;

    fn angle(from: Self, to: Self) -> f32 {
        let denominator = (from.length_squared() * to.length_squared()).sqrt();
        if denominator < 1e-15 {
            return 0.0;
        }
        let dot = (from.dot(to) / denominator).clamp(-1.0, 1.0);
        dot.acos() * Mathf::RAD2DEG
    }

    fn signed_angle(from: Self, to: Self, axis: Self) -> f32 {
        let unsigned = Self::angle(from, to);
        unsigned * Mathf::sign(axis.dot(from.cross(to)))
    }

    fn clamp_magnitude(v: Self, max_length: f32) -> Self {
        v.clamp_length_max(max_length)
    }

    fn project_on_plane(v: Self, plane_normal: Self) -> Self {
        let sqr = plane_normal.length_squared();
        if sqr < f32::EPSILON {
            return v;
        }
        v - plane_normal * (v.dot(plane_normal) / sqr)
    }

    fn smooth_damp(
        current: Self,
        target: Self,
        current_velocity: &mut Self,
        smooth_time: f32,
        max_speed: f32,
        delta_time: f32,
    ) -> Self {
        let smooth_time = smooth_time.max(0.0001);
        let omega = 2.0 / smooth_time;
        let x = omega * delta_time;
        let exp = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
        let original_to = target;
        let change = (current - target).clamp_length_max(max_speed * smooth_time);
        let target = current - change;
        let temp = (*current_velocity + omega * change) * delta_time;
        *current_velocity = (*current_velocity - omega * temp) * exp;
        let mut output = target + (change + temp) * exp;
        if (original_to - current).dot(output - original_to) > 0.0 {
            output = original_to;
            *current_velocity = (output - original_to) / delta_time;
        }
        output
    }
}

/// Unity-named constants and static functions for [`Vector2`].
pub trait Vector2Ext: Sized {
    const UP: Self;
    const DOWN: Self;
    const LEFT: Self;
    const RIGHT: Self;

    fn angle(from: Self, to: Self) -> f32;
    fn signed_angle(from: Self, to: Self) -> f32;
}

impl Vector2Ext for Vector2 {
    const UP: Self = Vector2::Y;
    const DOWN: Self = Vector2::NEG_Y;
    const LEFT: Self = Vector2::NEG_X;
    const RIGHT: Self = Vector2::X;

    fn angle(from: Self, to: Self) -> f32 {
        let denominator = (from.length_squared() * to.length_squared()).sqrt();
        if denominator < 1e-15 {
            return 0.0;
        }
        (from.dot(to) / denominator).clamp(-1.0, 1.0).acos() * Mathf::RAD2DEG
    }

    fn signed_angle(from: Self, to: Self) -> f32 {
        Self::angle(from, to) * Mathf::sign(from.x * to.y - from.y * to.x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles() {
        assert!((Vector3::angle(Vector3::RIGHT, Vector3::FORWARD) - 90.0).abs() < 1e-4);
        // Left-handed, Y up: turning from forward (+Z) to right (+X) is +90 around up.
        let a = Vector3::signed_angle(Vector3::FORWARD, Vector3::RIGHT, Vector3::UP);
        assert!((a - 90.0).abs() < 1e-4, "{a}");
        assert!((Vector2::signed_angle(Vector2::RIGHT, Vector2::UP) - 90.0).abs() < 1e-4);
    }

    #[test]
    fn helpers() {
        let v = Vector3::ZERO.move_towards(Vector3::new(10.0, 0.0, 0.0), 2.0);
        assert_eq!(v, Vector3::new(2.0, 0.0, 0.0));
        let p = Vector3::project_on_plane(Vector3::new(1.0, 5.0, 2.0), Vector3::UP);
        assert_eq!(p, Vector3::new(1.0, 0.0, 2.0));
        assert_eq!(
            Vector3::clamp_magnitude(Vector3::new(0.0, 4.0, 0.0), 1.0),
            Vector3::UP
        );
    }
}
