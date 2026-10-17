//! Particle System modules, simulated headless.

use oxide_assets::AssetServer;
use oxide_core::{AnimationCurve, Time};
use oxide_math::{Color, Quaternion, Vector2, Vector3, Vector4};
use oxide_particles::*;
use oxide_scene::{GameObjectId, Phase, System, World};

struct Sim {
    world: World,
    particles: Box<dyn System>,
    physics: Box<dyn System>,
}

impl Sim {
    fn new() -> Self {
        let mut world = World::new();
        oxide_scene::register_builtin(&mut world.registry);
        world.resources.insert(Time::new());
        world
            .resources
            .insert(AssetServer::new(std::env::temp_dir()));
        world.resources.insert(oxide_scene::Timers::default());
        world.resources.insert(oxide_input::Input::new());
        let physics = oxide_physics::install(&mut world);
        let particles = install(&mut world);
        Self {
            world,
            particles,
            physics,
        }
    }

    fn step(&mut self, seconds: f32) {
        let frames = (seconds * 60.0).round() as u32;
        for _ in 0..frames {
            let w = &mut self.world;
            w.resource_mut::<Time>().begin_frame(1.0 / 60.0);
            w.run_start();
            while w.resource_mut::<Time>().next_fixed_step() {
                w.run_phase(Phase::FixedUpdate);
                self.physics.fixed_update(w);
            }
            w.run_phase(Phase::Update);
            self.physics.update(w);
            self.particles.update(w);
            w.run_phase(Phase::LateUpdate);
        }
    }

    fn system(&mut self, ps: ParticleSystem) -> GameObjectId {
        let id = self.world.create("Particles");
        self.world.add_component(id, ps);
        id
    }

    fn ps(&self, id: GameObjectId) -> &ParticleSystem {
        self.world.get_component::<ParticleSystem>(id).unwrap()
    }
}

/// A system that emits nothing on its own (tests emit or burst).
fn quiet() -> ParticleSystem {
    ParticleSystem {
        rate_over_time: 0.0,
        start_speed: Vector2::ZERO,
        start_lifetime: Vector2::new(10.0, 10.0),
        shape: ParticleShape::Point,
        ..Default::default()
    }
}

#[test]
fn bursts_repeat_with_cycles_and_probability() {
    let mut s = Sim::new();
    let a = s.system(ParticleSystem {
        bursts: vec![Burst {
            time: 0.1,
            count: Vector2::new(5.0, 5.0),
            cycles: 3,
            interval: 0.2,
            probability: 1.0,
        }],
        looping: false,
        duration: 2.0,
        ..quiet()
    });
    let never = s.system(ParticleSystem {
        bursts: vec![Burst {
            probability: 0.0,
            ..Default::default()
        }],
        ..quiet()
    });
    s.step(0.2);
    assert_eq!(s.ps(a).particle_count(), 5);
    s.step(0.5);
    assert_eq!(s.ps(a).particle_count(), 15, "three cycles");
    s.step(1.0);
    assert_eq!(s.ps(a).particle_count(), 15);
    assert_eq!(s.ps(never).particle_count(), 0);
}

#[test]
fn shapes_place_particles() {
    let mut s = Sim::new();
    let circle = s.system(ParticleSystem {
        shape: ParticleShape::Circle,
        shape_radius: 2.0,
        shape_options: ShapeOptions {
            radius_thickness: 0.0,
            ..Default::default()
        },
        ..quiet()
    });
    let donut = s.system(ParticleSystem {
        shape: ParticleShape::Donut,
        shape_radius: 3.0,
        shape_options: ShapeOptions {
            donut_radius: 0.5,
            ..Default::default()
        },
        ..quiet()
    });
    let edge = s.system(ParticleSystem {
        shape: ParticleShape::Edge,
        shape_radius: 1.5,
        ..quiet()
    });
    for id in [circle, donut, edge] {
        s.world
            .get_component_mut::<ParticleSystem>(id)
            .unwrap()
            .emit(50);
    }
    s.step(1.0 / 60.0);
    for p in &s.ps(circle).particles {
        assert!((p.position.y).abs() < 1e-4);
        let r = Vector2::new(p.position.x, p.position.z).length();
        assert!((r - 2.0).abs() < 1e-3, "on the rim (thickness 0): {r}");
    }
    for p in &s.ps(donut).particles {
        let ring = Vector2::new(p.position.x, p.position.z).length();
        let tube = Vector2::new(ring - 3.0, p.position.y).length();
        assert!(tube <= 0.5 + 1e-3, "inside the tube: {tube}");
    }
    for p in &s.ps(edge).particles {
        assert!(
            p.position.x.abs() <= 1.5 && p.position.y.abs() < 1e-4 && p.position.z.abs() < 1e-4
        );
    }
}

#[test]
fn velocity_force_and_limit_modules() {
    let mut s = Sim::new();
    let moving = s.system(ParticleSystem {
        velocity_over_lifetime: VelocityOverLifetime {
            enabled: true,
            linear: Vector3::new(2.0, 0.0, 0.0),
            ..Default::default()
        },
        ..quiet()
    });
    let pushed = s.system(ParticleSystem {
        force_over_lifetime: ForceOverLifetime {
            enabled: true,
            force: Vector3::new(0.0, 0.0, 4.0),
            ..Default::default()
        },
        limit_velocity_over_lifetime: LimitVelocityOverLifetime {
            enabled: true,
            limit: AnimationCurve::constant(0.0, 1.0, 1.0),
            dampen: 1.0,
            drag: 0.0,
            ..Default::default()
        },
        ..quiet()
    });
    for id in [moving, pushed] {
        s.world
            .get_component_mut::<ParticleSystem>(id)
            .unwrap()
            .emit(1);
    }
    s.step(1.0);
    let p = &s.ps(moving).particles[0];
    assert!(
        (p.position.x - 2.0).abs() < 0.1,
        "2 m/s for 1 s: {}",
        p.position.x
    );
    let q = &s.ps(pushed).particles[0];
    assert!(
        q.velocity.length() <= 1.0 + 1e-3,
        "limited to 1 m/s: {}",
        q.velocity.length()
    );
    assert!(q.position.z > 0.5);
}

#[test]
fn collision_planes_bounce_and_world_colliders_stop_particles() {
    let mut s = Sim::new();
    let planes = s.system(ParticleSystem {
        gravity_modifier: 1.0,
        start_size: Vector2::new(0.2, 0.2),
        collision: CollisionModule {
            enabled: true,
            kind: CollisionType::Planes,
            planes: vec![Vector4::new(0.0, 1.0, 0.0, 0.0)],
            bounce: 0.5,
            ..Default::default()
        },
        ..quiet()
    });
    s.world.transform_mut(planes).unwrap().local_position = Vector3::new(0.0, 2.0, 0.0);
    // A floor collider for World collision.
    let floor = s.world.create("Floor");
    s.world.transform_mut(floor).unwrap().local_position = Vector3::new(10.0, -0.5, 0.0);
    s.world.transform_mut(floor).unwrap().local_scale = Vector3::new(5.0, 1.0, 5.0);
    s.world
        .add_component(floor, oxide_physics::BoxCollider::default());
    let world_mode = s.system(ParticleSystem {
        gravity_modifier: 1.0,
        start_size: Vector2::new(0.2, 0.2),
        collision: CollisionModule {
            enabled: true,
            kind: CollisionType::World,
            bounce: 0.0,
            send_collision_messages: true,
            ..Default::default()
        },
        ..quiet()
    });
    s.world.transform_mut(world_mode).unwrap().local_position = Vector3::new(10.0, 2.0, 0.0);
    s.step(0.1);
    for id in [planes, world_mode] {
        s.world
            .get_component_mut::<ParticleSystem>(id)
            .unwrap()
            .emit(1);
    }
    let mut lowest = f32::MAX;
    let mut went_up = false;
    let mut last = 2.0;
    for _ in 0..120 {
        s.step(1.0 / 60.0);
        let y = s.ps(planes).particles[0].position.y;
        lowest = lowest.min(y);
        if y > last + 1e-4 {
            went_up = true;
        }
        last = y;
    }
    assert!(
        lowest >= 0.1 - 1e-3,
        "stays above the plane by its radius: {lowest}"
    );
    assert!(went_up, "bounced");
    let w = &s.ps(world_mode).particles[0];
    assert!(
        (w.position.y - 0.1).abs() < 0.05,
        "rests on the collider: {}",
        w.position.y
    );
}

#[test]
fn death_sub_emitters_spawn_children() {
    let mut s = Sim::new();
    let child = s.system(ParticleSystem {
        burst_count: 4,
        play_on_awake: false,
        start_lifetime: Vector2::new(5.0, 5.0),
        ..quiet()
    });
    let parent = s.system(ParticleSystem {
        start_lifetime: Vector2::new(0.3, 0.3),
        start_color: Color::rgb(1.0, 0.0, 0.0),
        start_color_b: Color::rgb(1.0, 0.0, 0.0),
        sub_emitters: vec![SubEmitter {
            emitter: Some(child),
            kind: SubEmitterType::Death,
            inherit_color: true,
            ..Default::default()
        }],
        ..quiet()
    });
    s.world.transform_mut(parent).unwrap().local_position = Vector3::new(3.0, 0.0, 0.0);
    s.world
        .get_component_mut::<ParticleSystem>(parent)
        .unwrap()
        .emit(2);
    s.step(0.2);
    assert_eq!(
        s.ps(child).particle_count(),
        0,
        "a sub-emitter doesn't emit by itself"
    );
    s.step(0.3);
    assert_eq!(s.ps(parent).particle_count(), 0);
    let kids = &s.ps(child).particles;
    assert_eq!(kids.len(), 8, "a 4-particle burst per death");
    assert!(
        kids.iter().all(|k| (k.position.x - 3.0).abs() < 1e-3),
        "at the parent particles"
    );
    assert!(kids.iter().all(|k| k.color.g == 0.0), "inherited red");
}

#[test]
fn texture_sheets_trails_lights_and_render_modes_extract() {
    let mut s = Sim::new();
    let id = s.system(ParticleSystem {
        start_speed: Vector2::new(3.0, 3.0),
        texture_sheet_animation: TextureSheetAnimation {
            enabled: true,
            tiles: Vector2::new(4.0, 2.0),
            ..Default::default()
        },
        trails: TrailModule {
            enabled: true,
            minimum_vertex_distance: 0.05,
            lifetime: 1.0,
            ..Default::default()
        },
        lights: LightsModule {
            enabled: true,
            ratio: 1.0,
            range: 2.0,
            max_lights: 1,
            ..Default::default()
        },
        renderer: ParticleRendererSettings {
            render_mode: ParticleRenderMode::StretchedBillboard,
            length_scale: 2.0,
            speed_scale: 0.5,
        },
        shape: ParticleShape::Cone,
        shape_angle: 0.0,
        shape_radius: 0.0001,
        ..quiet()
    });
    s.world
        .get_component_mut::<ParticleSystem>(id)
        .unwrap()
        .emit(2);
    s.step(5.1);
    let mut batches = Vec::new();
    extract(&s.world, &mut batches);
    assert_eq!(batches.len(), 2, "particles and their trails");
    let p = &batches[0].particles[0];
    // Just past halfway through a 10 s life: frame 4 of 8 = column 0, row 1.
    assert_eq!(p.uv, [0.0, 0.5, 0.25, 0.5]);
    assert_eq!(p.mode, oxide_render::ParticleMode::Stretched as u8 as f32);
    let half = Vector3::from_array(p.axis).length();
    assert!(
        (half - (1.0 * 2.0 + 3.0 * 0.5) * 0.5).abs() < 1e-3,
        "{half}"
    );
    let trail = &batches[1].particles;
    assert!(trail.len() > 10, "{}", trail.len());
    assert!(
        trail
            .iter()
            .all(|q| q.mode == oxide_render::ParticleMode::Segment as u8 as f32)
    );
    let mut lights = Vec::new();
    extract_lights(&s.world, &mut lights);
    assert_eq!(lights.len(), 1, "max lights");
    assert!((lights[0].range - 2.0).abs() < 1e-3);
}

#[test]
fn force_fields_pull_particles_in() {
    let mut s = Sim::new();
    let field = s.world.create("Field");
    s.world.add_component(
        field,
        ParticleSystemForceField {
            end_range: 10.0,
            gravity: 5.0,
            ..Default::default()
        },
    );
    let id = s.system(ParticleSystem {
        external_forces: ExternalForces {
            enabled: true,
            ..Default::default()
        },
        ..quiet()
    });
    s.world.transform_mut(id).unwrap().local_position = Vector3::new(4.0, 0.0, 0.0);
    s.world
        .get_component_mut::<ParticleSystem>(id)
        .unwrap()
        .emit(1);
    s.step(0.5);
    let p = &s.ps(id).particles[0];
    assert!(
        p.velocity.x < -1.0,
        "pulled toward the field: {:?}",
        p.velocity
    );
}

#[test]
fn prewarm_delay_and_stop_actions() {
    let mut s = Sim::new();
    let warm = s.system(ParticleSystem {
        prewarm: true,
        rate_over_time: 10.0,
        duration: 2.0,
        ..quiet()
    });
    let delayed = s.system(ParticleSystem {
        start_delay: 0.5,
        rate_over_time: 60.0,
        ..quiet()
    });
    let once = s.system(ParticleSystem {
        looping: false,
        duration: 0.2,
        burst_count: 3,
        start_lifetime: Vector2::new(0.3, 0.3),
        stop_action: StopAction::Destroy,
        ..quiet()
    });
    s.step(1.0 / 60.0);
    assert!(
        s.ps(warm).particle_count() >= 18,
        "{}",
        s.ps(warm).particle_count()
    );
    s.step(0.3);
    assert_eq!(s.ps(delayed).particle_count(), 0, "still waiting");
    s.step(0.4);
    assert!(s.ps(delayed).particle_count() > 5);
    s.step(1.0);
    assert!(s.world.get(once).is_none(), "destroyed when finished");
}

#[test]
fn noise_module_moves_particles_smoothly() {
    let mut s = Sim::new();
    let id = s.system(ParticleSystem {
        noise_module: NoiseModule {
            enabled: true,
            strength: 2.0,
            frequency: 0.5,
            ..Default::default()
        },
        ..quiet()
    });
    s.world
        .get_component_mut::<ParticleSystem>(id)
        .unwrap()
        .emit(20);
    s.step(1.0 / 60.0);
    let before: Vec<Vector3> = s.ps(id).particles.iter().map(|p| p.position).collect();
    s.step(1.0);
    let moved = s
        .ps(id)
        .particles
        .iter()
        .zip(&before)
        .filter(|(p, b)| p.position.distance(**b) > 0.05)
        .count();
    assert!(moved > 10, "{moved} of 20 moved");
    let _ = Quaternion::IDENTITY;
}
