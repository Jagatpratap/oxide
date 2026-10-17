//! Oxide particles: Unity's `ParticleSystem` (Shuriken). The main module (duration, looping,
//! prewarm, start delay, lifetime/speed/size/rotation/color ranges, gravity, simulation space
//! and speed, max particles, stop action), emission (rate over time and distance, bursts with
//! cycles and probability), shapes (cone, sphere, hemisphere, box, point, circle, donut, edge,
//! rectangle, mesh; arc, thickness, shape transform, randomized / spherized directions), and
//! the [`modules`]: velocity / limit velocity / force / color / size / rotation over lifetime
//! and by speed, inherit velocity, lifetime by emitter speed, external forces (Force Fields),
//! noise, collision (planes or physics), triggers, sub-emitters, texture sheet animation,
//! lights, trails and the renderer's billboard modes.
//! Reference: <https://docs.unity3d.com/Manual/ParticleSystems.html>

pub mod effects;
pub mod modules;
pub use effects::{
    BillboardAsset, BillboardRenderer, DecalProjector, LineAlignment, LineRenderer,
    LineTextureMode, TrailRenderer,
};
pub use modules::*;

use oxide_assets::{AssetServer, Handle, Mesh, Texture2D};
use oxide_core::{AnimationCurve, Gradient, Reflect, Time};
use oxide_math::{Color, Quaternion, Vector2, Vector3};
use oxide_render::{ParticleBatch, ParticleBlend, ParticleInstance, ParticleMode, PunctualLight};
use oxide_scene::{Component, Ctx, GameObjectId, System, World};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum ParticleShape {
    #[default]
    Cone,
    Sphere,
    Hemisphere,
    Box,
    Point,
    /// A disc in the XZ plane, emitting outward.
    Circle,
    /// A torus in the XZ plane (`shape_radius` ring, `donut_radius` tube).
    Donut,
    /// A line along X, emitting up.
    Edge,
    /// A rectangle in the XZ plane (`shape_box` x/z), emitting up.
    Rectangle,
    /// The vertices of a mesh, along their normals.
    Mesh,
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

#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct Particle {
    pub position: Vector3,
    pub velocity: Vector3,
    pub age: f32,
    pub lifetime: f32,
    pub start_lifetime: f32,
    pub size: f32,
    pub rotation: f32,
    pub angular_velocity: f32,
    pub color: Color,
    pub seed: f32,
    /// Texture sheet: row and start frame.
    pub sheet_row: f32,
    pub sheet_start: f32,
    /// Birth sub-emitters: particles owed to them.
    pub sub_accum: f32,
    /// Size multiplier from noise.
    pub noise_size: f32,
    /// Trail points (world xyz, system time when recorded); `None` = no trail.
    pub trail: Option<Vec<[f32; 4]>>,
    pub light: bool,
    /// Trigger colliders it is inside (bit per collider).
    pub inside: u32,
}

/// A trail left behind by a dead particle, fading out.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct OrphanTrail {
    pub points: Vec<[f32; 4]>,
    pub color: Color,
    pub width: f32,
    pub lifetime: f32,
}

/// Particles a parent system sends to a sub-emitter.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct SubSpawn {
    pub position: Vector3,
    pub velocity: Vector3,
    pub color: Option<Color>,
    pub size: Option<f32>,
    pub rotation: Option<f32>,
    pub lifetime: Option<f32>,
    pub count: u32,
}

/// Unity `ParticleSystem` (Shuriken).
#[derive(Clone, Debug, Reflect)]
pub struct ParticleSystem {
    pub duration: f32,
    pub looping: bool,
    /// Starts as if it had already run one full cycle (looping systems).
    pub prewarm: bool,
    /// Seconds before emission starts.
    pub start_delay: f32,
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
    /// Plays faster (> 1) or slower.
    pub simulation_speed: f32,
    /// Ignores Time.timeScale.
    pub use_unscaled_time: bool,
    pub max_particles: u32,
    /// What happens when it finishes.
    pub stop_action: StopAction,
    /// 0 picks a seed per object.
    pub random_seed: u32,
    /// Emission per second.
    pub rate_over_time: f32,
    /// Emission per meter moved (trails, tire dust).
    pub rate_over_distance: f32,
    /// One burst at `burst_time` of `burst_count` particles each cycle.
    pub burst_count: u32,
    pub burst_time: f32,
    /// Unity's Bursts list (with cycles, intervals and probability).
    pub bursts: Vec<Burst>,
    pub shape: ParticleShape,
    /// Cone half-angle in degrees.
    pub shape_angle: f32,
    pub shape_radius: f32,
    pub shape_box: Vector3,
    pub shape_options: ShapeOptions,
    /// Multiplies each particle's start colour over its life (Unity's Color over Lifetime).
    pub color_over_lifetime: Gradient,
    /// Multiplies each particle's start size over its life, 0..1 (Unity's Size over Lifetime).
    pub size_over_lifetime: AnimationCurve,
    /// Velocity damping per second (0 = none).
    pub drag: f32,
    /// Random turbulence strength (m/s²); the Noise module is the full version.
    pub noise: f32,
    pub velocity_over_lifetime: VelocityOverLifetime,
    pub limit_velocity_over_lifetime: LimitVelocityOverLifetime,
    pub inherit_velocity: InheritVelocity,
    pub lifetime_by_emitter_speed: LifetimeByEmitterSpeed,
    pub force_over_lifetime: ForceOverLifetime,
    pub color_by_speed: ColorBySpeed,
    pub size_by_speed: SizeBySpeed,
    pub rotation_over_lifetime: RotationOverLifetime,
    pub rotation_by_speed: RotationBySpeed,
    pub external_forces: ExternalForces,
    pub noise_module: NoiseModule,
    pub collision: CollisionModule,
    pub triggers: TriggerModule,
    pub sub_emitters: Vec<SubEmitter>,
    pub texture_sheet_animation: TextureSheetAnimation,
    pub lights: LightsModule,
    pub trails: TrailModule,
    pub renderer: ParticleRendererSettings,
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
    /// Total simulated time (noise scrolling, trail ages).
    pub clock: f32,
    pub delay_left: f32,
    pub emit_accum: f32,
    pub burst_done: bool,
    /// Per burst: repetitions fired this cycle.
    pub burst_fired: Vec<u32>,
    pub last_position: Option<Vector3>,
    /// The emitter's velocity (m/s).
    pub velocity: Vector3,
    pub rng: u32,
    pub pending_emit: u32,
    pub sub_pending: Vec<SubSpawn>,
    pub orphan_trails: Vec<OrphanTrail>,
    pub stopped_handled: bool,
    /// Trigger callbacks this frame: inside, outside, enter, exit.
    pub trigger_counts: [u32; 4],
    /// Manual sub-emitters fired by scripts.
    pub manual_subs: Vec<usize>,
}

impl Default for ParticleSystem {
    fn default() -> Self {
        Self {
            duration: 5.0,
            looping: true,
            prewarm: false,
            start_delay: 0.0,
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
            simulation_speed: 1.0,
            use_unscaled_time: false,
            max_particles: 1000,
            stop_action: StopAction::None,
            random_seed: 0,
            rate_over_time: 10.0,
            rate_over_distance: 0.0,
            burst_count: 0,
            burst_time: 0.0,
            bursts: Vec::new(),
            shape: ParticleShape::Cone,
            shape_angle: 25.0,
            shape_radius: 1.0,
            shape_box: Vector3::ONE,
            shape_options: ShapeOptions::default(),
            color_over_lifetime: fade(Color::new(1.0, 1.0, 1.0, 0.0)),
            size_over_lifetime: AnimationCurve::constant(0.0, 1.0, 1.0),
            drag: 0.0,
            noise: 0.0,
            velocity_over_lifetime: VelocityOverLifetime::default(),
            limit_velocity_over_lifetime: LimitVelocityOverLifetime::default(),
            inherit_velocity: InheritVelocity::default(),
            lifetime_by_emitter_speed: LifetimeByEmitterSpeed::default(),
            force_over_lifetime: ForceOverLifetime::default(),
            color_by_speed: ColorBySpeed::default(),
            size_by_speed: SizeBySpeed::default(),
            rotation_over_lifetime: RotationOverLifetime::default(),
            rotation_by_speed: RotationBySpeed::default(),
            external_forces: ExternalForces::default(),
            noise_module: NoiseModule::default(),
            collision: CollisionModule::default(),
            triggers: TriggerModule::default(),
            sub_emitters: Vec::new(),
            texture_sheet_animation: TextureSheetAnimation::default(),
            lights: LightsModule::default(),
            trails: TrailModule::default(),
            renderer: ParticleRendererSettings::default(),
            texture: Some(Handle::from_path("builtin:particle")),
            blend: BlendMode::Alpha,
            intensity: 1.0,
            particles: Vec::new(),
            state: EmitterState::default(),
        }
    }
}

/// White fading to `end` over a particle's life (the old start/end colour pair).
pub fn fade(end: Color) -> Gradient {
    Gradient::two(Color::WHITE, end)
}

impl ParticleSystem {
    pub fn play(&mut self) {
        self.state.playing = true;
        self.state.started = true;
        self.state.time = 0.0;
        self.state.burst_done = false;
        self.state.burst_fired.clear();
        self.state.delay_left = self.start_delay;
        self.state.stopped_handled = false;
    }
    /// Stops emitting; live particles finish their lives (Unity `Stop()`).
    pub fn stop(&mut self) {
        self.state.playing = false;
    }
    /// Stops and removes every particle.
    pub fn clear(&mut self) {
        self.state.playing = false;
        self.particles.clear();
        self.state.orphan_trails.clear();
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
        self.state.playing || !self.particles.is_empty() || !self.state.orphan_trails.is_empty()
    }
    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }
    /// Unity `TriggerSubEmitter`: fires the Manual sub-emitter `index` at every particle.
    pub fn trigger_sub_emitter(&mut self, index: usize) {
        self.state.manual_subs.push(index);
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
            color_over_lifetime: fade(Color::new(0.6, 0.6, 0.6, 0.0)),
            size_over_lifetime: AnimationCurve::linear(0.0, 0.6, 1.0, 2.8),
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
            color_over_lifetime: fade(Color::new(0.8, 0.1, 0.0, 0.0)),
            size_over_lifetime: AnimationCurve::linear(0.0, 1.0, 1.0, 0.2),
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
            color_over_lifetime: fade(Color::new(0.4, 0.05, 0.0, 0.0)),
            size_over_lifetime: AnimationCurve::linear(0.0, 1.0, 1.0, 2.0),
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
            color_over_lifetime: fade(Color::new(1.0, 0.5, 0.1, 0.0)),
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
            color_over_lifetime: fade(Color::new(0.6, 0.55, 0.45, 0.0)),
            size_over_lifetime: AnimationCurve::linear(0.0, 0.8, 1.0, 3.0),
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
            color_over_lifetime: fade(Color::new(0.7, 0.75, 0.85, 0.3)),
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

fn unit_vector(s: &mut u32) -> Vector3 {
    loop {
        let d = Vector3::new(
            rand(s) * 2.0 - 1.0,
            rand(s) * 2.0 - 1.0,
            rand(s) * 2.0 - 1.0,
        );
        let l = d.length_squared();
        if l <= 1.0 && l > 1e-4 {
            return d / l.sqrt();
        }
    }
}

// ---- noise ---------------------------------------------------------------------------------

fn hash3(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f)
        ^ seed.wrapping_mul(0x1656_67b1);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 32767.5 - 1.0
}

/// Smooth 3D value noise in -1..1.
fn value_noise(p: Vector3, seed: u32) -> f32 {
    let f = p.floor();
    let t = p - f;
    let s = t * t * (Vector3::splat(3.0) - t * 2.0);
    let (x, y, z) = (f.x as i32, f.y as i32, f.z as i32);
    let c = |dx: i32, dy: i32, dz: i32| hash3(x + dx, y + dy, z + dz, seed);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(c(0, 0, 0), c(1, 0, 0), s.x);
    let x10 = lerp(c(0, 1, 0), c(1, 1, 0), s.x);
    let x01 = lerp(c(0, 0, 1), c(1, 0, 1), s.x);
    let x11 = lerp(c(0, 1, 1), c(1, 1, 1), s.x);
    lerp(lerp(x00, x10, s.y), lerp(x01, x11, s.y), s.z)
}

/// The Noise module's 3D field at `p` (fractal value noise per axis).
fn noise_field(m: &NoiseModule, p: Vector3, time: f32) -> Vector3 {
    let mut out = Vector3::ZERO;
    let (mut amp, mut freq) = (1.0, m.frequency.max(1e-3));
    let scroll = Vector3::splat(m.scroll_speed * time);
    for o in 0..m.octaves.clamp(1, 4) {
        let q = p * freq + scroll;
        out += Vector3::new(
            value_noise(q, 11 + o * 3),
            value_noise(q, 12 + o * 3),
            value_noise(q, 13 + o * 3),
        ) * amp;
        amp *= m.octave_multiplier;
        freq *= m.octave_scale;
    }
    out
}

// ---- spawning ------------------------------------------------------------------------------

/// What spawning needs from outside the component.
struct SpawnEnv<'a> {
    origin: Vector3,
    rotation: Quaternion,
    emitter_velocity: Vector3,
    /// Mesh shape: vertices and normals (object space).
    mesh: Option<&'a [(Vector3, Vector3)]>,
}

/// Position and direction from the shape, in the shape's space.
fn shape_point(
    ps: &ParticleSystem,
    s: &mut u32,
    mesh: Option<&[(Vector3, Vector3)]>,
) -> (Vector3, Vector3) {
    let o = &ps.shape_options;
    let r_max = ps.shape_radius;
    // Distance from the centre: anywhere in the shell `thickness` thick.
    let radial = |s: &mut u32, area_like: bool| {
        let t = rand(s);
        let t = if area_like { t.sqrt() } else { t.cbrt() };
        r_max * (1.0 - o.radius_thickness.clamp(0.0, 1.0) * (1.0 - t))
    };
    let arc = o.arc.clamp(0.0, 360.0).to_radians();
    match ps.shape {
        ParticleShape::Point => (Vector3::ZERO, Vector3::Y),
        ParticleShape::Cone => {
            let a = rand(s) * arc;
            let r = radial(s, true);
            let base = Vector3::new(a.cos() * r, 0.0, a.sin() * r);
            let spread = ps.shape_angle.to_radians().tan();
            let d = Vector3::new(
                base.x / r_max.max(1e-4) * spread,
                1.0,
                base.z / r_max.max(1e-4) * spread,
            );
            (base, d.normalize())
        }
        ParticleShape::Sphere | ParticleShape::Hemisphere => {
            let mut d = unit_vector(s);
            if ps.shape == ParticleShape::Hemisphere {
                d.y = d.y.abs();
            }
            (d * radial(s, false), d)
        }
        ParticleShape::Box => {
            let b = ps.shape_box;
            (
                Vector3::new(
                    (rand(s) - 0.5) * b.x,
                    (rand(s) - 0.5) * b.y,
                    (rand(s) - 0.5) * b.z,
                ),
                Vector3::Y,
            )
        }
        ParticleShape::Circle => {
            let a = rand(s) * arc;
            let d = Vector3::new(a.cos(), 0.0, a.sin());
            (d * radial(s, true), d)
        }
        ParticleShape::Donut => {
            let a = rand(s) * arc;
            let ring = Vector3::new(a.cos(), 0.0, a.sin());
            let b = rand(s) * std::f32::consts::TAU;
            let tube = (ring * b.cos() + Vector3::Y * b.sin()) * o.donut_radius * rand(s).sqrt();
            let d = tube.normalize_or(ring);
            (ring * r_max + tube, d)
        }
        ParticleShape::Edge => (
            Vector3::new((rand(s) * 2.0 - 1.0) * r_max, 0.0, 0.0),
            Vector3::Y,
        ),
        ParticleShape::Rectangle => (
            Vector3::new(
                (rand(s) - 0.5) * ps.shape_box.x,
                0.0,
                (rand(s) - 0.5) * ps.shape_box.z,
            ),
            Vector3::Y,
        ),
        ParticleShape::Mesh => match mesh.filter(|m| !m.is_empty()) {
            Some(m) => {
                let i = ((rand(s) * m.len() as f32) as usize).min(m.len() - 1);
                (m[i].0, m[i].1.normalize_or(Vector3::Y))
            }
            None => (Vector3::ZERO, Vector3::Y),
        },
    }
}

fn spawn(ps: &mut ParticleSystem, env: &SpawnEnv, count: u32) {
    let o = ps.shape_options.clone();
    let shape_rot = Quaternion::from_euler(
        oxide_math::glam::EulerRot::YXZ,
        o.rotation.y.to_radians(),
        o.rotation.x.to_radians(),
        o.rotation.z.to_radians(),
    );
    let sheet = ps.texture_sheet_animation.clone();
    let (cols, rows) = (
        sheet.tiles.x.max(1.0).floor(),
        sheet.tiles.y.max(1.0).floor(),
    );
    let emitter_speed = env.emitter_velocity.length();
    for _ in 0..count {
        if ps.particles.len() >= ps.max_particles as usize {
            break;
        }
        let mut s = ps.state.rng;
        let (mut pos, mut dir) = shape_point(ps, &mut s, env.mesh);
        pos = shape_rot * (pos * o.scale) + o.position;
        dir = (shape_rot * dir).normalize_or(Vector3::Y);
        if o.randomize_position > 0.0 {
            pos += unit_vector(&mut s) * o.randomize_position * rand(&mut s);
        }
        if o.spherize_direction > 0.0 {
            dir = dir
                .lerp(pos.normalize_or(dir), o.spherize_direction.clamp(0.0, 1.0))
                .normalize_or(dir);
        }
        if o.randomize_direction > 0.0 {
            dir = dir
                .lerp(unit_vector(&mut s), o.randomize_direction.clamp(0.0, 1.0))
                .normalize_or(dir);
        }
        let speed = range(&mut s, ps.start_speed);
        let (mut p, mut v) = match ps.simulation_space {
            SimulationSpace::World => (env.origin + env.rotation * pos, env.rotation * dir * speed),
            SimulationSpace::Local => (pos, dir * speed),
        };
        if ps.inherit_velocity.enabled && ps.inherit_velocity.mode == InheritVelocityMode::Initial {
            let ev = env.emitter_velocity * ps.inherit_velocity.multiplier;
            v += match ps.simulation_space {
                SimulationSpace::World => ev,
                SimulationSpace::Local => env.rotation.inverse() * ev,
            };
        }
        if p.is_nan() {
            p = env.origin;
        }
        let t = rand(&mut s);
        let color = Color::lerp(ps.start_color, ps.start_color_b, t);
        let mut lifetime = range(&mut s, ps.start_lifetime).max(0.01);
        let lbs = &ps.lifetime_by_emitter_speed;
        if lbs.enabled {
            let k = ((emitter_speed - lbs.range.x) / (lbs.range.y - lbs.range.x).max(1e-4))
                .clamp(0.0, 1.0);
            lifetime = (lifetime * lbs.curve.evaluate(k)).max(0.01);
        }
        let mut rotation = range(&mut s, ps.start_rotation).to_radians();
        if o.align_to_direction {
            rotation = (-dir.x).atan2(dir.y);
        }
        let sheet_row = if sheet.animation == SheetAnimation::SingleRow {
            if sheet.random_row {
                (rand(&mut s) * rows).floor().min(rows - 1.0)
            } else {
                (sheet.row_index as f32).min(rows - 1.0)
            }
        } else {
            0.0
        };
        let sheet_start = range(&mut s, sheet.start_frame).floor();
        let trail = (ps.trails.enabled && rand(&mut s) < ps.trails.ratio).then(Vec::new);
        let light = ps.lights.enabled && rand(&mut s) < ps.lights.ratio;
        let _ = cols;
        ps.particles.push(Particle {
            position: p,
            velocity: v,
            age: 0.0,
            lifetime,
            start_lifetime: lifetime,
            size: range(&mut s, ps.start_size),
            rotation,
            angular_velocity: range(&mut s, ps.angular_velocity).to_radians(),
            color,
            seed: rand(&mut s) * 100.0,
            sheet_row,
            sheet_start,
            sub_accum: 0.0,
            noise_size: 1.0,
            trail,
            light,
            inside: 0,
        });
        ps.state.rng = s;
    }
}

/// Spawns what a parent sent at its particles' positions, with what they inherit.
fn spawn_sub(ps: &mut ParticleSystem, rotation: Quaternion, sub: &SubSpawn) {
    let before = ps.particles.len();
    let env = SpawnEnv {
        origin: sub.position,
        rotation,
        emitter_velocity: Vector3::ZERO,
        mesh: None,
    };
    // Sub-emitters live in world space at the parent's particles.
    let space = ps.simulation_space;
    ps.simulation_space = SimulationSpace::World;
    spawn(ps, &env, sub.count);
    ps.simulation_space = space;
    for p in &mut ps.particles[before..] {
        p.velocity += sub.velocity;
        if let Some(c) = sub.color {
            p.color = Color::new(
                p.color.r * c.r,
                p.color.g * c.g,
                p.color.b * c.b,
                p.color.a * c.a,
            );
        }
        if let Some(sz) = sub.size {
            p.size *= sz;
        }
        if let Some(r) = sub.rotation {
            p.rotation += r;
        }
        if let Some(l) = sub.lifetime {
            p.lifetime = l.max(0.01);
            p.start_lifetime = p.lifetime;
        }
    }
}

// ---- simulation ----------------------------------------------------------------------------

/// A force field in the scene, in world space.
#[derive(Clone)]
struct Field {
    position: Vector3,
    rotation: Quaternion,
    field: ParticleSystemForceField,
    layer: u8,
}

/// What a sub-emitter target needs: how many particles a Birth sub-emitter makes per second
/// and a Death/Collision one per event.
#[derive(Clone, Copy)]
struct SubTarget {
    rate: f32,
    burst: u32,
}

struct StepEnv<'a> {
    id: GameObjectId,
    spawn: SpawnEnv<'a>,
    physics: Option<&'a oxide_physics::Physics>,
    fields: &'a [Field],
    sub_targets: &'a HashMap<GameObjectId, SubTarget>,
    /// This system is another's sub-emitter: it doesn't emit by itself.
    is_sub: bool,
}

#[derive(Default)]
struct StepOut {
    subs: Vec<(GameObjectId, SubSpawn)>,
    /// Objects particles hit (for OnParticleCollision).
    collisions: Vec<GameObjectId>,
    finished: bool,
}

/// One sub-emitter event at a particle.
fn sub_event(
    se: &SubEmitter,
    p: &Particle,
    count: u32,
    world_pos: Vector3,
    world_vel: Vector3,
    rng: &mut u32,
    out: &mut StepOut,
) {
    let Some(target) = se.emitter else { return };
    if count == 0 || rand(rng) > se.emit_probability {
        return;
    }
    out.subs.push((
        target,
        SubSpawn {
            position: world_pos,
            velocity: world_vel * se.inherit_velocity,
            color: se.inherit_color.then_some(p.color),
            size: se.inherit_size.then_some(p.size),
            rotation: se.inherit_rotation.then_some(p.rotation),
            lifetime: se.inherit_lifetime.then_some(p.lifetime - p.age),
            count,
        },
    ));
}

fn step(ps: &mut ParticleSystem, env: &StepEnv, dt: f32, out: &mut StepOut) {
    let (origin, rot) = (env.spawn.origin, env.spawn.rotation);
    let to_world = |ps: &ParticleSystem, p: Vector3| match ps.simulation_space {
        SimulationSpace::World => p,
        SimulationSpace::Local => origin + rot * p,
    };
    let dir_to_world = |ps: &ParticleSystem, v: Vector3| match ps.simulation_space {
        SimulationSpace::World => v,
        SimulationSpace::Local => rot * v,
    };
    let dir_from_world = |ps: &ParticleSystem, v: Vector3| match ps.simulation_space {
        SimulationSpace::World => v,
        SimulationSpace::Local => rot.inverse() * v,
    };
    ps.state.clock += dt;
    let clock = ps.state.clock;
    // Sub-emitter particles sent by parents.
    for sub in std::mem::take(&mut ps.state.sub_pending) {
        spawn_sub(ps, rot, &sub);
    }
    // Manual sub-emitters fired from scripts.
    for i in std::mem::take(&mut ps.state.manual_subs) {
        let Some(se) = ps.sub_emitters.get(i).cloned() else {
            continue;
        };
        let burst = se
            .emitter
            .and_then(|t| env.sub_targets.get(&t))
            .map_or(1, |t| t.burst);
        let mut rng = ps.state.rng;
        for p in &ps.particles {
            let (wp, wv) = (to_world(ps, p.position), dir_to_world(ps, p.velocity));
            sub_event(&se, p, burst, wp, wv, &mut rng, out);
        }
        ps.state.rng = rng;
    }

    let g = Vector3::new(0.0, -9.81 * ps.gravity_modifier, 0.0);
    let gravity = dir_from_world(ps, g);
    let damp = (-ps.drag * dt).exp();
    let fol = ps.force_over_lifetime.clone();
    let fol_force = if fol.enabled {
        match (fol.space, ps.simulation_space) {
            (SimulationSpace::Local, SimulationSpace::World) => rot * fol.force,
            (SimulationSpace::World, SimulationSpace::Local) => rot.inverse() * fol.force,
            _ => fol.force,
        }
    } else {
        Vector3::ZERO
    };
    let vol = ps.velocity_over_lifetime.clone();
    let vol_linear = if vol.enabled {
        match (vol.space, ps.simulation_space) {
            (SimulationSpace::Local, SimulationSpace::World) => rot * vol.linear,
            (SimulationSpace::World, SimulationSpace::Local) => rot.inverse() * vol.linear,
            _ => vol.linear,
        }
    } else {
        Vector3::ZERO
    };
    let vol_center = match ps.simulation_space {
        SimulationSpace::World => origin + rot * vol.offset,
        SimulationSpace::Local => vol.offset,
    };
    let inherit_current = (ps.inherit_velocity.enabled
        && ps.inherit_velocity.mode == InheritVelocityMode::Current)
        .then(|| {
            dir_from_world(
                ps,
                env.spawn.emitter_velocity * ps.inherit_velocity.multiplier,
            )
        });
    let limit = ps.limit_velocity_over_lifetime.clone();
    let noise = ps.noise_module.clone();
    let legacy_noise = ps.noise;
    let rol = ps.rotation_over_lifetime.clone();
    let rbs = ps.rotation_by_speed.clone();
    let ext = ps.external_forces.clone();
    let col = ps.collision.clone();
    let trig = ps.triggers.clone();
    let trails = ps.trails.clone();
    let subs = ps.sub_emitters.clone();
    let birth: Vec<(SubEmitter, f32)> = subs
        .iter()
        .filter(|s| s.kind == SubEmitterType::Birth)
        .filter_map(|s| Some((s.clone(), env.sub_targets.get(&s.emitter?)?.rate)))
        .collect();
    let burst_of = |kind: SubEmitterType| -> Vec<(SubEmitter, u32)> {
        subs.iter()
            .filter(|s| s.kind == kind)
            .filter_map(|s| Some((s.clone(), env.sub_targets.get(&s.emitter?)?.burst)))
            .collect()
    };
    let (death_subs, collision_subs, trigger_subs) = (
        burst_of(SubEmitterType::Death),
        burst_of(SubEmitterType::Collision),
        burst_of(SubEmitterType::Trigger),
    );
    let space = ps.simulation_space;
    let mut rng = ps.state.rng;
    let mut orphans = Vec::new();
    let mut trigger_counts = [0u32; 4];
    let mut particles = std::mem::take(&mut ps.particles);
    particles.retain_mut(|p| {
        p.age += dt;
        let world_pos = |p: &Particle| match space {
            SimulationSpace::World => p.position,
            SimulationSpace::Local => origin + rot * p.position,
        };
        let world_vel = |v: Vector3| match space {
            SimulationSpace::World => v,
            SimulationSpace::Local => rot * v,
        };
        if p.age >= p.lifetime {
            for (se, burst) in &death_subs {
                sub_event(
                    se,
                    p,
                    *burst,
                    world_pos(p),
                    world_vel(p.velocity),
                    &mut rng,
                    out,
                );
            }
            if let Some(t) = p.trail.take()
                && !trails.die_with_particles
                && !t.is_empty()
            {
                orphans.push(OrphanTrail {
                    points: t,
                    color: p.color,
                    width: p.size,
                    lifetime: trails.lifetime * p.start_lifetime,
                });
            }
            return false;
        }
        let t = p.age / p.lifetime.max(1e-4);
        let prev = p.position;
        // Forces.
        let mut accel = gravity;
        if fol.enabled {
            accel += if fol.randomized {
                Vector3::new(
                    (rand(&mut rng) * 2.0 - 1.0) * fol_force.x,
                    (rand(&mut rng) * 2.0 - 1.0) * fol_force.y,
                    (rand(&mut rng) * 2.0 - 1.0) * fol_force.z,
                )
            } else {
                fol_force
            };
        }
        if ext.enabled {
            let wp = world_pos(p);
            let wv = world_vel(p.velocity);
            let mut f = Vector3::ZERO;
            let mut field_drag = 0.0;
            for fld in env.fields {
                if ext.influence_mask & (1 << fld.layer.min(31)) == 0 {
                    continue;
                }
                let inv = fld.rotation.inverse();
                let local = inv * (wp - fld.position);
                let Some(k) = fld.field.falloff(local) else {
                    continue;
                };
                let ff = &fld.field;
                f += fld.rotation * ff.direction * k;
                if ff.gravity != 0.0 {
                    // Toward the centre, or the shell at `gravity_focus`.
                    let r = local.length();
                    let target =
                        local.normalize_or_zero() * ff.end_range * ff.gravity_focus.clamp(0.0, 1.0);
                    let to = (target - local).normalize_or_zero();
                    if r > 1e-4 {
                        f += fld.rotation * to * ff.gravity * k;
                    }
                }
                if ff.rotation_speed != 0.0 {
                    let tangent = Vector3::Y
                        .cross(Vector3::new(local.x, 0.0, local.z))
                        .normalize_or_zero();
                    let swirl = fld.rotation * tangent * ff.rotation_speed;
                    f += swirl * k;
                    // Attraction pulls the velocity toward the swirl.
                    f += (swirl - wv) * ff.rotation_attraction.clamp(0.0, 1.0) * k;
                }
                field_drag += ff.drag * k;
            }
            accel += dir_from_world_space(space, rot, f * ext.multiplier);
            if field_drag > 0.0 {
                p.velocity *= (-field_drag * ext.multiplier * dt).exp();
            }
        }
        p.velocity = (p.velocity + accel * dt) * damp;
        if legacy_noise > 0.0 && !noise.enabled {
            let tt = p.age * 2.0 + p.seed;
            p.velocity += Vector3::new((tt * 1.7).sin(), (tt * 2.3).cos() * 0.5, (tt * 1.3).cos())
                * legacy_noise
                * dt;
        }
        // Limit velocity.
        if limit.enabled {
            let max = limit.limit.evaluate(t).max(0.0);
            let speed = p.velocity.length();
            if speed > max && speed > 1e-6 {
                let target = p.velocity * (max / speed);
                p.velocity = p.velocity.lerp(target, limit.dampen.clamp(0.0, 1.0));
            }
            if limit.drag > 0.0 {
                let mut d = limit.drag;
                if limit.multiply_drag_by_size {
                    d *= p.size.max(0.0);
                }
                if limit.multiply_drag_by_velocity {
                    d *= p.velocity.length();
                }
                p.velocity *= (-d * dt).exp();
            }
        }
        // Move.
        let speed_mod = if vol.enabled {
            vol.speed_modifier.evaluate(t)
        } else {
            1.0
        };
        let mut step = p.velocity * speed_mod * dt;
        if vol.enabled {
            step += vol_linear * dt;
            let rel = p.position - vol_center;
            if vol.orbital != Vector3::ZERO {
                let q = Quaternion::from_euler(
                    oxide_math::glam::EulerRot::XYZ,
                    vol.orbital.x * dt,
                    vol.orbital.y * dt,
                    vol.orbital.z * dt,
                );
                let axis_rot = match space {
                    SimulationSpace::World => rot * q * rot.inverse(),
                    SimulationSpace::Local => q,
                };
                step += axis_rot * rel - rel;
            }
            if vol.radial != 0.0 {
                step += rel.normalize_or_zero() * vol.radial * dt;
            }
        }
        if let Some(iv) = inherit_current {
            step += iv * dt;
        }
        if noise.enabled {
            let n = noise_field(&noise, world_pos(p), clock);
            let strength = if noise.damping {
                noise.strength / noise.frequency.max(0.01)
            } else {
                noise.strength
            };
            step += dir_from_world_space(space, rot, n * strength * noise.position_amount * dt);
            p.rotation += n.x * noise.rotation_amount.to_radians() * dt;
            p.noise_size = (1.0 + n.y * noise.size_amount).max(0.0);
        }
        p.position += step;
        // Spin.
        let mut spin = p.angular_velocity;
        if rol.enabled {
            spin += rol.angular_velocity.evaluate(t).to_radians();
        }
        if rbs.enabled {
            let k = ((p.velocity.length() - rbs.range.x) / (rbs.range.y - rbs.range.x).max(1e-4))
                .clamp(0.0, 1.0);
            spin += rbs.angular_velocity.evaluate(k).to_radians();
        }
        p.rotation += spin * dt;
        // Collision.
        if col.enabled {
            let radius = p.size * 0.5 * col.radius_scale;
            let mut hit: Option<(Vector3, Option<GameObjectId>)> = None;
            match col.kind {
                CollisionType::Planes => {
                    for pl in &col.planes {
                        let n = Vector3::new(pl.x, pl.y, pl.z).normalize_or_zero();
                        if n == Vector3::ZERO {
                            continue;
                        }
                        // Planes are in world space.
                        let wp = world_pos(p);
                        let dist = wp.dot(n) - pl.w;
                        if dist < radius && world_vel(p.velocity).dot(n) < 0.0 {
                            let fix = n * (radius - dist);
                            p.position += dir_from_world_space(space, rot, fix);
                            hit = Some((dir_from_world_space(space, rot, n), None));
                            break;
                        }
                    }
                }
                CollisionType::World => {
                    if let Some(ph) = env.physics {
                        let (a, b) = (to_world_space(space, origin, rot, prev), world_pos(p));
                        let d = b - a;
                        let len = d.length();
                        if len > 1e-6
                            && let Some(h) = ph.raycast(a, d / len, len + radius, col.collides_with)
                        {
                            let at = h.point + h.normal * radius;
                            p.position = from_world_space(space, origin, rot, at);
                            hit = Some((
                                dir_from_world_space(space, rot, h.normal),
                                Some(h.game_object),
                            ));
                        }
                    }
                }
            }
            if let Some((n, obj)) = hit {
                let vn = n * p.velocity.dot(n);
                let vt = p.velocity - vn;
                p.velocity = (vt - vn * col.bounce) * (1.0 - col.dampen.clamp(0.0, 1.0));
                p.lifetime -= p.start_lifetime * col.lifetime_loss.clamp(0.0, 1.0);
                if col.send_collision_messages {
                    out.collisions.push(obj.unwrap_or(env.id));
                }
                for (se, burst) in &collision_subs {
                    sub_event(
                        se,
                        p,
                        *burst,
                        world_pos(p),
                        world_vel(p.velocity),
                        &mut rng,
                        out,
                    );
                }
                let speed = p.velocity.length();
                if speed < col.min_kill_speed || speed > col.max_kill_speed || p.age >= p.lifetime {
                    for (se, burst) in &death_subs {
                        sub_event(
                            se,
                            p,
                            *burst,
                            world_pos(p),
                            world_vel(p.velocity),
                            &mut rng,
                            out,
                        );
                    }
                    return false;
                }
            }
        }
        // Triggers.
        if trig.enabled
            && !trig.colliders.is_empty()
            && let Some(ph) = env.physics
        {
            let radius = (p.size * 0.5 * trig.radius_scale).max(1e-3);
            let touching = ph.overlap_sphere(world_pos(p), radius, u32::MAX);
            let mut now = 0u32;
            for (i, c) in trig.colliders.iter().take(32).enumerate() {
                if touching.contains(c) {
                    now |= 1 << i;
                }
            }
            let entered = now & !p.inside;
            let exited = p.inside & !now;
            p.inside = now;
            let mut kill = false;
            for (k, (cond, action)) in [
                (now != 0, trig.inside),
                (now == 0, trig.outside),
                (entered != 0, trig.enter),
                (exited != 0, trig.exit),
            ]
            .into_iter()
            .enumerate()
            {
                if cond {
                    match action {
                        TriggerAction::Kill => kill = true,
                        TriggerAction::Callback => trigger_counts[k] += 1,
                        TriggerAction::Ignore => {}
                    }
                }
            }
            if entered != 0 {
                for (se, burst) in &trigger_subs {
                    sub_event(
                        se,
                        p,
                        *burst,
                        world_pos(p),
                        world_vel(p.velocity),
                        &mut rng,
                        out,
                    );
                }
            }
            if kill {
                return false;
            }
        }
        // Birth sub-emitters emit along the particle's path.
        for (se, rate) in &birth {
            p.sub_accum += rate * dt;
            let n = p.sub_accum.floor();
            if n >= 1.0 {
                p.sub_accum -= n;
                sub_event(
                    se,
                    p,
                    n as u32,
                    world_pos(p),
                    world_vel(p.velocity),
                    &mut rng,
                    out,
                );
            }
        }
        // Trail points.
        if let Some(tr) = p.trail.as_mut() {
            let wp = to_world_space(space, origin, rot, p.position);
            let last = tr.last().map(|q| Vector3::new(q[0], q[1], q[2]));
            if last.is_none_or(|l| l.distance(wp) >= trails.minimum_vertex_distance.max(1e-4)) {
                tr.push([wp.x, wp.y, wp.z, clock]);
            }
            let keep = trails.lifetime * p.start_lifetime;
            tr.retain(|q| clock - q[3] <= keep);
        }
        true
    });
    ps.particles = particles;
    ps.state.rng = rng;
    ps.state.orphan_trails.extend(orphans);
    for o in &mut ps.state.orphan_trails {
        let keep = o.lifetime;
        o.points.retain(|q| clock - q[3] <= keep);
    }
    ps.state.orphan_trails.retain(|o| !o.points.is_empty());
    for (i, n) in trigger_counts.iter().enumerate() {
        ps.state.trigger_counts[i] += n;
    }

    // Emit.
    let mut count = std::mem::take(&mut ps.state.pending_emit);
    if ps.state.playing && ps.state.delay_left > 0.0 {
        ps.state.delay_left -= dt;
    } else if ps.state.playing {
        ps.state.time += dt;
        if !env.is_sub {
            if ps.burst_count > 0 && !ps.state.burst_done && ps.state.time >= ps.burst_time {
                count += ps.burst_count;
                ps.state.burst_done = true;
            }
            ps.state.burst_fired.resize(ps.bursts.len(), 0);
            let mut rng = ps.state.rng;
            for (i, b) in ps.bursts.iter().enumerate() {
                loop {
                    let fired = ps.state.burst_fired[i];
                    if b.cycles != 0 && fired >= b.cycles {
                        break;
                    }
                    let at = b.time + fired as f32 * b.interval.max(1e-3);
                    if ps.state.time < at || at > ps.duration {
                        break;
                    }
                    ps.state.burst_fired[i] += 1;
                    if rand(&mut rng) <= b.probability {
                        count += range(&mut rng, b.count).round().max(0.0) as u32;
                    }
                }
            }
            ps.state.rng = rng;
            ps.state.emit_accum += ps.rate_over_time * dt;
            if let Some(last) = ps.state.last_position {
                ps.state.emit_accum += ps.rate_over_distance * last.distance(origin);
            }
            let n = ps.state.emit_accum.floor();
            ps.state.emit_accum -= n;
            count += n as u32;
        }
        if ps.state.time >= ps.duration {
            if ps.looping {
                ps.state.time -= ps.duration;
                ps.state.burst_done = false;
                ps.state.burst_fired.iter_mut().for_each(|f| *f = 0);
            } else {
                ps.state.playing = false;
            }
        }
    }
    ps.state.last_position = Some(origin);
    if count > 0 {
        spawn(ps, &env.spawn, count);
    }
    if ps.state.started && !ps.is_alive() && !ps.state.stopped_handled && ps.state.pending_emit == 0
    {
        ps.state.stopped_handled = true;
        out.finished = true;
    }
}

fn to_world_space(space: SimulationSpace, origin: Vector3, rot: Quaternion, p: Vector3) -> Vector3 {
    match space {
        SimulationSpace::World => p,
        SimulationSpace::Local => origin + rot * p,
    }
}

fn from_world_space(
    space: SimulationSpace,
    origin: Vector3,
    rot: Quaternion,
    p: Vector3,
) -> Vector3 {
    match space {
        SimulationSpace::World => p,
        SimulationSpace::Local => rot.inverse() * (p - origin),
    }
}

fn dir_from_world_space(space: SimulationSpace, rot: Quaternion, v: Vector3) -> Vector3 {
    match space {
        SimulationSpace::World => v,
        SimulationSpace::Local => rot.inverse() * v,
    }
}

/// Steps every ParticleSystem.
pub fn simulate(world: &mut World) {
    let (scaled, unscaled) = {
        let t = world.resource::<Time>();
        (t.delta_time(), t.unscaled_delta_time())
    };
    let ids = world.query_ids::<ParticleSystem>();
    if ids.is_empty() {
        return;
    }
    let physics = world.resources.remove::<oxide_physics::Physics>();
    let fields: Vec<Field> = world
        .query::<ParticleSystemForceField>()
        .filter(|(id, _)| world.get(*id).is_some_and(|o| o.active_in_hierarchy()))
        .map(|(id, f)| Field {
            position: world.position(id),
            rotation: world.rotation(id),
            field: f.clone(),
            layer: world.get(id).map_or(0, |o| o.layer),
        })
        .collect();
    let mut sub_targets: HashMap<GameObjectId, SubTarget> = HashMap::new();
    for (_, ps) in world.query::<ParticleSystem>() {
        for se in &ps.sub_emitters {
            let Some(t) = se.emitter else { continue };
            if let Some(target) = world.get_component::<ParticleSystem>(t) {
                let burst: f32 = target.burst_count as f32
                    + target
                        .bursts
                        .iter()
                        .map(|b| (b.count.x + b.count.y) * 0.5)
                        .sum::<f32>();
                sub_targets.insert(
                    t,
                    SubTarget {
                        rate: target.rate_over_time,
                        burst: (burst.round() as u32).max(1),
                    },
                );
            }
        }
    }
    let mut subs: Vec<(GameObjectId, SubSpawn)> = Vec::new();
    let mut collisions: Vec<(GameObjectId, GameObjectId)> = Vec::new();
    let mut finished: Vec<(GameObjectId, StopAction)> = Vec::new();
    let mut triggers: Vec<(GameObjectId, [u32; 4])> = Vec::new();
    for (id, cid) in ids {
        let (origin, rot) = (world.position(id), world.rotation(id));
        // Mesh shape vertices (the shape's mesh, or the object's).
        let mesh: Option<Vec<(Vector3, Vector3)>> = {
            let wants = world
                .get_component::<ParticleSystem>(id)
                .is_some_and(|p| p.shape == ParticleShape::Mesh);
            if wants {
                let h = world
                    .get_component::<ParticleSystem>(id)
                    .and_then(|p| p.shape_options.mesh.clone())
                    .or_else(|| {
                        world
                            .get_component::<oxide_scene::MeshFilter>(id)
                            .and_then(|f| f.mesh.clone())
                    });
                h.and_then(|h: Handle<Mesh>| {
                    let m = world.try_resource::<AssetServer>()?.meshes.get(&h)?;
                    Some(
                        m.vertices
                            .iter()
                            .enumerate()
                            .map(|(i, v)| (*v, m.normals.get(i).copied().unwrap_or(Vector3::Y)))
                            .collect(),
                    )
                })
            } else {
                None
            }
        };
        let is_sub = sub_targets.contains_key(&id);
        let Some(ps) = world.component_mut::<ParticleSystem>(id, cid) else {
            continue;
        };
        let dt = if ps.use_unscaled_time {
            unscaled
        } else {
            scaled
        } * ps.simulation_speed.max(0.0);
        if dt <= 0.0 {
            continue;
        }
        if ps.state.rng == 0 {
            ps.state.rng = if ps.random_seed != 0 {
                ps.random_seed | 1
            } else {
                (id.to_bits() as u32).wrapping_mul(2654435761) | 1
            };
        }
        let emitter_velocity = ps
            .state
            .last_position
            .map_or(Vector3::ZERO, |l| (origin - l) / dt);
        ps.state.velocity = emitter_velocity;
        let env = StepEnv {
            id,
            spawn: SpawnEnv {
                origin,
                rotation: rot,
                emitter_velocity,
                mesh: mesh.as_deref(),
            },
            physics: physics.as_ref(),
            fields: &fields,
            sub_targets: &sub_targets,
            is_sub,
        };
        let mut out = StepOut::default();
        if !ps.state.started {
            ps.state.started = true;
            if ps.play_on_awake && !is_sub {
                ps.play();
                // Prewarm: one full cycle already simulated.
                if ps.prewarm && ps.looping {
                    let step_dt = 1.0 / 30.0;
                    let mut t = 0.0;
                    let mut ignored = StepOut::default();
                    while t < ps.duration {
                        step(ps, &env, step_dt, &mut ignored);
                        t += step_dt;
                    }
                }
            }
        }
        step(ps, &env, dt, &mut out);
        if out.finished {
            finished.push((id, ps.stop_action));
        }
        let tc = std::mem::take(&mut ps.state.trigger_counts);
        if tc.iter().any(|&n| n > 0) {
            triggers.push((id, tc));
        }
        subs.extend(out.subs);
        collisions.extend(out.collisions.into_iter().map(|o| (id, o)));
    }
    if let Some(p) = physics {
        world.resources.insert(p);
    }
    for (target, sub) in subs {
        let rot = world.rotation(target);
        if let Some(ps) = world.get_component_mut::<ParticleSystem>(target) {
            spawn_sub(ps, rot, &sub);
        }
    }
    use oxide_core::reflect::Value;
    for (system, other) in collisions {
        world.send_message(
            system,
            "OnParticleCollision",
            Value::ObjectRef(other.to_bits()),
        );
        if other != system {
            world.send_message(
                other,
                "OnParticleCollision",
                Value::ObjectRef(system.to_bits()),
            );
        }
    }
    for (system, counts) in triggers {
        let total: u32 = counts.iter().sum();
        world.send_message(system, "OnParticleTrigger", Value::Float(total as f64));
    }
    for (id, action) in finished {
        match action {
            StopAction::None => {}
            StopAction::Disable => world.set_active(id, false),
            StopAction::Destroy => world.destroy(id),
            StopAction::Callback => {
                world.send_message(id, "OnParticleSystemStopped", Value::Null);
            }
        }
    }
}

/// A particle's colour and size now (lifetime and speed modules applied).
fn appearance(ps: &ParticleSystem, p: &Particle) -> (Color, f32) {
    let t = (p.age / p.lifetime.max(1e-4)).clamp(0.0, 1.0);
    let g = ps.color_over_lifetime.evaluate(t);
    let mut c = Color::new(
        p.color.r * g.r,
        p.color.g * g.g,
        p.color.b * g.b,
        p.color.a * g.a,
    );
    let speed = p.velocity.length();
    let k = |r: Vector2| ((speed - r.x) / (r.y - r.x).max(1e-4)).clamp(0.0, 1.0);
    if ps.color_by_speed.enabled {
        let s = ps.color_by_speed.color.evaluate(k(ps.color_by_speed.range));
        c = Color::new(c.r * s.r, c.g * s.g, c.b * s.b, c.a * s.a);
    }
    let mut size = p.size * ps.size_over_lifetime.evaluate(t).max(0.0) * p.noise_size;
    if ps.size_by_speed.enabled {
        size *= ps
            .size_by_speed
            .size
            .evaluate(k(ps.size_by_speed.range))
            .max(0.0);
    }
    (c, size)
}

/// The texture sheet tile a particle shows (min x, min y, width, height).
fn sheet_uv(ps: &ParticleSystem, p: &Particle) -> [f32; 4] {
    let s = &ps.texture_sheet_animation;
    if !s.enabled {
        return [0.0, 0.0, 1.0, 1.0];
    }
    let (cols, rows) = (s.tiles.x.max(1.0).floor(), s.tiles.y.max(1.0).floor());
    let frames = match s.animation {
        SheetAnimation::WholeSheet => cols * rows,
        SheetAnimation::SingleRow => cols,
    };
    let t = (p.age / p.lifetime.max(1e-4)).clamp(0.0, 1.0);
    let phase = (t * s.cycles.max(0.0)).fract();
    let phase = if t >= 1.0 && s.cycles.fract() == 0.0 {
        0.999
    } else {
        phase
    };
    let f = ((s.frame_over_time.evaluate(phase).clamp(0.0, 1.0) * frames).floor() + p.sheet_start)
        .rem_euclid(frames)
        .min(frames - 1.0);
    let (col, row) = match s.animation {
        SheetAnimation::WholeSheet => (f % cols, (f / cols).floor()),
        SheetAnimation::SingleRow => (f, p.sheet_row),
    };
    [col / cols, row / rows, 1.0 / cols, 1.0 / rows]
}

/// Ribbon segments through `points` (oldest first) ending at `head`.
#[allow(clippy::too_many_arguments)]
fn trail_segments(
    trails: &TrailModule,
    points: &[[f32; 4]],
    head: Option<Vector3>,
    width: f32,
    base: Color,
    life_t: f32,
    k: f32,
    out: &mut Vec<ParticleInstance>,
) {
    let mut pts: Vec<Vector3> = points
        .iter()
        .map(|q| Vector3::new(q[0], q[1], q[2]))
        .collect();
    if let Some(h) = head
        && pts.last().is_none_or(|l| l.distance(h) > 1e-4)
    {
        pts.push(h);
    }
    let n = pts.len();
    if n < 2 {
        return;
    }
    let life = trails.color_over_lifetime.evaluate(life_t);
    let tint = if trails.inherit_particle_color {
        base
    } else {
        Color::WHITE
    };
    for i in 0..n - 1 {
        let (a, b) = (pts[i], pts[i + 1]);
        // 0 at the head (newest), 1 at the tail.
        let (u_b, u_a) = (
            (n - 2 - i) as f32 / (n - 1) as f32,
            (n - 1 - i) as f32 / (n - 1) as f32,
        );
        let um = (u_a + u_b) * 0.5;
        let w = width * trails.width_over_trail.evaluate(um).max(0.0);
        let ct = trails.color_over_trail.evaluate(um);
        let c = Color::new(
            tint.r * life.r * ct.r,
            tint.g * life.g * ct.g,
            tint.b * life.b * ct.b,
            tint.a * life.a * ct.a,
        )
        .linear();
        let mid = (a + b) * 0.5;
        out.push(
            ParticleInstance::billboard(mid.to_array(), w, [c.r * k, c.g * k, c.b * k, c.a], 0.0)
                .with_mode(ParticleMode::Segment, ((b - a) * 0.5).to_array())
                .with_uv([0.0, u_b, 1.0, u_a - u_b]),
        );
    }
}

/// Adds every system's particles (and trails) to the frame (sorted later by the renderer).
pub fn extract(world: &World, out: &mut Vec<ParticleBatch>) {
    for (id, ps) in world.query::<ParticleSystem>() {
        if ps.particles.is_empty() && ps.state.orphan_trails.is_empty() {
            continue;
        }
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let (o, r) = (world.position(id), world.rotation(id));
        let k = ps.intensity;
        let to_world = |p: Vector3| match ps.simulation_space {
            SimulationSpace::World => p,
            SimulationSpace::Local => o + r * p,
        };
        let vel_world = |v: Vector3| match ps.simulation_space {
            SimulationSpace::World => v,
            SimulationSpace::Local => r * v,
        };
        let rs = &ps.renderer;
        let blend = match ps.blend {
            BlendMode::Alpha => ParticleBlend::Alpha,
            BlendMode::Additive => ParticleBlend::Additive,
        };
        let mut trail_quads = Vec::new();
        let instances = ps
            .particles
            .iter()
            .map(|p| {
                let (c0, size) = appearance(ps, p);
                let c = c0.linear();
                let pos = to_world(p.position);
                let mut inst = ParticleInstance::billboard(
                    pos.to_array(),
                    size,
                    [c.r * k, c.g * k, c.b * k, c.a],
                    p.rotation,
                )
                .with_uv(sheet_uv(ps, p));
                match rs.render_mode {
                    ParticleRenderMode::Billboard => {}
                    ParticleRenderMode::StretchedBillboard => {
                        let v = vel_world(p.velocity);
                        let speed = v.length();
                        let half = (size * rs.length_scale + speed * rs.speed_scale) * 0.5;
                        let axis = v.normalize_or(Vector3::Y) * half;
                        inst = inst.with_mode(ParticleMode::Stretched, axis.to_array());
                    }
                    ParticleRenderMode::HorizontalBillboard => {
                        inst = inst.with_mode(ParticleMode::Horizontal, [0.0; 3]);
                    }
                    ParticleRenderMode::VerticalBillboard => {
                        inst = inst.with_mode(ParticleMode::Vertical, [0.0; 3]);
                    }
                }
                if let Some(t) = &p.trail {
                    let w = if ps.trails.size_affects_width {
                        size
                    } else {
                        1.0
                    };
                    let life_t = (p.age / p.lifetime.max(1e-4)).clamp(0.0, 1.0);
                    trail_segments(&ps.trails, t, Some(pos), w, c0, life_t, k, &mut trail_quads);
                }
                inst
            })
            .collect();
        for t in &ps.state.orphan_trails {
            trail_segments(
                &ps.trails,
                &t.points,
                None,
                t.width,
                t.color,
                1.0,
                k,
                &mut trail_quads,
            );
        }
        out.push(ParticleBatch {
            texture: ps.texture.as_ref().map(|h| h.id()),
            blend,
            particles: instances,
        });
        if !trail_quads.is_empty() {
            out.push(ParticleBatch {
                texture: ps
                    .trails
                    .texture
                    .as_ref()
                    .or(ps.texture.as_ref())
                    .map(|h| h.id()),
                blend,
                particles: trail_quads,
            });
        }
    }
}

/// The Lights module's point lights.
pub fn extract_lights(world: &World, out: &mut Vec<PunctualLight>) {
    for (id, ps) in world.query::<ParticleSystem>() {
        let l = &ps.lights;
        if !l.enabled
            || ps.particles.is_empty()
            || !world.get(id).is_some_and(|o| o.active_in_hierarchy())
        {
            continue;
        }
        let (o, r) = (world.position(id), world.rotation(id));
        for p in ps
            .particles
            .iter()
            .filter(|p| p.light)
            .take(l.max_lights as usize)
        {
            let (c, size) = appearance(ps, p);
            let base = if l.use_particle_color {
                Color::new(c.r * l.color.r, c.g * l.color.g, c.b * l.color.b, c.a)
            } else {
                l.color
            };
            let mut intensity = l.intensity;
            if l.alpha_affects_intensity {
                intensity *= c.a;
            }
            let lin = base.linear();
            out.push(PunctualLight {
                position: match ps.simulation_space {
                    SimulationSpace::World => p.position,
                    SimulationSpace::Local => o + r * p.position,
                },
                range: l.range
                    * if l.size_affects_range {
                        size.max(0.0)
                    } else {
                        1.0
                    },
                color: [lin.r * intensity, lin.g * intensity, lin.b * intensity],
                ..Default::default()
            });
        }
    }
}

/// Built-in particle textures: soft round `builtin:particle`, noisy `builtin:smoke` and
/// `builtin:trail`.
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
    // Trails: soft across the width, solid along the length.
    let mut trail = Vec::new();
    for _y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let a = (1.0 - u * u).clamp(0.0, 1.0);
            trail.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    assets.textures.insert(
        "builtin:trail",
        Texture2D::from_rgba8("trail", size, size, trail, true),
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
    /// Line and trail meshes, in Edit mode too.
    fn always(&mut self, world: &mut World) {
        effects::update(world);
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
        let life = effect.start_delay + effect.duration + effect.start_lifetime.y + 0.1;
        self.world.add_component(id, effect);
        self.destroy_after(id, life);
        id
    }
}

pub fn install(world: &mut World) -> Box<dyn System> {
    world
        .registry
        .register::<ParticleSystem>("Effects/Particle System");
    world
        .registry
        .register::<ParticleSystemForceField>("Effects/Particle System Force Field");
    let r = &mut world.registry;
    r.register::<LineRenderer>("Effects/Line Renderer");
    r.register::<TrailRenderer>("Effects/Trail Renderer");
    r.register::<BillboardRenderer>("Rendering/Billboard Renderer");
    r.register::<DecalProjector>("Rendering/Decal Projector");
    if let Some(a) = world.try_resource_mut::<AssetServer>() {
        register_builtin_textures(a);
        effects::register_default_material(a);
        a.register_data_asset::<BillboardAsset>(
            oxide_assets::AssetTypeInfo::new("BillboardAsset", &["billboard"])
                .menu("Billboard Asset", "New Billboard", 140)
                .color(150, 210, 150),
        );
    }
    Box::new(ParticleSystemRunner)
}
