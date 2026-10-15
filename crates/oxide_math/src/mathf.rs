//! Unity `Mathf`. Reference: <https://docs.unity3d.com/ScriptReference/Mathf.html>

/// Namespace for Unity's `Mathf` functions (`Mathf::lerp`, `Mathf::DEG2RAD`, ...).
pub struct Mathf;

impl Mathf {
    pub const PI: f32 = std::f32::consts::PI;
    pub const DEG2RAD: f32 = std::f32::consts::PI / 180.0;
    pub const RAD2DEG: f32 = 180.0 / std::f32::consts::PI;
    /// Smallest positive `f32` (Unity's `Mathf.Epsilon` is the denormal minimum, not machine epsilon).
    pub const EPSILON: f32 = f32::from_bits(1);
    pub const INFINITY: f32 = f32::INFINITY;
    pub const NEGATIVE_INFINITY: f32 = f32::NEG_INFINITY;

    pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
        if value < min {
            min
        } else if value > max {
            max
        } else {
            value
        }
    }

    pub fn clamp01(value: f32) -> f32 {
        Self::clamp(value, 0.0, 1.0)
    }

    /// Interpolates with `t` clamped to `[0, 1]`.
    pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * Self::clamp01(t)
    }

    pub fn lerp_unclamped(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }

    pub fn inverse_lerp(a: f32, b: f32, value: f32) -> f32 {
        if a != b {
            Self::clamp01((value - a) / (b - a))
        } else {
            0.0
        }
    }

    /// Like `lerp` but wraps correctly around 360 degrees.
    pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
        let delta = Self::repeat(b - a, 360.0);
        let delta = if delta > 180.0 { delta - 360.0 } else { delta };
        a + delta * Self::clamp01(t)
    }

    pub fn move_towards(current: f32, target: f32, max_delta: f32) -> f32 {
        if (target - current).abs() <= max_delta {
            target
        } else {
            current + (target - current).signum() * max_delta
        }
    }

    pub fn move_towards_angle(current: f32, target: f32, max_delta: f32) -> f32 {
        let delta = Self::delta_angle(current, target);
        if -max_delta < delta && delta < max_delta {
            return target;
        }
        Self::move_towards(current, current + delta, max_delta)
    }

    /// Hermite interpolation between `from` and `to`.
    pub fn smooth_step(from: f32, to: f32, t: f32) -> f32 {
        let t = Self::clamp01(t);
        let t = -2.0 * t * t * t + 3.0 * t * t;
        to * t + from * (1.0 - t)
    }

    /// Critically damped spring. Returns the new value; updates `current_velocity` in place.
    pub fn smooth_damp(
        current: f32,
        target: f32,
        current_velocity: &mut f32,
        smooth_time: f32,
        max_speed: f32,
        delta_time: f32,
    ) -> f32 {
        let smooth_time = smooth_time.max(0.0001);
        let omega = 2.0 / smooth_time;
        let x = omega * delta_time;
        let exp = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
        let original_to = target;
        let max_change = max_speed * smooth_time;
        let change = Self::clamp(current - target, -max_change, max_change);
        let target = current - change;
        let temp = (*current_velocity + omega * change) * delta_time;
        *current_velocity = (*current_velocity - omega * temp) * exp;
        let mut output = target + (change + temp) * exp;
        if (original_to - current > 0.0) == (output > original_to) {
            output = original_to;
            *current_velocity = (output - original_to) / delta_time;
        }
        output
    }

    /// Compares two floats with a tolerance relative to their magnitude.
    pub fn approximately(a: f32, b: f32) -> bool {
        (b - a).abs() < (1e-6 * a.abs().max(b.abs())).max(f32::EPSILON * 8.0)
    }

    /// Loops `t` so it is never larger than `length` and never smaller than 0.
    pub fn repeat(t: f32, length: f32) -> f32 {
        Self::clamp(t - (t / length).floor() * length, 0.0, length)
    }

    pub fn ping_pong(t: f32, length: f32) -> f32 {
        let t = Self::repeat(t, length * 2.0);
        length - (t - length).abs()
    }

    /// Shortest signed difference between two angles in degrees, in `(-180, 180]`.
    pub fn delta_angle(current: f32, target: f32) -> f32 {
        let delta = Self::repeat(target - current, 360.0);
        if delta > 180.0 { delta - 360.0 } else { delta }
    }

    /// Unity's `Mathf.Sign` returns 1 for zero.
    pub fn sign(f: f32) -> f32 {
        if f >= 0.0 { 1.0 } else { -1.0 }
    }

    pub fn is_power_of_two(value: u32) -> bool {
        value.is_power_of_two()
    }

    pub fn next_power_of_two(value: u32) -> u32 {
        value.max(1).next_power_of_two()
    }

    pub fn closest_power_of_two(value: u32) -> u32 {
        if value <= 1 {
            return 1;
        }
        let next = value.next_power_of_two();
        let prev = next >> 1;
        if value - prev < next - value {
            prev
        } else {
            next
        }
    }

    /// sRGB → linear (exact sRGB transfer function).
    pub fn gamma_to_linear_space(value: f32) -> f32 {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    /// Linear → sRGB (exact sRGB transfer function).
    pub fn linear_to_gamma_space(value: f32) -> f32 {
        if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Mathf;

    #[test]
    fn lerp_clamps_unclamped_does_not() {
        assert_eq!(Mathf::lerp(0.0, 10.0, 1.5), 10.0);
        assert_eq!(Mathf::lerp_unclamped(0.0, 10.0, 1.5), 15.0);
        assert_eq!(Mathf::inverse_lerp(0.0, 10.0, 2.5), 0.25);
    }

    #[test]
    fn angles_wrap() {
        assert_eq!(Mathf::delta_angle(350.0, 10.0), 20.0);
        assert_eq!(Mathf::delta_angle(10.0, 350.0), -20.0);
        assert_eq!(Mathf::lerp_angle(350.0, 10.0, 0.5), 360.0);
        assert_eq!(Mathf::repeat(-1.0, 3.0), 2.0);
        assert_eq!(Mathf::ping_pong(3.0, 2.0), 1.0);
    }

    #[test]
    fn misc() {
        assert_eq!(Mathf::sign(0.0), 1.0);
        assert_eq!(Mathf::move_towards(0.0, 10.0, 3.0), 3.0);
        assert_eq!(Mathf::move_towards(9.0, 10.0, 3.0), 10.0);
        assert_eq!(Mathf::closest_power_of_two(5), 4);
        assert_eq!(Mathf::closest_power_of_two(7), 8);
        assert_eq!(Mathf::next_power_of_two(17), 32);
        assert!(Mathf::approximately(1.0, 1.0 + 1e-7));
        let g = Mathf::linear_to_gamma_space(Mathf::gamma_to_linear_space(0.5));
        assert!((g - 0.5).abs() < 1e-6);
    }

    #[test]
    fn smooth_damp_converges() {
        let (mut x, mut v) = (0.0, 0.0);
        for _ in 0..600 {
            x = Mathf::smooth_damp(x, 10.0, &mut v, 0.3, f32::INFINITY, 1.0 / 60.0);
        }
        assert!((x - 10.0).abs() < 1e-3);
    }
}
