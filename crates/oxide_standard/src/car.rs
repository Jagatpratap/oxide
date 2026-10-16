//! `CarController`: drives the WheelColliders below a Rigidbody from input (or AI fields),
//! like Unity's Standard Assets vehicle: an engine with a torque curve by RPM, a gearbox
//! (automatic or manual), traction control, braking/reverse, steering angle by speed,
//! handbrake, downforce and anti-roll bars.

use oxide_core::{AnimationCurve, Reflect};
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
    /// Engine torque (N·m) by engine RPM.
    #[reflect(tooltip = "Engine torque (N·m) by engine RPM")]
    pub torque_curve: AnimationCurve,
    pub idle_rpm: f32,
    /// Rev limiter: no torque above this.
    pub max_rpm: f32,
    /// Forward gear ratios, first gear first.
    pub gear_ratios: Vec<f32>,
    pub reverse_ratio: f32,
    /// Differential ratio, applied in every gear.
    pub final_drive_ratio: f32,
    /// Shift up and down by RPM; off = call `shift_up` / `shift_down` (or the "Shift Up" and
    /// "Shift Down" buttons).
    pub automatic: bool,
    pub shift_up_rpm: f32,
    pub shift_down_rpm: f32,
    /// Seconds without drive while changing gear.
    pub shift_time: f32,
    /// 0 = off, 1 = cuts all torque to a wheel spinning past peak grip.
    #[reflect(range(0, 1))]
    pub traction_control: f32,
    pub max_brake_torque: f32,
    pub handbrake_torque: f32,
    /// Steering lock (degrees) by speed (km/h).
    #[reflect(tooltip = "Steering lock (degrees) by speed (km/h)")]
    pub steer_angle_by_speed: AnimationCurve,
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
    #[reflect(skip)]
    #[doc(hidden)]
    pub engine_rpm: f32,
    /// -1 reverse, 1.. forward gears.
    #[reflect(skip)]
    #[doc(hidden)]
    pub gear: i32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub shift_timer: f32,
    /// Traction control's torque scale per wheel (same order as `wheels`).
    #[reflect(skip)]
    #[doc(hidden)]
    pub tc_scale: Vec<f32>,
}

impl Default for CarController {
    fn default() -> Self {
        Self {
            drive: DriveType::RearWheelDrive,
            // A 3-litre petrol engine: strong mid-range, falling off near the limiter.
            torque_curve: AnimationCurve::smooth(&[
                (800.0, 200.0),
                (2500.0, 300.0),
                (4500.0, 340.0),
                (6000.0, 310.0),
                (7000.0, 250.0),
            ]),
            idle_rpm: 900.0,
            max_rpm: 7000.0,
            gear_ratios: vec![3.2, 2.1, 1.5, 1.15, 0.92, 0.78],
            reverse_ratio: 3.3,
            final_drive_ratio: 3.4,
            automatic: true,
            shift_up_rpm: 6300.0,
            shift_down_rpm: 3000.0,
            shift_time: 0.2,
            traction_control: 0.8,
            max_brake_torque: 6000.0,
            handbrake_torque: 9000.0,
            steer_angle_by_speed: AnimationCurve::smooth(&[
                (0.0, 32.0),
                (80.0, 20.0),
                (200.0, 10.0),
            ]),
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
            engine_rpm: 900.0,
            gear: 1,
            shift_timer: 0.0,
            tc_scale: Vec::new(),
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

    /// Engine speed in RPM.
    pub fn engine_rpm(&self) -> f32 {
        self.engine_rpm
    }

    /// Current gear: -1 reverse, 1 and up forward.
    pub fn current_gear(&self) -> i32 {
        self.gear
    }

    /// Manual gearbox: next gear up (no-op in top gear).
    pub fn shift_up(&mut self) {
        let top = self.gear_ratios.len().max(1) as i32;
        if self.gear < top && !self.reversing {
            self.gear = (self.gear + 1).max(1);
            self.shift_timer = self.shift_time;
        }
    }

    /// Manual gearbox: next gear down (no-op in first).
    pub fn shift_down(&mut self) {
        if self.gear > 1 {
            self.gear -= 1;
            self.shift_timer = self.shift_time;
        }
    }

    fn ratio(&self) -> f32 {
        let r = if self.gear < 0 {
            self.reverse_ratio
        } else {
            let i = (self.gear.max(1) - 1) as usize;
            self.gear_ratios
                .get(i)
                .or(self.gear_ratios.last())
                .copied()
                .unwrap_or(1.0)
        };
        r * self.final_drive_ratio
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
            if !self.automatic {
                if input.get_button_down("Shift Up") {
                    self.shift_up();
                }
                if input.get_button_down("Shift Down") {
                    self.shift_down();
                }
            }
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

        let driven = |front: bool| match self.drive {
            DriveType::FrontWheelDrive => front,
            DriveType::RearWheelDrive => !front,
            DriveType::AllWheelDrive => true,
        };

        // Throttle forward / brake, and reverse once (nearly) stopped while braking.
        let t = self.throttle_input;
        let mut drive = 0.0;
        let mut brake = 0.0;
        if t > 0.01 {
            if v_fwd < -0.5 {
                brake = t * self.max_brake_torque;
            } else {
                self.reversing = false;
                drive = t;
            }
        } else if t < -0.01 {
            if v_fwd > 0.5 && !self.reversing {
                brake = -t * self.max_brake_torque;
            } else {
                self.reversing = true;
                drive = -t;
            }
        } else if v_fwd.abs() < 1.5 {
            // Parked: hold the car on slopes.
            brake = self.max_brake_torque;
        } else {
            // Engine braking / rolling resistance.
            brake = self.max_brake_torque * 0.05;
        }
        if self.reversing {
            self.gear = -1;
        } else if self.gear < 1 {
            self.gear = 1;
        }

        // Engine speed from the driven wheels through the gearbox; the clutch slips
        // (engine above wheel speed) when pulling away.
        let driven_rpm: Vec<f32> = self
            .wheels
            .iter()
            .filter(|w| driven(w.1))
            .filter_map(|w| {
                ctx.world
                    .get_component::<WheelCollider>(w.0)
                    .map(|c| c.rpm().abs())
            })
            .collect();
        let wheel_rpm = driven_rpm.iter().sum::<f32>() / driven_rpm.len().max(1) as f32;
        let gearbox_rpm = wheel_rpm * self.ratio();
        let launch_rpm = self.idle_rpm + drive * (self.idle_rpm * 2.5);
        let target_rpm = gearbox_rpm.max(launch_rpm).max(self.idle_rpm);
        // The flywheel follows quickly but not instantly.
        self.engine_rpm += (target_rpm - self.engine_rpm) * (dt * 12.0).min(1.0);

        // Automatic gearbox, shifting on road speed (like a real gearbox's output shaft)
        // so wheelspin doesn't trigger upshifts.
        self.shift_timer = (self.shift_timer - dt).max(0.0);
        if self.automatic && self.gear >= 1 && self.shift_timer <= 0.0 {
            let radius = self
                .wheels
                .iter()
                .filter(|w| driven(w.1))
                .filter_map(|w| {
                    ctx.world
                        .get_component::<WheelCollider>(w.0)
                        .map(|c| c.radius)
                })
                .next()
                .unwrap_or(0.35)
                .max(0.05);
            let road_wheel_rpm = v_fwd.abs() / radius / std::f32::consts::TAU * 60.0;
            let top = self.gear_ratios.len().max(1) as i32;
            let rpm_in = |gear: i32| {
                road_wheel_rpm
                    * self
                        .gear_ratios
                        .get((gear - 1) as usize)
                        .copied()
                        .unwrap_or(1.0)
                    * self.final_drive_ratio
            };
            // Up at the shift point, or at the limiter once moving (wheelspin in a low gear).
            let limiter =
                self.engine_rpm >= self.max_rpm * 0.98 && rpm_in(self.gear) > self.shift_down_rpm;
            if (rpm_in(self.gear) > self.shift_up_rpm || limiter) && self.gear < top {
                self.gear += 1;
                self.shift_timer = self.shift_time;
            } else if self.gear > 1
                && rpm_in(self.gear) < self.shift_down_rpm
                // Only if the lower gear wouldn't hit the shift-up point straight away.
                && rpm_in(self.gear - 1) < self.shift_up_rpm * 0.9
            {
                self.gear -= 1;
                self.shift_timer = self.shift_time;
            }
        }

        let engine_torque = if self.engine_rpm >= self.max_rpm || self.shift_timer > 0.0 {
            0.0
        } else {
            self.torque_curve.evaluate(self.engine_rpm).max(0.0)
        };
        let mut motor = drive * engine_torque * self.ratio();
        if self.reversing {
            motor = -motor;
        }

        let speed = v_fwd.abs() * 3.6;
        if (motor > 0.0 && speed > self.top_speed_kmh)
            || (motor < 0.0 && speed > self.reverse_speed_kmh)
        {
            motor = 0.0;
        }

        // Steering lock by speed, smoothed.
        let lock = self.steer_angle_by_speed.evaluate(speed).max(0.0);
        let target = self.steer_input * lock;
        let step = self.steer_speed * dt;
        self.steer += (target - self.steer).clamp(-step, step);

        let n_driven = self.wheels.iter().filter(|w| driven(w.1)).count().max(1) as f32;
        let mut compression: Vec<(bool, bool, GameObjectId, f32)> = Vec::new();
        self.tc_scale.resize(self.wheels.len(), 1.0);
        for (i, &(w, front, left)) in self.wheels.iter().enumerate() {
            let Some(wc) = ctx.world.get_component_mut::<WheelCollider>(w) else {
                continue;
            };
            wc.steer_angle = if front { self.steer } else { 0.0 };
            wc.motor_torque = if driven(front) { motor / n_driven } else { 0.0 };
            // Traction control: winds a wheel's torque down while it slips past peak grip
            // and lets it back up gradually, holding slip near the peak.
            let (slip, _) = wc.slip();
            let peak = wc.forward_friction.extremum_slip.max(1e-3);
            let scale = &mut self.tc_scale[i];
            if slip * wc.motor_torque > 0.0 && slip.abs() > peak {
                let over = ((slip.abs() - peak) / peak).min(2.0);
                *scale -= over * 6.0 * dt;
            } else {
                *scale += 2.0 * dt;
            }
            *scale = scale.clamp(1.0 - self.traction_control, 1.0);
            wc.motor_torque *= *scale;
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

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_core::Time;
    use oxide_math::Quaternion;
    use oxide_physics::{BoxCollider, WheelCollider};
    use oxide_scene::{Phase, System, Timers, World};

    struct Sim {
        world: World,
        sys: Box<dyn System>,
    }

    impl Sim {
        fn new() -> Self {
            let mut world = World::new();
            oxide_scene::register_builtin(&mut world.registry);
            world.resources.insert(Time::new());
            world.resources.insert(Timers::default());
            world.resources.insert(oxide_input::Input::new());
            let sys = oxide_physics::install(&mut world);
            let g = world.create("Ground");
            world.transform_mut(g).unwrap().local_scale = Vector3::new(4000.0, 1.0, 4000.0);
            world.transform_mut(g).unwrap().local_position = Vector3::new(0.0, -0.5, 0.0);
            world.add_component(g, BoxCollider::default());
            Self { world, sys }
        }

        fn step(&mut self, seconds: f32) {
            for _ in 0..(seconds * 60.0).round() as u32 {
                let w = &mut self.world;
                w.resource_mut::<Time>().begin_frame(1.0 / 60.0);
                w.run_start();
                while w.resource_mut::<Time>().next_fixed_step() {
                    w.run_phase(Phase::FixedUpdate);
                    self.sys.fixed_update(w);
                }
                w.run_phase(Phase::Update);
                self.sys.update(w);
                w.run_phase(Phase::LateUpdate);
            }
        }

        /// The vehicle sample's car (1300 kg, 0.36 m wheels), without visuals.
        fn car(&mut self) -> GameObjectId {
            let w = &mut self.world;
            let car = w.create_at("Car", Vector3::new(0.0, 0.6, 0.0), Quaternion::IDENTITY);
            w.add_component(
                car,
                Rigidbody {
                    mass: 1300.0,
                    center_of_mass: Vector3::new(0.0, 0.25, 0.1),
                    angular_damping: 0.3,
                    ..Default::default()
                },
            );
            w.add_component(
                car,
                BoxCollider {
                    center: Vector3::new(0.0, 0.65, 0.0),
                    size: Vector3::new(1.8, 0.6, 4.3),
                    ..Default::default()
                },
            );
            w.add_component(
                car,
                CarController {
                    use_input: false,
                    ..Default::default()
                },
            );
            for (x, z) in [(-0.8, 1.35), (0.8, 1.35), (-0.8, -1.35), (0.8, -1.35)] {
                let wheel = w.create("Wheel");
                w.set_parent(wheel, Some(car), false).unwrap();
                w.transform_mut(wheel).unwrap().local_position = Vector3::new(x, 0.55, z);
                w.add_component(
                    wheel,
                    WheelCollider {
                        radius: 0.36,
                        suspension_distance: 0.25,
                        ..Default::default()
                    },
                );
            }
            car
        }
    }

    #[test]
    fn engine_and_gearbox_accelerate_through_the_gears() {
        let mut s = Sim::new();
        let car = s.car();
        s.step(1.0);
        s.world
            .get_component_mut::<CarController>(car)
            .unwrap()
            .throttle_input = 1.0;
        let mut top_gear = 1;
        let mut t100 = None;
        for i in 1..=30 {
            s.step(0.5);
            let c = s.world.get_component::<CarController>(car).unwrap().clone();
            let slips: Vec<String> = c
                .wheels
                .iter()
                .filter(|w| !w.1)
                .map(|w| {
                    let wc = s.world.get_component::<WheelCollider>(w.0).unwrap();
                    format!("{:.2}/{:.0}", wc.slip().0, wc.motor_torque)
                })
                .collect();
            eprintln!(
                "t={:4.1} {:5.1} km/h gear {} {:5.0} rpm slip/torque {slips:?}",
                i as f32 * 0.5,
                c.speed_kmh(),
                c.current_gear(),
                c.engine_rpm()
            );
            top_gear = top_gear.max(c.current_gear());
            if t100.is_none() && c.speed_kmh() >= 100.0 {
                t100 = Some(i as f32 * 0.5);
            }
        }
        let t100 = t100.expect("reaches 100 km/h");
        assert!(t100 > 4.0 && t100 < 12.0, "0-100 km/h in {t100} s");
        assert!(top_gear >= 4, "shifted up to gear {top_gear}");
    }
}
