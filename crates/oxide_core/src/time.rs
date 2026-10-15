//! Unity `Time`.
//!
//! References: <https://docs.unity3d.com/Manual/managing-time-and-frame-rate.html>,
//! <https://docs.unity3d.com/Manual/fixed-updates.html>,
//! <https://docs.unity3d.com/Manual/execution-order.html>
//!
//! Per frame the engine calls [`Time::begin_frame`], then runs `FixedUpdate` while
//! [`Time::next_fixed_step`] returns `true`, then `Update` / `LateUpdate` / render.
//! Inside a fixed step, [`Time::delta_time`] and [`Time::time`] report the fixed values,
//! exactly like Unity.

use serde::Serialize;
use std::sync::OnceLock;
use std::time::Instant;

static STARTUP: OnceLock<Instant> = OnceLock::new();

/// Seconds since the engine first asked for the time (Unity `Time.realtimeSinceStartup`).
pub fn realtime_since_startup() -> f64 {
    STARTUP.get_or_init(Instant::now).elapsed().as_secs_f64()
}

#[derive(Clone, Debug, Serialize)]
pub struct Time {
    time: f64,
    delta_time: f64,
    unscaled_time: f64,
    unscaled_delta_time: f64,
    fixed_time: f64,
    fixed_delta_time: f64,
    maximum_delta_time: f64,
    time_scale: f64,
    smooth_delta_time: f64,
    frame_count: u64,
    fixed_step_count: u64,
    in_fixed_step: bool,
}

impl Default for Time {
    fn default() -> Self {
        Self {
            time: 0.0,
            delta_time: 0.0,
            unscaled_time: 0.0,
            unscaled_delta_time: 0.0,
            fixed_time: 0.0,
            fixed_delta_time: 0.02,
            maximum_delta_time: 1.0 / 3.0,
            time_scale: 1.0,
            smooth_delta_time: 0.0,
            frame_count: 0,
            fixed_step_count: 0,
            in_fixed_step: false,
        }
    }
}

impl Time {
    pub fn new() -> Self {
        realtime_since_startup();
        Self::default()
    }

    /// Advances game time by one frame that took `real_dt` seconds of wall-clock time.
    pub fn begin_frame(&mut self, real_dt: f64) {
        let real_dt = real_dt.max(0.0);
        self.in_fixed_step = false;
        self.unscaled_delta_time = real_dt;
        self.unscaled_time += real_dt;
        self.delta_time = (real_dt * self.time_scale).min(self.maximum_delta_time);
        self.time += self.delta_time;
        self.smooth_delta_time = if self.frame_count == 0 {
            self.delta_time
        } else {
            self.smooth_delta_time * 0.8 + self.delta_time * 0.2
        };
        self.frame_count += 1;
    }

    /// Call in a loop before `Update`: returns `true` once per pending `FixedUpdate`.
    pub fn next_fixed_step(&mut self) -> bool {
        // Tolerance so float error doesn't drop or add a step at exact multiples.
        const EPS: f64 = 1e-9;
        if self.time_scale > 0.0 && self.fixed_time + self.fixed_delta_time <= self.time + EPS {
            self.fixed_time += self.fixed_delta_time;
            self.fixed_step_count += 1;
            self.in_fixed_step = true;
            true
        } else {
            self.in_fixed_step = false;
            false
        }
    }

    /// Unity `Time.deltaTime` (returns `fixedDeltaTime` inside `FixedUpdate`).
    pub fn delta_time(&self) -> f32 {
        (if self.in_fixed_step {
            self.fixed_delta_time
        } else {
            self.delta_time
        }) as f32
    }

    /// Unity `Time.time` (returns `fixedTime` inside `FixedUpdate`).
    pub fn time(&self) -> f32 {
        self.time_f64() as f32
    }

    pub fn time_f64(&self) -> f64 {
        if self.in_fixed_step {
            self.fixed_time
        } else {
            self.time
        }
    }

    pub fn unscaled_delta_time(&self) -> f32 {
        self.unscaled_delta_time as f32
    }

    pub fn unscaled_time(&self) -> f32 {
        self.unscaled_time as f32
    }

    pub fn fixed_time(&self) -> f32 {
        self.fixed_time as f32
    }

    pub fn smooth_delta_time(&self) -> f32 {
        self.smooth_delta_time as f32
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Total `FixedUpdate` steps run since startup.
    pub fn fixed_step_count(&self) -> u64 {
        self.fixed_step_count
    }

    pub fn in_fixed_time_step(&self) -> bool {
        self.in_fixed_step
    }

    pub fn time_scale(&self) -> f32 {
        self.time_scale as f32
    }

    pub fn set_time_scale(&mut self, scale: f32) {
        self.time_scale = scale.max(0.0) as f64;
    }

    pub fn fixed_delta_time(&self) -> f32 {
        self.fixed_delta_time as f32
    }

    /// Unity clamps this to a minimum of 0.0001 s.
    pub fn set_fixed_delta_time(&mut self, dt: f32) {
        self.fixed_delta_time = (dt as f64).max(0.0001);
    }

    pub fn maximum_delta_time(&self) -> f32 {
        self.maximum_delta_time as f32
    }

    pub fn set_maximum_delta_time(&mut self, dt: f32) {
        self.maximum_delta_time = (dt as f64).max(self.fixed_delta_time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(time: &mut Time, frames: usize, dt: f64) -> usize {
        let mut fixed = 0;
        for _ in 0..frames {
            time.begin_frame(dt);
            while time.next_fixed_step() {
                assert_eq!(time.delta_time(), time.fixed_delta_time());
                fixed += 1;
            }
            assert!(!time.in_fixed_time_step());
        }
        fixed
    }

    #[test]
    fn fixed_steps_match_wall_time() {
        let mut t = Time::new();
        // 1 s at 60 fps -> exactly 50 FixedUpdates at the default 0.02 s.
        assert_eq!(run(&mut t, 60, 1.0 / 60.0), 50);
        assert!((t.time() - 1.0).abs() < 1e-5);
        assert_eq!(t.frame_count(), 60);
        // 1 s at 200 fps -> still 50 more.
        assert_eq!(run(&mut t, 200, 1.0 / 200.0), 50);
    }

    #[test]
    fn time_scale_and_pause() {
        let mut t = Time::new();
        t.set_time_scale(0.5);
        assert_eq!(run(&mut t, 60, 1.0 / 60.0), 25);
        assert!((t.time() - 0.5).abs() < 1e-5);
        assert!((t.unscaled_time() - 1.0).abs() < 1e-5);
        t.set_time_scale(0.0);
        assert_eq!(run(&mut t, 60, 1.0 / 60.0), 0);
        assert_eq!(t.delta_time(), 0.0);
    }

    #[test]
    fn maximum_delta_time_caps_spiral_of_death() {
        let mut t = Time::new();
        // A 5 s hitch only advances game time by maximumDeltaTime (1/3 s).
        let fixed = run(&mut t, 1, 5.0);
        assert!((t.time() - 1.0 / 3.0).abs() < 1e-5);
        assert_eq!(fixed, 16);
    }
}
