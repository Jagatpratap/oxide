//! Unity `Quaternion` and `Matrix4x4` statics.
//! References: <https://docs.unity3d.com/ScriptReference/Quaternion.html>,
//! <https://docs.unity3d.com/ScriptReference/Matrix4x4.html>

use crate::{Mathf, Matrix4x4, Quaternion, Vector3};

/// Unity-named functions for [`Quaternion`]. All angles are degrees.
pub trait QuaternionExt: Sized {
    /// Unity `Quaternion.Euler`: rotates `z` around Z, then `x` around X, then `y` around Y.
    fn euler(x: f32, y: f32, z: f32) -> Self;
    fn euler_v(angles: Vector3) -> Self;
    /// Unity `Quaternion.eulerAngles`: each component in `[0, 360)`.
    fn euler_angles(&self) -> Vector3;
    fn angle_axis(angle: f32, axis: Vector3) -> Self;
    /// Rotation whose +Z faces `forward` and +Y is as close to `up` as possible.
    fn look_rotation(forward: Vector3, up: Vector3) -> Self;
    /// Angle in degrees between two rotations.
    fn angle(a: Self, b: Self) -> f32;
    fn rotate_towards(from: Self, to: Self, max_degrees_delta: f32) -> Self;
}

impl QuaternionExt for Quaternion {
    fn euler(x: f32, y: f32, z: f32) -> Self {
        Quaternion::from_rotation_y(y * Mathf::DEG2RAD)
            * Quaternion::from_rotation_x(x * Mathf::DEG2RAD)
            * Quaternion::from_rotation_z(z * Mathf::DEG2RAD)
    }

    fn euler_v(angles: Vector3) -> Self {
        Self::euler(angles.x, angles.y, angles.z)
    }

    fn euler_angles(&self) -> Vector3 {
        let (y, x, z) = self.to_euler(glam::EulerRot::YXZ);
        let wrap = |rad: f32| Mathf::repeat(rad * Mathf::RAD2DEG, 360.0) % 360.0;
        Vector3::new(wrap(x), wrap(y), wrap(z))
    }

    fn angle_axis(angle: f32, axis: Vector3) -> Self {
        let axis = axis.normalize_or_zero();
        if axis == Vector3::ZERO {
            return Quaternion::IDENTITY;
        }
        Quaternion::from_axis_angle(axis, angle * Mathf::DEG2RAD)
    }

    fn look_rotation(forward: Vector3, up: Vector3) -> Self {
        let z = forward.normalize_or_zero();
        if z == Vector3::ZERO {
            return Quaternion::IDENTITY;
        }
        let mut x = up.cross(z);
        if x.length_squared() < 1e-12 {
            // `up` parallel to `forward`: pick any perpendicular right axis.
            x = z.any_orthonormal_vector();
        }
        let x = x.normalize();
        let y = z.cross(x);
        Quaternion::from_mat3(&glam::Mat3::from_cols(x, y, z)).normalize()
    }

    fn angle(a: Self, b: Self) -> f32 {
        let dot = a.dot(b).abs().min(1.0);
        if dot > 1.0 - 1e-6 {
            0.0
        } else {
            dot.acos() * 2.0 * Mathf::RAD2DEG
        }
    }

    fn rotate_towards(from: Self, to: Self, max_degrees_delta: f32) -> Self {
        let angle = Self::angle(from, to);
        if angle == 0.0 {
            return to;
        }
        from.slerp(to, (max_degrees_delta / angle).min(1.0))
    }
}

/// Unity-named functions for [`Matrix4x4`].
pub trait Matrix4x4Ext: Sized {
    /// Translation, rotation, scale (Unity `Matrix4x4.TRS`).
    fn trs(pos: Vector3, q: Quaternion, s: Vector3) -> Self;
}

impl Matrix4x4Ext for Matrix4x4 {
    fn trs(pos: Vector3, q: Quaternion, s: Vector3) -> Self {
        Matrix4x4::from_scale_rotation_translation(s, q, pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vector3Ext;

    fn close(a: Vector3, b: Vector3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn euler_matches_unity() {
        // Unity: Quaternion.Euler(0, 90, 0) * Vector3.forward == Vector3.right
        assert!(close(
            Quaternion::euler(0.0, 90.0, 0.0) * Vector3::FORWARD,
            Vector3::RIGHT
        ));
        // Unity: Quaternion.Euler(90, 0, 0) * Vector3.forward == Vector3.down
        assert!(close(
            Quaternion::euler(90.0, 0.0, 0.0) * Vector3::FORWARD,
            Vector3::DOWN
        ));
        // Z is applied first, then X, then Y.
        let q = Quaternion::euler(30.0, 40.0, 50.0);
        let manual = Quaternion::angle_axis(40.0, Vector3::UP)
            * Quaternion::angle_axis(30.0, Vector3::RIGHT)
            * Quaternion::angle_axis(50.0, Vector3::FORWARD);
        assert!(Quaternion::angle(q, manual) < 1e-3);
    }

    #[test]
    fn euler_angles_round_trip() {
        let e = Quaternion::euler(30.0, 200.0, 350.0).euler_angles();
        assert!(close(e, Vector3::new(30.0, 200.0, 350.0)), "{e}");
    }

    #[test]
    fn look_rotation() {
        let q = Quaternion::look_rotation(Vector3::RIGHT, Vector3::UP);
        assert!(close(q * Vector3::FORWARD, Vector3::RIGHT));
        assert!(close(q * Vector3::UP, Vector3::UP));
    }
}
