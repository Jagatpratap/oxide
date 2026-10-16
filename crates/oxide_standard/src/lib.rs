//! Oxide standard assets — ready-made gameplay components (Unity's Standard Assets /
//! Starter Assets equivalents) built only on the public engine API:
//!
//! - [`CarController`]: WheelCollider vehicle driving with torque split, brakes/reverse,
//!   speed-sensitive steering, handbrake, downforce and anti-roll bars.
//! - [`FollowCamera`], [`OrbitCamera`], [`FlyCamera`]: chase cam, mouse orbit, free fly.
//! - [`FirstPersonController`], [`ThirdPersonController`]: CharacterController players.
//! - [`WorldStreamer`]: loads and unloads scene cells around the player (big worlds).
//! - [`Rotator`], [`Lifetime`]: small utilities.

pub mod ai;
pub mod cameras;
pub mod car;
pub mod car_audio;
pub mod characters;
pub mod streaming;

pub use ai::{CarAIControl, WaypointCircuit};
pub use cameras::{FlyCamera, FollowCamera, OrbitCamera};
pub use car::{CarController, DriveType};
pub use car_audio::CarAudio;
pub use characters::{FirstPersonController, Locomotion, ThirdPersonController};
pub use streaming::{StreamingCell, WorldStreamer};

use oxide_core::Reflect;
use oxide_math::Vector3;
use oxide_scene::{Component, ComponentRegistry, Ctx, Space};

/// Spins its object (degrees per second, local axes).
#[derive(Clone, Debug, Default, Reflect)]
pub struct Rotator {
    pub degrees_per_second: Vector3,
}

impl Component for Rotator {
    fn update(&mut self, ctx: &mut Ctx) {
        let dt = ctx.delta_time();
        let me = ctx.this;
        ctx.world
            .rotate(me, self.degrees_per_second * dt, Space::Self_);
    }
}

/// Destroys its object after `seconds` (bullets, effects, debris).
#[derive(Clone, Debug, Reflect)]
pub struct Lifetime {
    pub seconds: f32,
}

impl Default for Lifetime {
    fn default() -> Self {
        Self { seconds: 5.0 }
    }
}

impl Component for Lifetime {
    fn update(&mut self, ctx: &mut Ctx) {
        self.seconds -= ctx.delta_time();
        if self.seconds <= 0.0 {
            ctx.destroy_self();
        }
    }
}

pub fn register(reg: &mut ComponentRegistry) {
    reg.register::<CarController>("Vehicles/Car Controller");
    reg.register::<CarAudio>("Vehicles/Car Audio");
    reg.register::<CarAIControl>("Vehicles/Car AI Control");
    reg.register::<WaypointCircuit>("Vehicles/Waypoint Circuit");
    reg.register::<FollowCamera>("Camera/Follow Camera");
    reg.register::<OrbitCamera>("Camera/Orbit Camera");
    reg.register::<FlyCamera>("Camera/Fly Camera");
    reg.register::<FirstPersonController>("Characters/First Person Controller");
    reg.register::<ThirdPersonController>("Characters/Third Person Controller");
    reg.register::<Rotator>("Utility/Rotator");
    reg.register::<Lifetime>("Utility/Lifetime");
    reg.register::<WorldStreamer>("Scene/World Streamer");
}
