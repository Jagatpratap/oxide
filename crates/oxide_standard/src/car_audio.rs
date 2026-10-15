//! `CarAudio`: engine loop pitched by simulated gears and speed, louder with throttle, and a
//! tire-skid loop that fades in with wheel slip.

use crate::car::CarController;
use oxide_audio::AudioSource;
use oxide_core::Reflect;
use oxide_physics::WheelCollider;
use oxide_scene::{Component, Ctx, GameObjectId};

#[derive(Clone, Debug, Reflect)]
pub struct CarAudio {
    pub engine_clip: String,
    pub skid_clip: String,
    pub min_pitch: f32,
    pub max_pitch: f32,
    pub gears: u32,
    #[reflect(range(0, 1))]
    pub idle_volume: f32,
    /// Sideways slip (m/s) at which the skid sound is at full volume.
    pub full_skid_slip: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub skid: Option<GameObjectId>,
}

impl Default for CarAudio {
    fn default() -> Self {
        Self {
            engine_clip: "builtin:engine".into(),
            skid_clip: "builtin:skid".into(),
            min_pitch: 0.7,
            max_pitch: 2.6,
            gears: 5,
            idle_volume: 0.35,
            full_skid_slip: 6.0,
            skid: None,
        }
    }
}

impl Component for CarAudio {
    fn start(&mut self, ctx: &mut Ctx) {
        if ctx.get_component::<AudioSource>().is_none() {
            let mut src = AudioSource::new(&self.engine_clip);
            src.looping = true;
            src.volume = self.idle_volume;
            src.min_distance = 3.0;
            ctx.add_component(src);
        }
        let skid = ctx.world.create("Skid Audio");
        let _ = ctx.world.set_parent(skid, Some(ctx.this), false);
        let mut s = AudioSource::new(&self.skid_clip);
        s.looping = true;
        s.volume = 0.0;
        s.min_distance = 3.0;
        ctx.world.add_component(skid, s);
        self.skid = Some(skid);
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let (speed, throttle, top) = ctx
            .get_component::<CarController>()
            .map_or((0.0, 0.0, 200.0), |c| {
                (c.speed_kmh().abs(), c.throttle_input.abs(), c.top_speed_kmh)
            });
        // Simulated gearbox: rpm climbs within each gear's speed band.
        let norm = (speed / top.max(1.0)).clamp(0.0, 1.0);
        let g = self.gears.max(1) as f32;
        let gear = (norm * g).floor().min(g - 1.0);
        let in_gear = norm * g - gear;
        let rpm = (0.25 + 0.75 * in_gear) * (0.7 + 0.3 * (gear + 1.0) / g);
        let pitch = self.min_pitch + (self.max_pitch - self.min_pitch) * rpm.max(norm * 0.5);
        if let Some(src) = ctx.get_component_mut::<AudioSource>() {
            src.pitch = pitch;
            src.volume = self.idle_volume
                + (1.0 - self.idle_volume) * (0.4 * throttle + 0.6 * norm).min(1.0);
        }
        let me = ctx.this;
        let slip = ctx
            .world
            .descendants(me)
            .into_iter()
            .filter_map(|d| {
                ctx.world
                    .get_component::<WheelCollider>(d)
                    .map(|w| w.slip().1)
            })
            .fold(0.0f32, f32::max);
        if let Some(skid) = self.skid
            && let Some(src) = ctx.world.get_component_mut::<AudioSource>(skid)
        {
            let v = ((slip - 1.5) / self.full_skid_slip).clamp(0.0, 1.0);
            src.volume = v * 0.8;
            src.pitch = 0.9 + v * 0.2;
        }
    }
}
