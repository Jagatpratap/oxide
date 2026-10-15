//! Oxide particles: a Unity `ParticleSystem` subset — main module (duration, looping,
//! lifetime/speed/size/color ranges, gravity, simulation space, max particles), emission
//! (rate over time and distance, bursts), shapes (cone, sphere, box, point), color/size over
//! lifetime, velocity damping, noise, and a billboard renderer (additive or alpha).
//! Reference: <https://docs.unity3d.com/Manual/ParticleSystems.html>

use oxide_assets::{AssetId, AssetServer, Handle, Texture2D};
use oxide_core::{Reflect, Time};
use oxide_math::{Color, Quaternion, Vector2, Vector3};
use oxide_render::{ParticleBatch, ParticleBlend, ParticleInstance};
use oxide_scene::{Component, Ctx, GameObjectId, System, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum ParticleShape {
    #[default]
    Cone,
    Sphere,
    Hemisphere,
    Box,
    Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum SimulationSpace {
    Local,
    #[default]
    World,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum BlendMode {
    #[default]
    Alpha,
    Additive,
}

#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct Particle {
    pub position: Vector3,
    pub velocity: Vector3,
    pub age: f32,
    pub lifetime: f32,
    pub size: f32,
    pub rotation: f32,
    pub angular_velocity: f32,
    pub color: Color,
    pub seed: f32,
}

/// Unity `ParticleSystem` (Shuriken), main + emission + shape + over-lifetime modules.
#[derive(Clone, Debug, Reflect)]
pub struct ParticleSystem {
    pub duration: f32,
    pub looping: bool,
    pub play_on_awake: bool,
    /// Lifetime range (seconds).
    pub start_lifetime: Vector2,
    /// Speed range (m/s).
    pub start_speed: Vector2,
    /// Size range (m).
    pub start_size: Vector2,
    /// Random start rotation range (degrees) and angular speed range (deg/s).
    pub start_rotation: Vector2,
    pub angular_velocity: Vector2,
    pub start_color: Color,
    /// Second color: each particle picks a random color between the two.
    pub start_color_b: Color,
    pub gravity_modifier: f32,
    pub simulation_space: SimulationSpace,
    pub max_particles: u32,
    /// Emission per second.
    pub rate_over_time: f32,
    /// Emission per meter moved (trails, tire dust).
    pub rate_over_distance: f32,
    /// One burst at `burst_time` of `burst_count` particles each cycle.
    pub burst_count: u32,
    pub burst_time: f32,
    pub shape: ParticleShape,
    /// Cone half-angle in degrees.
    pub shape_angle: f32,
    pub shape_radius: f32,
    pub shape_box: Vector3,
    /// Color multiplier at the end of life (with alpha for fading out).
    pub end_color: Color,
    /// Size multiplier at the start and end of life.
    pub size_over_lifetime: Vector2,
    /// Velocity damping per second (0 = none).
    pub drag: f32,
    /// Random turbulence strength (m/s²).
    pub noise: f32,
    pub texture: Option<Handle<Texture2D>>,
    pub blend: BlendMode,
    /// HDR brightness multiplier (values > 1 bloom, e.g. fire, muzzle flashes).
    pub intensity: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub particles: Vec<Particle>,
    #[reflect(skip)]
    #[doc(hidden)]
    pub state: EmitterState,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct EmitterState {
    pub playing: bool,
    pub started: bool,
    pub time: f32,
    pub emit_accum: f32,
    pub burst_done: bool,
    pub last_position: Option<Vector3>,
    pub rng: u32,
    pub pending_emit: u32,
}

impl Default for ParticleSystem {
    fn default() -> Self {
        Self {
            duration: 5.0,
            looping: true,
            play_on_awake: true,
            start_lifetime: Vector2::new(5.0, 5.0),
            start_speed: Vector2::new(5.0, 5.0),
            start_size: Vector2::new(1.0, 1.0),
            start_rotation: Vector2::ZERO,
            angular_velocity: Vector2::ZERO,
            start_color: Color::WHITE,
            start_color_b: Color::WHITE,
            gravity_modifier: 0.0,
            simulation_space: SimulationSpace::World,
            max_particles: 1000,
            rate_over_time: 10.0,
            rate_over_distance: 0.0,
            burst_count: 0,
            burst_time: 0.0,
            shape: ParticleShape::Cone,
            shape_angle: 25.0,
            shape_radius: 1.0,
            shape_box: Vector3::ONE,
            end_color: Color::new(1.0, 1.0, 1.0, 0.0),
            size_over_lifetime: Vector2::new(1.0, 1.0),
            drag: 0.0,
            noise: 0.0,
            texture: Some(Handle::from_path("builtin:particle")),
            blend: BlendMode::Alpha,
            intensity: 1.0,
            particles: Vec::new(),
            state: EmitterState::default(),
        }
    }
}

impl ParticleSystem {
    pub fn play(&mut self) {
        self.state.playing = true;
        self.state.started = true;
        self.state.time = 0.0;
        self.state.burst_done = false;
    }
    /// Stops emitting; live particles finish their lives (Unity `Stop()`).
    pub fn stop(&mut self) {
        self.state.playing = false;
    }
    /// Stops and removes every particle.
    pub fn clear(&mut self) {
        self.state.playing = false;
        self.particles.clear();
    }
    /// Unity `Emit(count)`: spawns particles immediately.
    pub fn emit(&mut self, count: u32) {
        self.state.pending_emit += count;
    }
    pub fn is_playing(&self) -> bool {
        self.state.playing
    }
    /// Playing or still has live particles (Unity `IsAlive()`).
    pub fn is_alive(&self) -> bool {
        self.state.playing || !self.particles.is_empty()
    }
    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }

    // ---- presets ---------------------------------------------------------------------

    /// Grey smoke rising and spreading.
    pub fn smoke() -> Self {
        Self {
            start_lifetime: Vector2::new(2.5, 4.0),
            start_speed: Vector2::new(0.6, 1.4),
            start_size: Vector2::new(0.6, 1.2),
            start_rotation: Vector2::new(0.0, 360.0),
            angular_velocity: Vector2::new(-30.0, 30.0),
            start_color: Color::new(0.45, 0.45, 0.45, 0.55),
            start_color_b: Color::new(0.3, 0.3, 0.3, 0.45),
            gravity_modifier: -0.05,
            rate_over_time: 12.0,
            shape_angle: 15.0,
            shape_radius: 0.2,
            end_color: Color::new(0.6, 0.6, 0.6, 0.0),
            size_over_lifetime: Vector2::new(0.6, 2.8),
            drag: 0.4,
            noise: 0.6,
            texture: Some(Handle::from_path("builtin:smoke")),
            ..Default::default()
        }
    }

    /// Fire: short-lived bright additive particles.
    pub fn fire() -> Self {
        Self {
            start_lifetime: Vector2::new(0.5, 1.0),
            start_speed: Vector2::new(1.0, 2.5),
            start_size: Vector2::new(0.4, 0.8),
            start_rotation: Vector2::new(0.0, 360.0),
            start_color: Color::rgb(1.0, 0.6, 0.15),
            start_color_b: Color::rgb(1.0, 0.35, 0.05),
            gravity_modifier: -0.2,
            rate_over_time: 40.0,
            shape_angle: 10.0,
            shape_radius: 0.3,
            end_color: Color::new(0.8, 0.1, 0.0, 0.0),
            size_over_lifetime: Vector2::new(1.0, 0.2),
            blend: BlendMode::Additive,
            intensity: 3.0,
            noise: 1.0,
            ..Default::default()
        }
    }

    /// One-shot explosion burst (spark shell + fireball).
    pub fn explosion() -> Self {
        Self {
            duration: 1.0,
            looping: false,
            start_lifetime: Vector2::new(0.4, 1.2),
            start_speed: Vector2::new(6.0, 16.0),
            start_size: Vector2::new(0.5, 1.5),
            start_color: Color::rgb(1.0, 0.7, 0.3),
            start_color_b: Color::rgb(1.0, 0.4, 0.1),
            gravity_modifier: 0.6,
            rate_over_time: 0.0,
            burst_count: 80,
            shape: ParticleShape::Sphere,
            shape_radius: 0.5,
            end_color: Color::new(0.4, 0.05, 0.0, 0.0),
            size_over_lifetime: Vector2::new(1.0, 2.0),
            drag: 2.5,
            blend: BlendMode::Additive,
            intensity: 4.0,
            ..Default::default()
        }
    }

    /// Muzzle flash: a tiny, very bright, very short burst.
    pub fn muzzle_flash() -> Self {
        Self {
            duration: 0.1,
            looping: false,
            play_on_awake: false,
            start_lifetime: Vector2::new(0.04, 0.07),
            start_speed: Vector2::new(1.0, 3.0),
            start_size: Vector2::new(0.25, 0.45),
            start_rotation: Vector2::new(0.0, 360.0),
            start_color: Color::rgb(1.0, 0.85, 0.5),
            start_color_b: Color::rgb(1.0, 0.6, 0.2),
            rate_over_time: 0.0,
            burst_count: 6,
            shape_angle: 8.0,
            shape_radius: 0.02,
            end_color: Color::new(1.0, 0.5, 0.1, 0.0),
            blend: BlendMode::Additive,
            intensity: 8.0,
            simulation_space: SimulationSpace::Local,
            ..Default::default()
        }
    }

    /// Dust kicked up by wheels or footsteps (use rate_over_distance).
    pub fn dust() -> Self {
        Self {
            start_lifetime: Vector2::new(1.0, 2.0),
            start_speed: Vector2::new(0.3, 1.2),
            start_size: Vector2::new(0.4, 0.9),
            start_rotation: Vector2::new(0.0, 360.0),
            start_color: Color::new(0.6, 0.52, 0.4, 0.4),
            start_color_b: Color::new(0.5, 0.45, 0.36, 0.3),
            gravity_modifier: -0.02,
            rate_over_time: 0.0,
            rate_over_distance: 3.0,
            shape: ParticleShape::Hemisphere,
            shape_radius: 0.3,
            end_color: Color::new(0.6, 0.55, 0.45, 0.0),
            size_over_lifetime: Vector2::new(0.8, 3.0),
            drag: 1.5,
            texture: Some(Handle::from_path("builtin:smoke")),
            ..Default::default()
        }
    }

    /// Rain falling inside a box above the object.
    pub fn rain() -> Self {
        Self {
            start_lifetime: Vector2::new(1.2, 1.2),
            start_speed: Vector2::new(0.0, 0.0),
            start_size: Vector2::new(0.03, 0.05),
            start_color: Color::new(0.7, 0.75, 0.85, 0.5),
            start_color_b: Color::new(0.7, 0.75, 0.85, 0.35),
            gravity_modifier: 2.0,
            max_particles: 6000,
            rate_over_time: 3000.0,
            shape: ParticleShape::Box,
            shape_box: Vector3::new(40.0, 1.0, 40.0),
            end_color: Color::new(0.7, 0.75, 0.85, 0.3),
            simulation_space: SimulationSpace::World,
            ..Default::default()
        }
    }
}

impl Component for ParticleSystem {}

fn rand(s: &mut u32) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    *s as f32 / u32::MAX as f32
}

fn range(s: &mut u32, r: Vector2) -> f32 {
    r.x + (r.y - r.x) * rand(s)
}

fn spawn(ps: &mut ParticleSystem, origin: Vector3, rot: Quaternion, count: u32) {
    let st = &mut ps.state;
    for _ in 0..count {
        if ps.particles.len() >= ps.max_particles as usize {
            break;
        }
        let mut s = st.rng;
        let (pos, dir) = match ps.shape {
            ParticleShape::Point => (Vector3::ZERO, Vector3::Y),
            ParticleShape::Cone => {
                let a = rand(&mut s) * std::f32::consts::TAU;
                let r = ps.shape_radius * rand(&mut s).sqrt();
                let base = Vector3::new(a.cos() * r, 0.0, a.sin() * r);
                let spread = (ps.shape_angle.to_radians()).tan();
                let d = Vector3::new(
                    base.x / ps.shape_radius.max(1e-4) * spread,
                    1.0,
                    base.z / ps.shape_radius.max(1e-4) * spread,
                );
                (base, d.normalize())
            }
            ParticleShape::Sphere | ParticleShape::Hemisphere => {
                let mut d;
                loop {
                    d = Vector3::new(
                        rand(&mut s) * 2.0 - 1.0,
                        rand(&mut s) * 2.0 - 1.0,
                        rand(&mut s) * 2.0 - 1.0,
                    );
                    if d.length_squared() <= 1.0 && d.length_squared() > 1e-4 {
                        break;
                    }
                }
                if ps.shape == ParticleShape::Hemisphere {
                    d.y = d.y.abs();
                }
                let d = d.normalize();
                (d * ps.shape_radius * rand(&mut s), d)
            }
            ParticleShape::Box => {
                let b = ps.shape_box;
                (
                    Vector3::new(
                        (rand(&mut s) - 0.5) * b.x,
                        (rand(&mut s) - 0.5) * b.y,
                        (rand(&mut s) - 0.5) * b.z,
                    ),
                    Vector3::Y,
                )
            }
        };
        let speed = range(&mut s, ps.start_speed);
        let (p, v) = match ps.simulation_space {
            SimulationSpace::World => (origin + rot * pos, rot * dir * speed),
            SimulationSpace::Local => (pos, dir * speed),
        };
        let t = rand(&mut s);
        let color = Color::lerp(ps.start_color, ps.start_color_b, t);
        ps.particles.push(Particle {
            position: p,
            velocity: v,
            age: 0.0,
            lifetime: range(&mut s, ps.start_lifetime).max(0.01),
            size: range(&mut s, ps.start_size),
            rotation: range(&mut s, ps.start_rotation).to_radians(),
            angular_velocity: range(&mut s, ps.angular_velocity).to_radians(),
            color,
            seed: rand(&mut s) * 100.0,
        });
        st.rng = s;
    }
}

/// Steps every ParticleSystem.
pub fn simulate(world: &mut World) {
    let dt = world.resource::<Time>().delta_time();
    if dt <= 0.0 {
        return;
    }
    for (id, cid) in world.query_ids::<ParticleSystem>() {
        let (origin, rot) = (world.position(id), world.rotation(id));
        let Some(ps) = world.component_mut::<ParticleSystem>(id, cid) else {
            continue;
        };
        if ps.state.rng == 0 {
            ps.state.rng = (id.to_bits() as u32).wrapping_mul(2654435761) | 1;
        }
        if !ps.state.started {
            ps.state.started = true;
            ps.state.playing = ps.play_on_awake;
        }
        // Age and move.
        let g = Vector3::new(0.0, -9.81 * ps.gravity_modifier, 0.0);
        let damp = (-ps.drag * dt).exp();
        let noise = ps.noise;
        let mut rng = ps.state.rng;
        ps.particles.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.lifetime {
                return false;
            }
            p.velocity = (p.velocity + g * dt) * damp;
            if noise > 0.0 {
                let t = p.age * 2.0 + p.seed;
                p.velocity += Vector3::new((t * 1.7).sin(), (t * 2.3).cos() * 0.5, (t * 1.3).cos())
                    * noise
                    * dt;
            }
            p.position += p.velocity * dt;
            p.rotation += p.angular_velocity * dt;
            true
        });
        let _ = rand(&mut rng);
        // Emit.
        let mut count = std::mem::take(&mut ps.state.pending_emit);
        if ps.state.playing {
            ps.state.time += dt;
            if ps.burst_count > 0 && !ps.state.burst_done && ps.state.time >= ps.burst_time {
                count += ps.burst_count;
                ps.state.burst_done = true;
            }
            ps.state.emit_accum += ps.rate_over_time * dt;
            if let Some(last) = ps.state.last_position {
                ps.state.emit_accum += ps.rate_over_distance * last.distance(origin);
            }
            let n = ps.state.emit_accum.floor();
            ps.state.emit_accum -= n;
            count += n as u32;
            if ps.state.time >= ps.duration {
                if ps.looping {
                    ps.state.time -= ps.duration;
                    ps.state.burst_done = false;
                } else {
                    ps.state.playing = false;
                }
            }
        }
        ps.state.last_position = Some(origin);
        if count > 0 {
            spawn(ps, origin, rot, count);
        }
    }
}

/// Adds every system's particles to the frame (sorted later by the renderer).
pub fn extract(world: &World, out: &mut Vec<ParticleBatch>) {
    for (id, ps) in world.query::<ParticleSystem>() {
        if ps.particles.is_empty() {
            continue;
        }
        let (o, r) = (world.position(id), world.rotation(id));
        let k = ps.intensity;
        let instances = ps
            .particles
            .iter()
            .map(|p| {
                let t = (p.age / p.lifetime).clamp(0.0, 1.0);
                let c = Color::lerp(
                    p.color,
                    Color::new(
                        p.color.r * ps.end_color.r,
                        p.color.g * ps.end_color.g,
                        p.color.b * ps.end_color.b,
                        ps.end_color.a,
                    ),
                    t,
                )
                .linear();
                let size = p.size
                    * (ps.size_over_lifetime.x
                        + (ps.size_over_lifetime.y - ps.size_over_lifetime.x) * t);
                let pos = match ps.simulation_space {
                    SimulationSpace::World => p.position,
                    SimulationSpace::Local => o + r * p.position,
                };
                ParticleInstance {
                    position: pos.to_array(),
                    size,
                    color: [c.r * k, c.g * k, c.b * k, c.a],
                    rotation: p.rotation,
                    _pad: [0.0; 3],
                }
            })
            .collect();
        out.push(ParticleBatch {
            texture: ps.texture.as_ref().map(|h| h.id()),
            blend: match ps.blend {
                BlendMode::Alpha => ParticleBlend::Alpha,
                BlendMode::Additive => ParticleBlend::Additive,
            },
            particles: instances,
        });
    }
}

/// Built-in particle textures: soft round `builtin:particle` and noisy `builtin:smoke`.
pub fn register_builtin_textures(assets: &mut AssetServer) {
    let size = 64u32;
    let mut soft = Vec::new();
    let mut smoke = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let (u, v) = (
                (x as f32 + 0.5) / size as f32 * 2.0 - 1.0,
                (y as f32 + 0.5) / size as f32 * 2.0 - 1.0,
            );
            let d = (u * u + v * v).sqrt();
            let a = (1.0 - d).clamp(0.0, 1.0).powf(1.5);
            soft.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
            let n = oxide_assets::terrain::value_noise(u * 3.0 + 10.0, v * 3.0 + 10.0, 5) * 0.5
                + oxide_assets::terrain::value_noise(u * 7.0, v * 7.0, 9) * 0.3;
            let s = ((1.0 - d * 1.1).clamp(0.0, 1.0) * (0.7 + n)).clamp(0.0, 1.0);
            smoke.extend_from_slice(&[255, 255, 255, (s * 255.0) as u8]);
        }
    }
    assets.textures.insert(
        "builtin:particle",
        Texture2D::from_rgba8("particle", size, size, soft, true),
    );
    assets.textures.insert(
        "builtin:smoke",
        Texture2D::from_rgba8("smoke", size, size, smoke, true),
    );
}

#[derive(Default)]
pub struct ParticleSystemRunner;

impl System for ParticleSystemRunner {
    fn name(&self) -> &'static str {
        "Particles"
    }
    fn update(&mut self, world: &mut World) {
        simulate(world);
    }
}

/// Script helper: spawn a one-shot effect at a point that destroys itself when done.
pub trait ParticlesExt {
    fn spawn_effect(
        &mut self,
        effect: ParticleSystem,
        position: Vector3,
        rotation: Quaternion,
    ) -> GameObjectId;
}

impl ParticlesExt for Ctx<'_> {
    fn spawn_effect(
        &mut self,
        mut effect: ParticleSystem,
        position: Vector3,
        rotation: Quaternion,
    ) -> GameObjectId {
        let id = self.world.create_at("Effect", position, rotation);
        effect.play();
        let life = effect.duration + effect.start_lifetime.y + 0.1;
        self.world.add_component(id, effect);
        self.destroy_after(id, life);
        id
    }
}

pub fn install(world: &mut World) -> Box<dyn System> {
    world
        .registry
        .register::<ParticleSystem>("Effects/Particle System");
    if let Some(a) = world.try_resource_mut::<AssetServer>() {
        register_builtin_textures(a);
    }
    Box::new(ParticleSystemRunner)
}

#[allow(dead_code)]
fn _id(_: AssetId) {}
