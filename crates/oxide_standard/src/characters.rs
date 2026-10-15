//! Player controllers on top of CharacterController (Unity Starter Assets equivalents):
//! `FirstPersonController` and `ThirdPersonController`.

use crate::cameras::handle_cursor_lock;
use oxide_core::Reflect;
use oxide_math::{Mathf, Quaternion, QuaternionExt, Vector3};
use oxide_physics::{CharacterController, CharacterExt};
use oxide_scene::{Camera, Component, Ctx, GameObjectId};

/// Shared movement settings and state.
#[derive(Clone, Debug, Reflect)]
pub struct Locomotion {
    pub walk_speed: f32,
    pub sprint_speed: f32,
    pub crouch_speed: f32,
    pub jump_height: f32,
    /// Negative, m/s² (Starter Assets use -15 for snappy jumps).
    pub gravity: f32,
    /// How quickly speed changes (m/s²).
    pub acceleration: f32,
}

impl Default for Locomotion {
    fn default() -> Self {
        Self {
            walk_speed: 4.0,
            sprint_speed: 7.0,
            crouch_speed: 2.0,
            jump_height: 1.2,
            gravity: -15.0,
            acceleration: 30.0,
        }
    }
}

/// Moves with `CharacterController::move_character`; returns new (horizontal velocity, vertical velocity).
#[allow(clippy::too_many_arguments)]
fn locomote(
    ctx: &mut Ctx,
    loco: &Locomotion,
    wish: Vector3,
    horiz: Vector3,
    mut vy: f32,
    jump: bool,
    sprint: bool,
    crouch: bool,
) -> (Vector3, f32, bool) {
    let dt = ctx.delta_time();
    let grounded = ctx
        .get_component::<CharacterController>()
        .is_some_and(|c| c.is_grounded);
    let speed = if crouch {
        loco.crouch_speed
    } else if sprint {
        loco.sprint_speed
    } else {
        loco.walk_speed
    };
    let target = wish.clamp_length_max(1.0) * speed;
    let delta = target - horiz;
    let step = loco.acceleration * dt;
    let horiz = horiz + delta.clamp_length_max(step);
    if grounded && vy < 0.0 {
        vy = -2.0; // keep pressed to the ground
    }
    let mut jumped = false;
    if grounded && jump {
        vy = (loco.jump_height * -2.0 * loco.gravity).sqrt();
        jumped = true;
    }
    vy += loco.gravity * dt;
    let flags = ctx.move_character(Vector3::new(horiz.x, vy, horiz.z) * dt);
    if flags.above && vy > 0.0 {
        vy = 0.0;
    }
    (horiz, vy, jumped)
}

/// First-person player: mouse look (yaw on the body, pitch on the camera child), WASD,
/// Shift sprint, C/Ctrl crouch, Space jump.
#[derive(Clone, Debug, Reflect)]
pub struct FirstPersonController {
    pub locomotion: Locomotion,
    /// Camera object (defaults to the first child with a Camera).
    pub camera: Option<GameObjectId>,
    pub sensitivity: f32,
    pub lock_cursor: bool,
    pub eye_height: f32,
    pub crouch_eye_height: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub pitch: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub velocity: Vector3,
    #[reflect(skip)]
    #[doc(hidden)]
    pub vertical: f32,
}

impl Default for FirstPersonController {
    fn default() -> Self {
        Self {
            locomotion: Locomotion::default(),
            camera: None,
            sensitivity: 2.0,
            lock_cursor: true,
            eye_height: 1.65,
            crouch_eye_height: 1.0,
            pitch: 0.0,
            velocity: Vector3::ZERO,
            vertical: 0.0,
        }
    }
}

fn find_camera(ctx: &Ctx) -> Option<GameObjectId> {
    ctx.get_component_in_children::<Camera>().map(|(id, _)| id)
}

impl Component for FirstPersonController {
    fn start(&mut self, ctx: &mut Ctx) {
        if self.camera.is_none() {
            self.camera = find_camera(ctx);
        }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let me = ctx.this;
        let active = handle_cursor_lock(ctx, self.lock_cursor);
        let input = ctx.input();
        let (mx, my) = if active {
            (
                input.get_axis("Mouse X") + input.get_axis("Look X") * 2.0,
                input.get_axis("Mouse Y") + input.get_axis("Look Y") * 2.0,
            )
        } else {
            (0.0, 0.0)
        };
        let wish_local = Vector3::new(
            input.get_axis_raw("Horizontal"),
            0.0,
            input.get_axis_raw("Vertical"),
        );
        let (jump, sprint, crouch) = (
            input.get_button_down("Jump"),
            input.get_button("Sprint"),
            input.get_button("Crouch"),
        );
        ctx.world.rotate(
            me,
            Vector3::new(0.0, mx * self.sensitivity, 0.0),
            oxide_scene::Space::World,
        );
        self.pitch = (self.pitch - my * self.sensitivity).clamp(-89.0, 89.0);
        if let Some(cam) = self.camera {
            let eye = if crouch {
                self.crouch_eye_height
            } else {
                self.eye_height
            };
            if let Some(t) = ctx.world.transform_mut(cam) {
                t.local_rotation = Quaternion::euler(self.pitch, 0.0, 0.0);
                let y = t.local_position.y;
                t.local_position.y = Mathf::lerp(y, eye, 0.2);
            }
        }
        let wish = ctx.world.rotation(me) * wish_local;
        let (h, v, _) = locomote(
            ctx,
            &self.locomotion,
            Vector3::new(wish.x, 0.0, wish.z),
            self.velocity,
            self.vertical,
            jump,
            sprint,
            crouch,
        );
        self.velocity = h;
        self.vertical = v;
    }
}

/// Third-person player: camera-relative movement, the body turns towards where it moves;
/// while aiming (Fire2 / right mouse) it faces the camera direction and strafes, like
/// BGMI/PUBG. Pair with an `OrbitCamera` targeting this object.
#[derive(Clone, Debug, Reflect)]
pub struct ThirdPersonController {
    pub locomotion: Locomotion,
    /// The camera used for movement directions (defaults to the main camera).
    pub camera: Option<GameObjectId>,
    /// Degrees per second the body turns.
    pub turn_speed: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub velocity: Vector3,
    #[reflect(skip)]
    #[doc(hidden)]
    pub vertical: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub aiming: bool,
}

impl Default for ThirdPersonController {
    fn default() -> Self {
        Self {
            locomotion: Locomotion::default(),
            camera: None,
            turn_speed: 720.0,
            velocity: Vector3::ZERO,
            vertical: 0.0,
            aiming: false,
        }
    }
}

impl ThirdPersonController {
    /// Horizontal speed (m/s) — feed this to an Animator's "Speed" parameter.
    pub fn speed(&self) -> f32 {
        self.velocity.length()
    }
    pub fn is_aiming(&self) -> bool {
        self.aiming
    }
    pub fn vertical_velocity(&self) -> f32 {
        self.vertical
    }
}

impl Component for ThirdPersonController {
    fn start(&mut self, ctx: &mut Ctx) {
        if self.camera.is_none() {
            self.camera = ctx.find_with_tag("MainCamera").first().copied();
        }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let me = ctx.this;
        let input = ctx.input();
        let raw = Vector3::new(
            input.get_axis_raw("Horizontal"),
            0.0,
            input.get_axis_raw("Vertical"),
        );
        let (jump, sprint, crouch) = (
            input.get_button_down("Jump"),
            input.get_button("Sprint"),
            input.get_button("Crouch"),
        );
        self.aiming = input.get_button("Fire2");
        let cam_yaw = self
            .camera
            .filter(|&c| ctx.world.is_alive(c))
            .map_or(0.0, |c| ctx.world.euler_angles(c).y);
        let wish = Quaternion::euler(0.0, cam_yaw, 0.0) * raw;
        let dt = ctx.delta_time();
        let face = if self.aiming {
            Some(cam_yaw)
        } else if wish.length_squared() > 0.01 {
            Some(wish.x.atan2(wish.z).to_degrees())
        } else {
            None
        };
        if let Some(target) = face {
            let cur = ctx.world.euler_angles(me).y;
            let y = Mathf::move_towards_angle(cur, target, self.turn_speed * dt);
            ctx.world.set_rotation(me, Quaternion::euler(0.0, y, 0.0));
        }
        let (h, v, _) = locomote(
            ctx,
            &self.locomotion,
            wish,
            self.velocity,
            self.vertical,
            jump,
            sprint && !self.aiming,
            crouch,
        );
        self.velocity = h;
        self.vertical = v;
    }
}
