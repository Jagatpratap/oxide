//! Camera rigs: `FollowCamera` (chase cam for vehicles), `OrbitCamera` (mouse orbit around a
//! target, third-person shooter style) and `FlyCamera` (free WASD + mouse, like the Scene view).

use oxide_core::Reflect;
use oxide_input::{CursorLockMode, KeyCode, MouseButton};
use oxide_math::{Mathf, Quaternion, QuaternionExt, Vector3};
use oxide_physics::DEFAULT_RAYCAST_LAYERS;
use oxide_scene::{Component, Ctx, GameObjectId};

/// Chase camera that trails `target` at `distance`/`height`, smoothing position and yaw.
#[derive(Clone, Debug, Reflect)]
pub struct FollowCamera {
    pub target: Option<GameObjectId>,
    pub distance: f32,
    pub height: f32,
    /// Point above the target to look at.
    pub look_height: f32,
    /// Higher = snappier.
    pub position_damping: f32,
    pub rotation_damping: f32,
    /// Keeps the camera out of walls with a sphere cast.
    pub avoid_obstacles: bool,
}

impl Default for FollowCamera {
    fn default() -> Self {
        Self {
            target: None,
            distance: 7.0,
            height: 2.5,
            look_height: 1.0,
            position_damping: 6.0,
            rotation_damping: 4.0,
            avoid_obstacles: true,
        }
    }
}

impl Component for FollowCamera {
    fn late_update(&mut self, ctx: &mut Ctx) {
        let Some(t) = self.target.filter(|&t| ctx.world.is_alive(t)) else {
            return;
        };
        let dt = ctx.delta_time();
        let tp = ctx.world.position(t);
        let tf = ctx.world.forward(t);
        let flat = Vector3::new(tf.x, 0.0, tf.z).normalize_or(Vector3::Z);
        let me = ctx.this;
        let cur = ctx.world.position(me);
        // Damp the yaw of the offset direction separately for a smooth swing.
        let cur_dir = Vector3::new(tp.x - cur.x, 0.0, tp.z - cur.z).normalize_or(flat);
        let a = 1.0 - (-self.rotation_damping * dt).exp();
        let dir = cur_dir.lerp(flat, a).normalize_or(flat);
        let mut want = tp - dir * self.distance + Vector3::Y * self.height;
        let focus = tp + Vector3::Y * self.look_height;
        if self.avoid_obstacles
            && let Some(phys) = ctx.world.try_resource::<oxide_physics::Physics>()
        {
            let to = want - focus;
            let len = to.length();
            if let Some(hit) = phys.sphere_cast(focus, 0.3, to, len, DEFAULT_RAYCAST_LAYERS)
                && hit.rigidbody != Some(t)
                && Some(hit.game_object) != Some(t)
            {
                want = focus + to.normalize() * (hit.distance - 0.2).max(0.5);
            }
        }
        let b = 1.0 - (-self.position_damping * dt).exp();
        let p = cur.lerp(want, b);
        ctx.world.set_position(me, p);
        ctx.world.look_at(me, focus, Vector3::Y);
    }
}

/// Mouse-orbit camera around a target (third-person shooter / editor orbit).
#[derive(Clone, Debug, Reflect)]
pub struct OrbitCamera {
    pub target: Option<GameObjectId>,
    pub distance: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    /// Offset from the target pivot (e.g. over the right shoulder: [0.6, 1.6, 0]).
    pub pivot_offset: Vector3,
    pub sensitivity: f32,
    pub min_pitch: f32,
    pub max_pitch: f32,
    /// Lock and hide the cursor (click to capture, Escape to release).
    pub lock_cursor: bool,
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: None,
            distance: 4.0,
            min_distance: 1.0,
            max_distance: 12.0,
            pivot_offset: Vector3::new(0.5, 1.6, 0.0),
            sensitivity: 2.0,
            min_pitch: -40.0,
            max_pitch: 75.0,
            lock_cursor: true,
            yaw: 0.0,
            pitch: 15.0,
        }
    }
}

pub(crate) fn handle_cursor_lock(ctx: &mut Ctx, lock: bool) -> bool {
    let input = ctx.world.resource_mut::<oxide_input::Input>();
    if lock {
        if input.get_mouse_button_down(MouseButton::Left)
            && input.cursor_lock == CursorLockMode::None
        {
            input.cursor_lock = CursorLockMode::Locked;
            input.cursor_visible = false;
        }
        if input.get_key_down(KeyCode::Escape) {
            input.cursor_lock = CursorLockMode::None;
            input.cursor_visible = true;
        }
    }
    !lock || input.cursor_lock != CursorLockMode::None
}

impl Component for OrbitCamera {
    fn late_update(&mut self, ctx: &mut Ctx) {
        let Some(t) = self.target.filter(|&t| ctx.world.is_alive(t)) else {
            return;
        };
        if handle_cursor_lock(ctx, self.lock_cursor) {
            let input = ctx.input();
            let look_x = input.get_axis("Mouse X") + input.get_axis("Look X") * 2.0;
            let look_y = input.get_axis("Mouse Y") + input.get_axis("Look Y") * 2.0;
            let scroll = input.get_axis("Mouse ScrollWheel");
            self.yaw += look_x * self.sensitivity;
            self.pitch =
                (self.pitch - look_y * self.sensitivity).clamp(self.min_pitch, self.max_pitch);
            self.distance =
                (self.distance - scroll * 5.0).clamp(self.min_distance, self.max_distance);
        }
        let rot = Quaternion::euler(self.pitch, self.yaw, 0.0);
        let pivot =
            ctx.world.position(t) + Quaternion::euler(0.0, self.yaw, 0.0) * self.pivot_offset;
        let mut dist = self.distance;
        if let Some(phys) = ctx.world.try_resource::<oxide_physics::Physics>()
            && let Some(hit) = phys.sphere_cast(
                pivot,
                0.2,
                rot * Vector3::NEG_Z,
                self.distance,
                DEFAULT_RAYCAST_LAYERS,
            )
            && hit.rigidbody != Some(t)
            && hit.game_object != t
        {
            dist = (hit.distance - 0.1).max(0.3);
        }
        let me = ctx.this;
        ctx.world.set_position(me, pivot - rot * Vector3::Z * dist);
        ctx.world.set_rotation(me, rot);
    }
}

/// Free-flying camera: hold the right mouse button to look, WASD/QE to move, Shift to speed up.
#[derive(Clone, Debug, Reflect)]
pub struct FlyCamera {
    pub speed: f32,
    pub sensitivity: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub yaw: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub pitch: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub init: bool,
}

impl Default for FlyCamera {
    fn default() -> Self {
        Self {
            speed: 10.0,
            sensitivity: 2.0,
            yaw: 0.0,
            pitch: 0.0,
            init: false,
        }
    }
}

impl Component for FlyCamera {
    fn update(&mut self, ctx: &mut Ctx) {
        let me = ctx.this;
        if !self.init {
            let e = ctx.world.euler_angles(me);
            self.pitch = Mathf::delta_angle(0.0, e.x);
            self.yaw = e.y;
            self.init = true;
        }
        let dt = ctx.time().unscaled_delta_time();
        let input = ctx.input();
        if input.get_mouse_button(MouseButton::Right) {
            self.yaw += input.get_axis("Mouse X") * self.sensitivity;
            self.pitch =
                (self.pitch - input.get_axis("Mouse Y") * self.sensitivity).clamp(-89.0, 89.0);
        }
        let mut mv = Vector3::new(
            input.get_axis_raw("Horizontal"),
            0.0,
            input.get_axis_raw("Vertical"),
        );
        if input.get_key(KeyCode::E) {
            mv.y += 1.0;
        }
        if input.get_key(KeyCode::Q) {
            mv.y -= 1.0;
        }
        let fast = if input.get_key(KeyCode::LeftShift) {
            4.0
        } else {
            1.0
        };
        let rot = Quaternion::euler(self.pitch, self.yaw, 0.0);
        ctx.world.set_rotation(me, rot);
        let p = ctx.world.position(me);
        ctx.world
            .set_position(me, p + rot * mv * self.speed * fast * dt);
    }
}
