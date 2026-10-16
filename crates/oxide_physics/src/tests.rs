use crate::*;
use oxide_assets::AssetServer;
use oxide_core::{Reflect, Time};
use oxide_math::{Quaternion, QuaternionExt, Vector3};
use oxide_scene::{Component, Ctx, GameObjectId, Phase, System, Timers, World};
use std::sync::atomic::{AtomicU32, Ordering};

struct Sim {
    world: World,
    sys: Box<dyn System>,
}

impl Sim {
    fn new() -> Self {
        let mut world = World::new();
        oxide_scene::register_builtin(&mut world.registry);
        world.resources.insert(Time::new());
        world.resources.insert(AssetServer::new("."));
        world.resources.insert(Timers::default());
        world.resources.insert(oxide_input::Input::new());
        let sys = install(&mut world);
        Self { world, sys }
    }

    fn step(&mut self, seconds: f32) {
        let frames = (seconds * 60.0).round() as u32;
        for _ in 0..frames {
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

    fn ground(&mut self) -> GameObjectId {
        self.ground_size(2000.0)
    }

    fn ground_size(&mut self, size: f32) -> GameObjectId {
        let g = self.world.create("Ground");
        self.world.transform_mut(g).unwrap().local_scale = Vector3::new(size, 1.0, size);
        self.world.transform_mut(g).unwrap().local_position = Vector3::new(0.0, -0.5, 0.0);
        self.world.add_component(g, BoxCollider::default());
        g
    }
}

#[test]
fn box_falls_and_rests_on_ground() {
    let mut s = Sim::new();
    s.ground();
    let b = s
        .world
        .create_at("Box", Vector3::new(0.0, 5.0, 0.0), Quaternion::IDENTITY);
    s.world.add_component(b, BoxCollider::default());
    s.world.add_component(b, Rigidbody::with_mass(2.0));
    s.step(3.0);
    let y = s.world.position(b).y;
    assert!((y - 0.5).abs() < 0.05, "box rests at y={y}");
    let phys = s.world.resource::<Physics>();
    let mass = phys
        .world
        .bodies
        .get(phys.bodies[&b].handle)
        .unwrap()
        .mass();
    assert!((mass - 2.0).abs() < 1e-3, "mass {mass}");
    let hit = phys
        .raycast(
            Vector3::new(0.0, 10.0, 0.0),
            Vector3::NEG_Y,
            100.0,
            DEFAULT_RAYCAST_LAYERS,
        )
        .unwrap();
    assert_eq!(hit.game_object, b);
    assert!((hit.point.y - 1.0).abs() < 0.05 && hit.normal.y > 0.9);
}

static TRIGGERS: AtomicU32 = AtomicU32::new(0);
static COLLISIONS: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Default, Reflect)]
struct Counter;
impl Component for Counter {
    fn on_trigger_enter(&mut self, _: &mut Ctx, _: GameObjectId) {
        TRIGGERS.fetch_add(1, Ordering::SeqCst);
    }
    fn on_collision_enter(&mut self, _: &mut Ctx, c: &oxide_scene::Collision) {
        assert!(!c.contacts.is_empty());
        COLLISIONS.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn trigger_and_collision_messages() {
    let mut s = Sim::new();
    s.ground();
    let zone = s
        .world
        .create_at("Zone", Vector3::new(0.0, 2.0, 0.0), Quaternion::IDENTITY);
    s.world.add_component(
        zone,
        BoxCollider {
            is_trigger: true,
            size: Vector3::splat(2.0),
            ..Default::default()
        },
    );
    s.world.add_component(zone, Counter);
    let ball = s
        .world
        .create_at("Ball", Vector3::new(0.0, 6.0, 0.0), Quaternion::IDENTITY);
    s.world.add_component(ball, SphereCollider::default());
    s.world.add_component(ball, Rigidbody::default());
    s.world.add_component(ball, Counter);
    s.step(3.0);
    assert!(
        TRIGGERS.load(Ordering::SeqCst) >= 2,
        "trigger enter on zone and ball"
    );
    assert!(
        COLLISIONS.load(Ordering::SeqCst) >= 1,
        "ball hit the ground"
    );
}

#[test]
fn character_controller_walks_and_is_blocked() {
    let mut s = Sim::new();
    s.ground_size(100.0);
    let wall = s
        .world
        .create_at("Wall", Vector3::new(0.0, 1.0, 5.0), Quaternion::IDENTITY);
    s.world.add_component(
        wall,
        BoxCollider {
            size: Vector3::new(10.0, 2.0, 1.0),
            ..Default::default()
        },
    );
    let p = s
        .world
        .create_at("Player", Vector3::new(0.0, 0.0, 0.0), Quaternion::IDENTITY);
    s.world.add_component(p, CharacterController::default());
    s.step(0.1);
    for _ in 0..120 {
        move_character(&mut s.world, p, Vector3::new(0.0, -0.2, 0.1));
        s.step(1.0 / 60.0);
    }
    let pos = s.world.position(p);
    assert!(pos.z < 4.5 && pos.z > 3.5, "stopped at the wall: {pos}");
    assert!(pos.y.abs() < 0.1, "stays on the ground: {pos}");
    assert!(
        s.world
            .get_component::<CharacterController>(p)
            .unwrap()
            .is_grounded
    );
}

fn car(s: &mut Sim) -> (GameObjectId, Vec<GameObjectId>) {
    let car = s
        .world
        .create_at("Car", Vector3::new(0.0, 1.2, 0.0), Quaternion::IDENTITY);
    s.world.add_component(
        car,
        BoxCollider {
            size: Vector3::new(1.8, 0.6, 4.0),
            ..Default::default()
        },
    );
    s.world.add_component(
        car,
        Rigidbody {
            mass: 1200.0,
            center_of_mass: Vector3::new(0.0, -0.3, 0.0),
            ..Default::default()
        },
    );
    let mut wheels = Vec::new();
    for (x, z) in [(-0.9, 1.3), (0.9, 1.3), (-0.9, -1.3), (0.9, -1.3)] {
        let w = s
            .world
            .create_at("Wheel", Vector3::new(x, 0.0, z), Quaternion::IDENTITY);
        s.world.set_parent(w, Some(car), false).unwrap();
        s.world.add_component(w, WheelCollider::default());
        wheels.push(w);
    }
    (car, wheels)
}

#[test]
fn wheel_collider_car_drives_and_steers() {
    let mut s = Sim::new();
    s.ground();
    let (car, wheels) = car(&mut s);
    s.step(1.0);
    let rest_y = s.world.position(car).y;
    assert!(
        rest_y > 0.3 && rest_y < 1.2,
        "car rests on its suspension: {rest_y}"
    );
    assert!(wheels.iter().all(|&w| {
        s.world
            .get_component::<WheelCollider>(w)
            .unwrap()
            .is_grounded()
    }));
    for &w in &wheels {
        s.world
            .get_component_mut::<WheelCollider>(w)
            .unwrap()
            .motor_torque = 400.0;
    }
    s.step(3.0);
    let p = s.world.position(car);
    assert!(p.z > 5.0, "drove forward (+Z): {p}");
    assert!(p.x.abs() < 1.0, "straight: {p}");
    let rpm = s
        .world
        .get_component::<WheelCollider>(wheels[0])
        .unwrap()
        .rpm();
    assert!(rpm > 10.0, "wheels spin forward: {rpm}");
    for &w in &wheels[..2] {
        s.world
            .get_component_mut::<WheelCollider>(w)
            .unwrap()
            .steer_angle = 25.0;
    }
    // Real tyres need a moment to build slip angle (the old model turned instantly).
    s.step(1.0);
    let p2 = s.world.position(car);
    assert!(
        p2.x > p.x + 1.0,
        "positive steer turns right (+X): {p} -> {p2}"
    );
    let yaw = s.world.rotation(car).euler_angles().y;
    assert!(yaw > 10.0 && yaw < 120.0, "yawed right: {yaw}");
}

/// Unity's WheelFrictionCurve tyre model: too much torque spins the wheels, a full brake
/// locks them, and low-grip (icy) tyres slide further.
#[test]
fn wheel_friction_curves_spin_lock_and_slide() {
    let wheel =
        |s: &Sim, w: GameObjectId| s.world.get_component::<WheelCollider>(w).unwrap().clone();
    // Wheelspin: 3000 N·m per wheel is far beyond what 1200 kg can put down.
    let mut s = Sim::new();
    s.ground();
    let (car, wheels) = car(&mut s);
    s.step(1.0);
    let load = wheel(&s, wheels[0]).load();
    assert!(
        load > 1500.0 && load < 4500.0,
        "about a quarter of the weight: {load}"
    );
    for &w in &wheels {
        s.world
            .get_component_mut::<WheelCollider>(w)
            .unwrap()
            .motor_torque = 3000.0;
    }
    s.step(0.3);
    let (fs, _) = wheel(&s, wheels[0]).slip();
    assert!(fs > 0.5, "wheelspin shows as positive forward slip: {fs}");
    let tread = wheel(&s, wheels[0]).state.angular_velocity * 0.35;
    let speed = s.world.position(car).z / 0.3;
    assert!(tread > speed * 1.5, "tread {tread} m/s outruns the car");

    // Locking: at speed, a huge brake stops the wheels and the car skids to a halt.
    for &w in &wheels {
        let wc = s.world.get_component_mut::<WheelCollider>(w).unwrap();
        wc.motor_torque = 0.0;
        wc.brake_torque = 50_000.0;
    }
    s.step(0.2);
    let wc = wheel(&s, wheels[0]);
    assert!(wc.rpm().abs() < 1.0, "locked: {} rpm", wc.rpm());
    assert!(
        wc.slip().0 < -0.5,
        "locking shows as negative slip: {}",
        wc.slip().0
    );
    s.step(4.0);
    let v = s
        .world
        .get_component::<Rigidbody>(car)
        .unwrap()
        .velocity()
        .length();
    assert!(v < 0.2, "skidded to a stop: {v} m/s");

    // Ice vs tarmac: the same steering input at the same speed turns far less on ice.
    let turn = |stiffness: f32| {
        let mut s = Sim::new();
        s.ground();
        let (body, wheels) = self::car(&mut s);
        let car = body;
        for &w in &wheels {
            let wc = s.world.get_component_mut::<WheelCollider>(w).unwrap();
            wc.sideways_friction.stiffness = stiffness;
            wc.forward_friction.stiffness = stiffness;
        }
        s.step(0.5);
        // Launch at 15 m/s and coast with the wheels turned.
        s.world
            .get_component_mut::<Rigidbody>(car)
            .unwrap()
            .set_velocity(Vector3::new(0.0, 0.0, 15.0));
        for &w in &wheels {
            let wc = s.world.get_component_mut::<WheelCollider>(w).unwrap();
            wc.state.angular_velocity = 15.0 / 0.35;
        }
        for &w in &wheels[..2] {
            s.world
                .get_component_mut::<WheelCollider>(w)
                .unwrap()
                .steer_angle = 15.0;
        }
        s.step(1.0);
        s.world.rotation(car).euler_angles().y
    };
    let tarmac = turn(1.0);
    let ice = turn(0.1);
    assert!(tarmac > 15.0, "tarmac turns: {tarmac}°");
    assert!(
        ice < tarmac * 0.5,
        "ice turns much less: {ice}° vs {tarmac}°"
    );
}

/// Big flat worlds (BGMI-size maps) use mesh/heightfield ground, like Unity's Plane and
/// Terrain; the character must stay on it far from the origin.
#[test]
fn character_on_8km_mesh_ground() {
    let mut s = Sim::new();
    let g = s.world.create("Plane");
    s.world.transform_mut(g).unwrap().local_scale = Vector3::new(800.0, 1.0, 800.0);
    s.world.add_component(
        g,
        oxide_scene::MeshFilter {
            mesh: Some(oxide_assets::Handle::from_path("builtin:Plane")),
        },
    );
    s.world.add_component(g, MeshCollider::default());
    let p = s.world.create_at(
        "Player",
        Vector3::new(3000.0, 0.0, 3000.0),
        Quaternion::IDENTITY,
    );
    s.world.add_component(p, CharacterController::default());
    s.step(0.1);
    for _ in 0..60 {
        move_character(&mut s.world, p, Vector3::new(0.0, -0.2, 0.1));
        s.step(1.0 / 60.0);
    }
    let pos = s.world.position(p);
    assert!(pos.y.abs() < 0.05 && (pos.z - 3006.0).abs() < 0.5, "{pos}");
}

/// A T-pose skeleton of empty bones facing +Z (left = -X), pelvis at `y`.
fn skeleton(w: &mut World, y: f32) -> (GameObjectId, RagdollBones) {
    let root = w.create("Character");
    let bone = |w: &mut World, name: &str, parent: GameObjectId, p: Vector3| {
        let id = w.create_at(name, p + Vector3::Y * (y - 1.0), Quaternion::IDENTITY);
        w.set_parent(id, Some(parent), true).unwrap();
        id
    };
    let pelvis = bone(w, "Hips", root, Vector3::new(0.0, 1.0, 0.0));
    let spine = bone(w, "Spine", pelvis, Vector3::new(0.0, 1.25, 0.0));
    let head = bone(w, "Head", spine, Vector3::new(0.0, 1.6, 0.0));
    let side = |w: &mut World, s: f32, n: &str| {
        let hips = bone(
            w,
            &format!("{n}UpLeg"),
            pelvis,
            Vector3::new(0.1 * s, 0.95, 0.0),
        );
        let knee = bone(w, &format!("{n}Leg"), hips, Vector3::new(0.1 * s, 0.5, 0.0));
        let foot = bone(
            w,
            &format!("{n}Foot"),
            knee,
            Vector3::new(0.1 * s, 0.08, 0.0),
        );
        let arm = bone(
            w,
            &format!("{n}Arm"),
            spine,
            Vector3::new(0.2 * s, 1.45, 0.0),
        );
        let elbow = bone(
            w,
            &format!("{n}ForeArm"),
            arm,
            Vector3::new(0.5 * s, 1.45, 0.0),
        );
        let hand = bone(
            w,
            &format!("{n}Hand"),
            elbow,
            Vector3::new(0.8 * s, 1.45, 0.0),
        );
        (hips, knee, foot, arm, elbow, hand)
    };
    let l = side(w, -1.0, "Left");
    let r = side(w, 1.0, "Right");
    (
        root,
        RagdollBones {
            pelvis,
            left_hips: l.0,
            left_knee: l.1,
            left_foot: l.2,
            right_hips: r.0,
            right_knee: r.1,
            right_foot: r.2,
            left_arm: l.3,
            left_elbow: l.4,
            left_hand: Some(l.5),
            right_arm: r.3,
            right_elbow: r.4,
            right_hand: Some(r.5),
            middle_spine: spine,
            head,
        },
    )
}

#[test]
fn ragdoll_falls_and_settles_in_one_piece() {
    let mut s = Sim::new();
    s.ground();
    let (root, b) = skeleton(&mut s.world, 2.0);
    let bodies = build_ragdoll(&mut s.world, &b, 70.0, true).unwrap();
    assert_eq!(bodies.len(), 11);
    // Kinematic: nothing moves.
    s.step(0.5);
    assert!((s.world.position(b.pelvis).y - 2.0).abs() < 1e-3);
    // Go limp.
    assert_eq!(set_ragdoll_kinematic(&mut s.world, root, false), 11);
    s.step(4.0);
    let p = s.world.position(b.pelvis);
    assert!(p.y < 0.5 && p.y > 0.0, "pelvis on the ground: {p}");
    for &id in &bodies {
        let q = s.world.position(id);
        assert!(
            q.distance(p) < 1.5,
            "bone stayed attached: {q} vs pelvis {p}"
        );
        let v = s.world.get_component::<Rigidbody>(id).unwrap().velocity;
        assert!(v.length() < 0.5, "settled: {v}");
    }
    // Joints hold: bones keep their lengths.
    let len = |a, c| s.world.position(a).distance(s.world.position(c));
    assert!((len(b.left_hips, b.left_knee) - 0.45).abs() < 0.05);
    assert!((len(b.right_arm, b.right_elbow) - 0.3).abs() < 0.05);
    // Knees never bend forward: the foot stays behind the thigh's line (in the thigh's
    // own frame, the knee joint only rotates one way).
    for (hips, knee, foot) in [
        (b.left_hips, b.left_knee, b.left_foot),
        (b.right_hips, b.right_knee, b.right_foot),
    ] {
        let (h, k, f) = (
            s.world.position(hips),
            s.world.position(knee),
            s.world.position(foot),
        );
        let thigh = (k - h).normalize();
        let shin = (f - k).normalize();
        let right = s.world.right(hips);
        let bend = thigh.cross(shin).dot(right);
        assert!(bend > -0.2, "knee bent the wrong way: {bend}");
    }
}
