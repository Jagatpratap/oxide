//! `CarController`: drives the WheelColliders below a Rigidbody from input (or AI fields),
//! like Unity's Standard Assets vehicle: torque split, braking/reverse, speed-sensitive
//! steering, handbrake, downforce and anti-roll bars.

use oxide_core::Reflect;
use oxide_math::Vector3;
use oxide_physics::{ForceMode, Rigidbody, WheelCollider};
use oxide_scene::{Component, Ctx, GameObjectId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum DriveType {
    FrontWheelDrive,
    #[default]
    RearWheelDrive,
    AllWheelDrive,
}

#[derive(Clone, Debug, Reflect)]
pub struct CarController {
    pub drive: DriveType,
    /// Engine torque at the driven wheels (N·m, shared between them).
    pub max_motor_torque: f32,
    pub max_brake_torque: f32,
    pub handbrake_torque: f32,
    /// Steering lock at standstill (degrees).
    pub max_steer_angle: f32,
    /// Steering lock at top speed, as a fraction of `max_steer_angle`.
    #[reflect(range(0, 1))]
    pub high_speed_steer: f32,
    /// Degrees per second the wheels turn towards the target angle.
    pub steer_speed: f32,
    pub top_speed_kmh: f32,
    pub reverse_speed_kmh: f32,
    /// Downward force per m/s of speed (N).
    pub downforce: f32,
    /// Anti-roll bar stiffness (N per unit of compression difference).
    pub anti_roll: f32,
    /// Read Vertical/Horizontal/Handbrake input; turn off for AI or network cars.
    pub use_input: bool,
    /// -1..1 inputs (set by AI when `use_input` is false).
    pub throttle_input: f32,
    pub steer_input: f32,
    pub handbrake_input: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub wheels: Vec<(GameObjectId, bool, bool)>, // (wheel, front, left)
    #[reflect(skip)]
    #[doc(hidden)]
    pub steer: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub speed_kmh: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub reversing: bool,
}

impl Default for CarController {
    fn default() -> Self {
        Self {
            drive: DriveType::RearWheelDrive,
            max_motor_torque: 2200.0,
            max_brake_torque: 6000.0,
            handbrake_torque: 9000.0,
            max_steer_angle: 32.0,
            high_speed_steer: 0.35,
            steer_speed: 120.0,
            top_speed_kmh: 200.0,
            reverse_speed_kmh: 35.0,
            downforce: 40.0,
            anti_roll: 9000.0,
            use_input: true,
            throttle_input: 0.0,
            steer_input: 0.0,
            handbrake_input: false,
            wheels: Vec::new(),
            steer: 0.0,
            speed_kmh: 0.0,
            reversing: false,
        }
    }
}

impl CarController {
    /// Signed speed along the car's forward axis in km/h.
    pub fn speed_kmh(&self) -> f32 {
        self.speed_kmh
    }

    pub fn is_reversing(&self) -> bool {
        self.reversing
    }

    fn find_wheels(&mut self, ctx: &mut Ctx) {
        let me = ctx.this;
        self.wheels = ctx
            .world
            .descendants(me)
            .into_iter()
            .filter(|&d| ctx.world.has_component::<WheelCollider>(d))
            .map(|d| {
                let local = ctx.world.inverse_transform_point(me, ctx.world.position(d));
                (d, local.z > 0.0, local.x < 0.0)
            })
            .collect();
    }
}

impl Component for CarController {
    fn start(&mut self, ctx: &mut Ctx) {
        self.find_wheels(ctx);
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if self.use_input {
            let input = ctx.input();
            let v =
                input.get_axis("Vertical") + input.get_axis("Throttle") - input.get_axis("Brake");
            self.throttle_input = v.clamp(-1.0, 1.0);
            self.steer_input = input.get_axis("Horizontal").clamp(-1.0, 1.0);
            self.handbrake_input = input.get_button("Handbrake");
        }
    }

    fn fixed_update(&mut self, ctx: &mut Ctx) {
        if self.wheels.is_empty() {
            self.find_wheels(ctx);
        }
        let dt = ctx.delta_time();
        let me = ctx.this;
        let fwd = ctx.world.forward(me);
        let up = ctx.world.up(me);
        let vel = ctx
            .get_component::<Rigidbody>()
            .map_or(Vector3::ZERO, |r| r.velocity());
        let v_fwd = vel.dot(fwd);
        self.speed_kmh = v_fwd * 3.6;

        // Throttle forward / brake, and reverse once (nearly) stopped while braking.
        let t = self.throttle_input;
        let mut motor = 0.0;
        let mut brake = 0.0;
        if t > 0.01 {
            if v_fwd < -0.5 {
                brake = t * self.max_brake_torque;
            } else {
                self.reversing = false;
                motor = t * self.max_motor_torque;
            }
        } else if t < -0.01 {
            if v_fwd > 0.5 && !self.reversing {
                brake = -t * self.max_brake_torque;
            } else {
                self.reversing = true;
                motor = t * self.max_motor_torque * 0.6;
            }
        } else if v_fwd.abs() < 1.5 {
            // Parked: hold the car on slopes.
            brake = self.max_brake_torque;
        } else {
            // Engine braking / rolling resistance.
            brake = self.max_brake_torque * 0.05;
        }

        let speed = v_fwd.abs() * 3.6;
        if (motor > 0.0 && speed > self.top_speed_kmh)
            || (motor < 0.0 && speed > self.reverse_speed_kmh)
        {
            motor = 0.0;
        }

        // Speed-sensitive steering, smoothed.
        let speed_factor = (speed / self.top_speed_kmh.max(1.0)).clamp(0.0, 1.0);
        let lock = self.max_steer_angle * (1.0 - speed_factor * (1.0 - self.high_speed_steer));
        let target = self.steer_input * lock;
        let step = self.steer_speed * dt;
        self.steer += (target - self.steer).clamp(-step, step);

        let driven = |front: bool| match self.drive {
            DriveType::FrontWheelDrive => front,
            DriveType::RearWheelDrive => !front,
            DriveType::AllWheelDrive => true,
        };
        let n_driven = self.wheels.iter().filter(|w| driven(w.1)).count().max(1) as f32;
        let mut compression: Vec<(bool, bool, GameObjectId, f32)> = Vec::new();
        for &(w, front, left) in &self.wheels {
            let Some(wc) = ctx.world.get_component_mut::<WheelCollider>(w) else {
                continue;
            };
            wc.steer_angle = if front { self.steer } else { 0.0 };
            wc.motor_torque = if driven(front) { motor / n_driven } else { 0.0 };
            wc.brake_torque = brake;
            if self.handbrake_input && !front {
                wc.brake_torque = self.handbrake_torque;
                wc.motor_torque = 0.0;
            }
            compression.push((
                front,
                left,
                w,
                if wc.is_grounded() {
                    wc.suspension_compression()
                } else {
                    0.0
                },
            ));
        }

        let Some(rb) = ctx.get_component_mut::<Rigidbody>() else {
            return;
        };
        rb.add_force(-up * self.downforce * v_fwd.abs(), ForceMode::Force);
        // Anti-roll bars: push down on the compressed side, lift the other.
        for front in [true, false] {
            let l = compression.iter().find(|c| c.0 == front && c.1);
            let r = compression.iter().find(|c| c.0 == front && !c.1);
            if let (Some(l), Some(r)) = (l, r) {
                // Extension difference (Unity's AntiRollBar): the bar pushes the
                // extended side down and the compressed side up, resisting body roll.
                let f = (r.3 - l.3) * self.anti_roll;
                let lp = ctx.world.position(l.2);
                let rp = ctx.world.position(r.2);
                let rb = ctx
                    .world
                    .get_component_mut::<Rigidbody>(me)
                    .expect("rigidbody");
                if f != 0.0 {
                    rb.add_force_at_position(-up * f, lp, ForceMode::Force);
                    rb.add_force_at_position(up * f, rp, ForceMode::Force);
                }
            }
        }
    }
}
