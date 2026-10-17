//! Unity's `Random`: one global generator (seedable with [`Random::init_state`]) for game
//! code — values, ranges, points in spheres and circles, rotations and colours.
//! <https://docs.unity3d.com/ScriptReference/Random.html>

use oxide_math::{Color, Quaternion, Vector2, Vector3};
use std::sync::atomic::{AtomicU64, Ordering};

static STATE: AtomicU64 = AtomicU64::new(0x853c_49e6_748f_ea9b);

/// Unity `Random` (static API).
pub struct Random;

impl Random {
    /// Unity `Random.InitState(seed)`: the same seed gives the same sequence.
    pub fn init_state(seed: u64) {
        STATE.store(seed ^ 0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    }

    /// Unity `Random.state` (save it and set it back to replay a sequence).
    pub fn state() -> u64 {
        STATE.load(Ordering::Relaxed)
    }

    pub fn set_state(state: u64) {
        STATE.store(state, Ordering::Relaxed);
    }

    /// The next 64 random bits (wyrand).
    pub fn next_u64() -> u64 {
        let s = STATE
            .fetch_add(0xa076_1d64_78bd_642f, Ordering::Relaxed)
            .wrapping_add(0xa076_1d64_78bd_642f);
        let t = (s as u128).wrapping_mul((s ^ 0xe703_7ed1_a0b4_28db) as u128);
        ((t >> 64) ^ t) as u64
    }

    /// Unity `Random.value`: 0..=1.
    pub fn value() -> f32 {
        (Self::next_u64() >> 40) as f32 / ((1u64 << 24) - 1) as f32
    }

    /// Unity `Random.Range(min, max)` for floats: `min..=max`.
    pub fn range(min: f32, max: f32) -> f32 {
        min + (max - min) * Self::value()
    }

    /// Unity `Random.Range(min, max)` for ints: `min..max` (max excluded; `min` when empty).
    pub fn range_int(min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        let span = (max as i64 - min as i64) as u64;
        (min as i64 + (Self::next_u64() % span) as i64) as i32
    }

    /// Unity `Random.insideUnitSphere`.
    pub fn inside_unit_sphere() -> Vector3 {
        loop {
            let p = Vector3::new(
                Self::range(-1.0, 1.0),
                Self::range(-1.0, 1.0),
                Self::range(-1.0, 1.0),
            );
            if p.length_squared() <= 1.0 {
                return p;
            }
        }
    }

    /// Unity `Random.onUnitSphere`.
    pub fn on_unit_sphere() -> Vector3 {
        loop {
            let p = Self::inside_unit_sphere();
            let l = p.length();
            if l > 1e-4 {
                return p / l;
            }
        }
    }

    /// Unity `Random.insideUnitCircle`.
    pub fn inside_unit_circle() -> Vector2 {
        loop {
            let p = Vector2::new(Self::range(-1.0, 1.0), Self::range(-1.0, 1.0));
            if p.length_squared() <= 1.0 {
                return p;
            }
        }
    }

    /// Unity `Random.rotationUniform` (and `rotation`).
    pub fn rotation() -> Quaternion {
        // Shoemake's uniform random quaternion.
        let (u1, u2, u3) = (Self::value(), Self::value(), Self::value());
        let tau = std::f32::consts::TAU;
        let (a, b) = ((1.0 - u1).sqrt(), u1.sqrt());
        Quaternion::from_xyzw(
            a * (tau * u2).sin(),
            a * (tau * u2).cos(),
            b * (tau * u3).sin(),
            b * (tau * u3).cos(),
        )
        .normalize()
    }

    /// Unity `Random.ColorHSV(hueMin, hueMax, saturationMin, saturationMax, valueMin, valueMax)`.
    pub fn color_hsv(hue: (f32, f32), saturation: (f32, f32), value: (f32, f32)) -> Color {
        let h = Self::range(hue.0, hue.1).rem_euclid(1.0);
        let s = Self::range(saturation.0, saturation.1).clamp(0.0, 1.0);
        let v = Self::range(value.0, value.1).max(0.0);
        let k = |n: f32| {
            let k = (n + h * 6.0) % 6.0;
            v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
        };
        Color::new(k(5.0), k(3.0), k(1.0), 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_sequences_repeat_and_ranges_hold() {
        Random::init_state(42);
        let a: Vec<f32> = (0..5).map(|_| Random::value()).collect();
        Random::init_state(42);
        let b: Vec<f32> = (0..5).map(|_| Random::value()).collect();
        assert_eq!(a, b);
        for _ in 0..1000 {
            let v = Random::range(-2.0, 3.0);
            assert!((-2.0..=3.0).contains(&v));
            let i = Random::range_int(0, 4);
            assert!((0..4).contains(&i));
            assert!(Random::inside_unit_sphere().length() <= 1.0 + 1e-5);
            assert!((Random::on_unit_sphere().length() - 1.0).abs() < 1e-4);
            assert!((Random::rotation().length() - 1.0).abs() < 1e-4);
        }
        assert_eq!(Random::range_int(5, 5), 5);
        // Every int in the range comes up.
        let mut seen = [false; 4];
        for _ in 0..200 {
            seen[Random::range_int(0, 4) as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
        let c = Random::color_hsv((0.0, 0.0), (1.0, 1.0), (1.0, 1.0));
        assert!((c.r - 1.0).abs() < 1e-5 && c.g.abs() < 1e-5, "{c:?}");
    }
}
