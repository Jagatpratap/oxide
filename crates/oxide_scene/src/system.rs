//! Engine subsystems (physics, animation, audio, ...) hooked into Unity's player loop.
//!
//! Order per frame (Unity's execution order):
//! `FixedUpdate` scripts → [`System::fixed_update`] (physics step, collision messages) — repeated
//! per fixed step → `Update` scripts → [`System::update`] (animation) → `LateUpdate` scripts →
//! [`System::late_update`] (audio listener, cameras) → render.

use crate::world::World;

#[allow(unused_variables)]
pub trait System {
    fn name(&self) -> &'static str;
    /// Called once after the scene is set up (and on every scene load).
    fn start(&mut self, world: &mut World) {}
    /// After scripts' `fixed_update`.
    fn fixed_update(&mut self, world: &mut World) {}
    /// After scripts' `update`.
    fn update(&mut self, world: &mut World) {}
    /// After scripts' `late_update`, before rendering.
    fn late_update(&mut self, world: &mut World) {}
    /// Runs every frame even while paused or in edit mode (debug drawing, editor sync).
    fn always(&mut self, world: &mut World) {}
}

/// Unity `Debug.DrawLine` / `Debug.DrawRay`: lines drawn for one frame (or `duration`).
#[derive(Default)]
pub struct DebugDraw {
    pub lines: Vec<DebugLine>,
}

#[derive(Clone, Copy, Debug)]
pub struct DebugLine {
    pub start: oxide_math::Vector3,
    pub end: oxide_math::Vector3,
    pub color: oxide_math::Color,
    /// Game time when the line expires (0 = this frame only).
    pub until: f32,
}

impl DebugDraw {
    pub fn line(
        &mut self,
        start: oxide_math::Vector3,
        end: oxide_math::Vector3,
        color: oxide_math::Color,
    ) {
        self.lines.push(DebugLine {
            start,
            end,
            color,
            until: 0.0,
        });
    }

    pub fn ray(
        &mut self,
        start: oxide_math::Vector3,
        dir: oxide_math::Vector3,
        color: oxide_math::Color,
    ) {
        self.line(start, start + dir, color);
    }

    /// Wireframe box (Gizmos.DrawWireCube) with a rotation.
    pub fn wire_box(
        &mut self,
        center: oxide_math::Vector3,
        half: oxide_math::Vector3,
        rot: oxide_math::Quaternion,
        color: oxide_math::Color,
    ) {
        let c = |x: f32, y: f32, z: f32| center + rot * (half * oxide_math::Vector3::new(x, y, z));
        let p = [
            c(-1., -1., -1.),
            c(1., -1., -1.),
            c(1., -1., 1.),
            c(-1., -1., 1.),
            c(-1., 1., -1.),
            c(1., 1., -1.),
            c(1., 1., 1.),
            c(-1., 1., 1.),
        ];
        for i in 0..4 {
            self.line(p[i], p[(i + 1) % 4], color);
            self.line(p[i + 4], p[(i + 1) % 4 + 4], color);
            self.line(p[i], p[i + 4], color);
        }
    }

    /// Three circles approximating a sphere (Gizmos.DrawWireSphere).
    pub fn wire_sphere(&mut self, center: oxide_math::Vector3, r: f32, color: oxide_math::Color) {
        use oxide_math::Vector3 as V;
        let n = 24;
        for axis in 0..3 {
            for i in 0..n {
                let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
                let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
                let pt = |a: f32| match axis {
                    0 => V::new(0.0, a.cos(), a.sin()),
                    1 => V::new(a.cos(), 0.0, a.sin()),
                    _ => V::new(a.cos(), a.sin(), 0.0),
                } * r + center;
                self.line(pt(a0), pt(a1), color);
            }
        }
    }
}
