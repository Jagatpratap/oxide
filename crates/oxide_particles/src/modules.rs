//! The Particle System's optional modules (Unity's Shuriken sections), each with `enabled`.
//! Reference: <https://docs.unity3d.com/Manual/ParticleSystemModules.html>

use crate::SimulationSpace;
use oxide_assets::{Handle, Mesh, Texture2D};
use oxide_core::{AnimationCurve, Gradient, Reflect};
use oxide_math::{Color, Vector2, Vector3, Vector4};
use oxide_scene::{Component, GameObjectId};

/// Unity `ParticleSystem.Burst`.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct Burst {
    /// Seconds into each cycle of the system.
    pub time: f32,
    /// Particle count range.
    pub count: Vector2,
    /// Times it repeats (0 = for as long as the system plays).
    pub cycles: u32,
    pub interval: f32,
    /// Chance (0..1) each repetition happens.
    pub probability: f32,
}

impl Default for Burst {
    fn default() -> Self {
        Self {
            time: 0.0,
            count: Vector2::new(30.0, 30.0),
            cycles: 1,
            interval: 0.01,
            probability: 1.0,
        }
    }
}

/// What the main module does when the system finishes (Unity `ParticleSystemStopAction`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum StopAction {
    #[default]
    None,
    Disable,
    Destroy,
    /// Sends `OnParticleSystemStopped` to the object.
    Callback,
}

/// Velocity over Lifetime.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct VelocityOverLifetime {
    pub enabled: bool,
    /// Added velocity (m/s).
    pub linear: Vector3,
    pub space: SimulationSpace,
    /// Orbit speed around the system's X/Y/Z axes (radians per second).
    pub orbital: Vector3,
    /// Centre of the orbit, relative to the system.
    pub offset: Vector3,
    /// Speed away from the centre (m/s).
    pub radial: f32,
    /// Multiplies the particle's speed over its life.
    pub speed_modifier: AnimationCurve,
}

impl Default for VelocityOverLifetime {
    fn default() -> Self {
        Self {
            enabled: false,
            linear: Vector3::ZERO,
            space: SimulationSpace::Local,
            orbital: Vector3::ZERO,
            offset: Vector3::ZERO,
            radial: 0.0,
            speed_modifier: AnimationCurve::constant(0.0, 1.0, 1.0),
        }
    }
}

/// Limit Velocity over Lifetime.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct LimitVelocityOverLifetime {
    pub enabled: bool,
    /// Speed limit (m/s) over the particle's life.
    pub limit: AnimationCurve,
    /// Fraction of the speed above the limit removed each frame (0..1).
    pub dampen: f32,
    /// Linear drag (per second).
    pub drag: f32,
    pub multiply_drag_by_size: bool,
    pub multiply_drag_by_velocity: bool,
}

impl Default for LimitVelocityOverLifetime {
    fn default() -> Self {
        Self {
            enabled: false,
            limit: AnimationCurve::constant(0.0, 1.0, 1.0),
            dampen: 1.0,
            drag: 0.0,
            multiply_drag_by_size: true,
            multiply_drag_by_velocity: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum InheritVelocityMode {
    /// The emitter's velocity when the particle is born.
    #[default]
    Initial,
    /// The emitter's velocity every frame.
    Current,
}

/// Inherit Velocity: particles pick up the emitter's motion.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct InheritVelocity {
    pub enabled: bool,
    pub mode: InheritVelocityMode,
    pub multiplier: f32,
}

/// Lifetime by Emitter Speed: start lifetime scaled by how fast the emitter moves.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct LifetimeByEmitterSpeed {
    pub enabled: bool,
    /// Multiplier over the speed range (0 = range min, 1 = range max).
    pub curve: AnimationCurve,
    pub range: Vector2,
}

impl Default for LifetimeByEmitterSpeed {
    fn default() -> Self {
        Self {
            enabled: false,
            curve: AnimationCurve::linear(0.0, 1.0, 1.0, 0.0),
            range: Vector2::new(0.0, 1.0),
        }
    }
}

/// Force over Lifetime (m/s²).
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct ForceOverLifetime {
    pub enabled: bool,
    pub force: Vector3,
    pub space: SimulationSpace,
    /// A new random direction (within ±force) every frame.
    pub randomized: bool,
}

impl Default for ForceOverLifetime {
    fn default() -> Self {
        Self {
            enabled: false,
            force: Vector3::ZERO,
            space: SimulationSpace::Local,
            randomized: false,
        }
    }
}

/// Color by Speed.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct ColorBySpeed {
    pub enabled: bool,
    pub color: Gradient,
    /// Speeds mapped to the gradient's ends (m/s).
    pub range: Vector2,
}

impl Default for ColorBySpeed {
    fn default() -> Self {
        Self {
            enabled: false,
            color: Gradient::two(Color::WHITE, Color::WHITE),
            range: Vector2::new(0.0, 1.0),
        }
    }
}

/// Size by Speed.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct SizeBySpeed {
    pub enabled: bool,
    pub size: AnimationCurve,
    pub range: Vector2,
}

impl Default for SizeBySpeed {
    fn default() -> Self {
        Self {
            enabled: false,
            size: AnimationCurve::linear(0.0, 0.0, 1.0, 1.0),
            range: Vector2::new(0.0, 1.0),
        }
    }
}

/// Rotation over Lifetime: angular velocity (degrees per second) over the particle's life.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct RotationOverLifetime {
    pub enabled: bool,
    pub angular_velocity: AnimationCurve,
}

impl Default for RotationOverLifetime {
    fn default() -> Self {
        Self {
            enabled: false,
            angular_velocity: AnimationCurve::constant(0.0, 1.0, 45.0),
        }
    }
}

/// Rotation by Speed: angular velocity (degrees per second) over the speed range.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct RotationBySpeed {
    pub enabled: bool,
    pub angular_velocity: AnimationCurve,
    pub range: Vector2,
}

impl Default for RotationBySpeed {
    fn default() -> Self {
        Self {
            enabled: false,
            angular_velocity: AnimationCurve::constant(0.0, 1.0, 45.0),
            range: Vector2::new(0.0, 1.0),
        }
    }
}

/// External Forces: Particle System Force Fields in the scene push the particles.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct ExternalForces {
    pub enabled: bool,
    pub multiplier: f32,
    /// Layers of the force fields that apply.
    pub influence_mask: u32,
}

impl Default for ExternalForces {
    fn default() -> Self {
        Self {
            enabled: false,
            multiplier: 1.0,
            influence_mask: u32::MAX,
        }
    }
}

/// Noise: smooth turbulence (3D value noise, fractal).
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct NoiseModule {
    pub enabled: bool,
    /// Displacement speed (m/s).
    pub strength: f32,
    /// Detail: higher is busier.
    pub frequency: f32,
    /// How fast the noise field moves.
    pub scroll_speed: f32,
    /// Strength scales with frequency, so changing frequency keeps the look.
    pub damping: bool,
    pub octaves: u32,
    pub octave_multiplier: f32,
    pub octave_scale: f32,
    /// How much the noise moves, spins (degrees per second) and resizes particles.
    pub position_amount: f32,
    pub rotation_amount: f32,
    pub size_amount: f32,
}

impl Default for NoiseModule {
    fn default() -> Self {
        Self {
            enabled: false,
            strength: 1.0,
            frequency: 0.5,
            scroll_speed: 0.0,
            damping: true,
            octaves: 1,
            octave_multiplier: 0.5,
            octave_scale: 2.0,
            position_amount: 1.0,
            rotation_amount: 0.0,
            size_amount: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum CollisionType {
    /// Infinite planes.
    #[default]
    Planes,
    /// The physics colliders in the scene.
    World,
}

/// Collision: particles bounce off planes or scene colliders.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct CollisionModule {
    pub enabled: bool,
    pub kind: CollisionType,
    /// Planes mode: each plane's normal (xyz) and distance from the origin along it (w).
    pub planes: Vec<Vector4>,
    /// Fraction of speed lost on each hit (0..1).
    pub dampen: f32,
    /// Fraction of speed kept along the normal (0..1).
    pub bounce: f32,
    /// Fraction of total lifetime lost on each hit (0..1).
    pub lifetime_loss: f32,
    /// Particles slower than this after a hit die.
    pub min_kill_speed: f32,
    /// Particles faster than this after a hit die.
    pub max_kill_speed: f32,
    /// Collision radius as a fraction of the particle size.
    pub radius_scale: f32,
    /// World mode: layers it collides with.
    pub collides_with: u32,
    /// Sends `OnParticleCollision` to the system and the object hit.
    pub send_collision_messages: bool,
}

impl Default for CollisionModule {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: CollisionType::Planes,
            planes: vec![Vector4::new(0.0, 1.0, 0.0, 0.0)],
            dampen: 0.0,
            bounce: 1.0,
            lifetime_loss: 0.0,
            min_kill_speed: 0.0,
            max_kill_speed: 10_000.0,
            radius_scale: 1.0,
            collides_with: u32::MAX,
            send_collision_messages: false,
        }
    }
}

/// What happens to particles inside / outside / entering / exiting trigger colliders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum TriggerAction {
    #[default]
    Ignore,
    Kill,
    /// Sends `OnParticleTrigger` to the system (with the count).
    Callback,
}

/// Triggers: particle reactions to listed colliders.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct TriggerModule {
    pub enabled: bool,
    pub colliders: Vec<GameObjectId>,
    pub inside: TriggerAction,
    pub outside: TriggerAction,
    pub enter: TriggerAction,
    pub exit: TriggerAction,
    /// Particle radius as a fraction of its size.
    pub radius_scale: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum SubEmitterType {
    /// Every frame of each particle's life.
    #[default]
    Birth,
    Collision,
    Death,
    Trigger,
    /// Only from scripts (`trigger_sub_emitter`).
    Manual,
}

/// One sub-emitter: another Particle System that emits where this system's particles are
/// born, collide or die.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct SubEmitter {
    pub emitter: Option<GameObjectId>,
    pub kind: SubEmitterType,
    pub inherit_color: bool,
    pub inherit_size: bool,
    pub inherit_rotation: bool,
    pub inherit_lifetime: bool,
    /// Fraction of the parent particle's velocity the new particles get.
    pub inherit_velocity: f32,
    pub emit_probability: f32,
}

impl Default for SubEmitter {
    fn default() -> Self {
        Self {
            emitter: None,
            kind: SubEmitterType::Death,
            inherit_color: false,
            inherit_size: false,
            inherit_rotation: false,
            inherit_lifetime: false,
            inherit_velocity: 0.0,
            emit_probability: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum SheetAnimation {
    #[default]
    WholeSheet,
    SingleRow,
}

/// Texture Sheet Animation: flipbook frames from a grid of tiles.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct TextureSheetAnimation {
    pub enabled: bool,
    /// Columns and rows.
    pub tiles: Vector2,
    pub animation: SheetAnimation,
    /// Single Row: a random row per particle, else `row_index`.
    pub random_row: bool,
    pub row_index: u32,
    /// Normalized frame (0..1 of the frames) over the particle's life.
    pub frame_over_time: AnimationCurve,
    /// Start frame range (frames).
    pub start_frame: Vector2,
    /// Times the animation plays over a particle's life.
    pub cycles: f32,
}

impl Default for TextureSheetAnimation {
    fn default() -> Self {
        Self {
            enabled: false,
            tiles: Vector2::new(1.0, 1.0),
            animation: SheetAnimation::WholeSheet,
            random_row: true,
            row_index: 0,
            frame_over_time: AnimationCurve::linear(0.0, 0.0, 1.0, 1.0),
            start_frame: Vector2::ZERO,
            cycles: 1.0,
        }
    }
}

/// Lights: real point lights on some particles.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct LightsModule {
    pub enabled: bool,
    /// Fraction of particles with a light (0..1).
    pub ratio: f32,
    pub color: Color,
    pub use_particle_color: bool,
    pub range: f32,
    pub intensity: f32,
    pub size_affects_range: bool,
    pub alpha_affects_intensity: bool,
    pub max_lights: u32,
}

impl Default for LightsModule {
    fn default() -> Self {
        Self {
            enabled: false,
            ratio: 0.0,
            color: Color::WHITE,
            use_particle_color: true,
            range: 1.0,
            intensity: 1.0,
            size_affects_range: true,
            alpha_affects_intensity: true,
            max_lights: 20,
        }
    }
}

/// Trails: a ribbon behind each particle.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct TrailModule {
    pub enabled: bool,
    /// Fraction of particles with a trail (0..1).
    pub ratio: f32,
    /// Trail length in time, as a fraction of the particle's lifetime.
    pub lifetime: f32,
    /// Distance a particle moves before the trail gets a new point.
    pub minimum_vertex_distance: f32,
    /// Trails vanish with their particle (else they fade out).
    pub die_with_particles: bool,
    pub width_over_trail: AnimationCurve,
    /// Width follows the particle's size.
    pub size_affects_width: bool,
    pub inherit_particle_color: bool,
    pub color_over_lifetime: Gradient,
    /// From the head (0) to the tail (1).
    pub color_over_trail: Gradient,
    /// Empty uses the particle texture.
    pub texture: Option<Handle<Texture2D>>,
}

impl Default for TrailModule {
    fn default() -> Self {
        Self {
            enabled: false,
            ratio: 1.0,
            lifetime: 0.2,
            minimum_vertex_distance: 0.1,
            die_with_particles: true,
            width_over_trail: AnimationCurve::constant(0.0, 1.0, 1.0),
            size_affects_width: true,
            inherit_particle_color: true,
            color_over_lifetime: Gradient::two(Color::WHITE, Color::WHITE),
            color_over_trail: Gradient::two(Color::WHITE, Color::new(1.0, 1.0, 1.0, 0.0)),
            texture: Some(Handle::from_path("builtin:trail")),
        }
    }
}

/// Unity `ParticleSystemRenderMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum ParticleRenderMode {
    #[default]
    Billboard,
    StretchedBillboard,
    HorizontalBillboard,
    VerticalBillboard,
}

/// The Renderer module's settings (blend and texture sit on the system).
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct ParticleRendererSettings {
    pub render_mode: ParticleRenderMode,
    /// Stretched: length as a multiple of size, plus speed × speed scale.
    pub length_scale: f32,
    pub speed_scale: f32,
}

impl Default for ParticleRendererSettings {
    fn default() -> Self {
        Self {
            render_mode: ParticleRenderMode::Billboard,
            length_scale: 2.0,
            speed_scale: 0.0,
        }
    }
}

/// Shapes beyond the basic ones, and shape options.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct ShapeOptions {
    /// Fraction of the radius particles start in, from the edge (1 = whole volume, 0 =
    /// surface).
    pub radius_thickness: f32,
    /// Degrees of the circle / cone / donut used.
    pub arc: f32,
    /// Shape transform relative to the system.
    pub position: Vector3,
    pub rotation: Vector3,
    pub scale: Vector3,
    /// Blend of the start direction toward a random one (0..1).
    pub randomize_direction: f32,
    /// Blend toward pointing away from the shape's centre (0..1).
    pub spherize_direction: f32,
    /// Random offset added to start positions (m).
    pub randomize_position: f32,
    /// Particles start rotated to their direction of travel.
    pub align_to_direction: bool,
    /// Donut: the tube's radius.
    pub donut_radius: f32,
    /// Mesh shape: the mesh (empty uses the object's MeshFilter mesh).
    pub mesh: Option<Handle<Mesh>>,
}

impl Default for ShapeOptions {
    fn default() -> Self {
        Self {
            radius_thickness: 1.0,
            arc: 360.0,
            position: Vector3::ZERO,
            rotation: Vector3::ZERO,
            scale: Vector3::ONE,
            randomize_direction: 0.0,
            spherize_direction: 0.0,
            randomize_position: 0.0,
            align_to_direction: false,
            donut_radius: 0.2,
            mesh: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum ForceFieldShape {
    #[default]
    Sphere,
    Hemisphere,
    Cylinder,
    Box,
}

/// Unity `ParticleSystemForceField`: pushes, pulls and swirls particles of systems with
/// External Forces on.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct ParticleSystemForceField {
    pub shape: ForceFieldShape,
    /// Inner and outer extent (radius, or box half-size uses `end_range` on each axis).
    pub start_range: f32,
    pub end_range: f32,
    /// Cylinder length.
    pub length: f32,
    /// Constant force in the field's space (m/s²).
    pub direction: Vector3,
    /// Pull toward the centre (m/s²; negative pushes).
    pub gravity: f32,
    /// 0 pulls to the centre point, 1 to the outer shell.
    pub gravity_focus: f32,
    /// Swirl around the field's Y axis (m/s²).
    pub rotation_speed: f32,
    /// How strongly particles are dragged into the swirl (0..1).
    pub rotation_attraction: f32,
    /// Drag inside the field (per second).
    pub drag: f32,
}

impl Default for ParticleSystemForceField {
    fn default() -> Self {
        Self {
            shape: ForceFieldShape::Sphere,
            start_range: 0.0,
            end_range: 5.0,
            length: 5.0,
            direction: Vector3::ZERO,
            gravity: 0.0,
            gravity_focus: 0.0,
            rotation_speed: 0.0,
            rotation_attraction: 0.0,
            drag: 0.0,
        }
    }
}

impl Component for ParticleSystemForceField {
    fn allow_multiple(&self) -> bool {
        false
    }
}

impl ParticleSystemForceField {
    /// How strongly the field acts at `local` (a point in the field's space): 1 inside
    /// `start_range`, falling to 0 at `end_range`; None outside.
    pub fn falloff(&self, local: Vector3) -> Option<f32> {
        let (inner, outer) = (self.start_range.max(0.0), self.end_range.max(1e-4));
        let d = match self.shape {
            ForceFieldShape::Sphere => local.length(),
            ForceFieldShape::Hemisphere => {
                if local.y < 0.0 {
                    return None;
                }
                local.length()
            }
            ForceFieldShape::Cylinder => {
                if local.y.abs() > self.length * 0.5 {
                    return None;
                }
                Vector2::new(local.x, local.z).length()
            }
            ForceFieldShape::Box => local.x.abs().max(local.y.abs()).max(local.z.abs()),
        };
        if d > outer {
            return None;
        }
        Some(if d <= inner || outer <= inner {
            1.0
        } else {
            1.0 - (d - inner) / (outer - inner)
        })
    }
}
