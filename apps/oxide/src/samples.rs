//! Sample scenes used to exercise the engine (like Unity's template scenes).

use oxide_engine::objects::{create_default_scene, create_visual, set_local};
use oxide_engine::prelude::*;

fn mat(
    world: &mut World,
    name: &str,
    color: Color,
    metallic: f32,
    smoothness: f32,
) -> Handle<Material> {
    world.resource_mut::<AssetServer>().materials.add(Material {
        name: name.into(),
        base_color: color,
        metallic,
        smoothness,
        ..Default::default()
    })
}

fn grid_ground(world: &mut World, size: f32) -> GameObjectId {
    let ground = create_primitive(world, PrimitiveType::Plane);
    world.get_mut(ground).unwrap().name = "Ground".into();
    world.transform_mut(ground).unwrap().local_scale = Vector3::new(size / 10.0, 1.0, size / 10.0);
    let m = world.resource_mut::<AssetServer>().materials.add(Material {
        name: "Ground".into(),
        base_map: Some(Handle::from_path("builtin:grid")),
        tiling: Vector2::splat(size / 4.0),
        smoothness: 0.15,
        ..Default::default()
    });
    set_material(world, ground, m);
    ground
}

fn sun_and_camera(world: &mut World) -> GameObjectId {
    create_default_scene(world);
    let sun = world.find("Directional Light").unwrap();
    world.set_euler_angles(sun, Vector3::new(45.0, -40.0, 0.0));
    world.find("Main Camera").unwrap()
}

/// A small scene exercising lighting, shadows and materials.
pub fn demo(world: &mut World) {
    let cam = sun_and_camera(world);
    world.set_position(cam, Vector3::new(0.0, 4.0, -12.0));
    world.look_at(cam, Vector3::new(0.0, 1.0, 0.0), Vector3::Y);
    grid_ground(world, 50.0);
    let items = [
        (
            PrimitiveType::Cube,
            Vector3::new(-4.5, 0.5, 0.0),
            Color::rgb(0.9, 0.2, 0.15),
            0.0,
            0.4,
        ),
        (
            PrimitiveType::Sphere,
            Vector3::new(-1.5, 0.5, 0.0),
            Color::rgb(1.0, 0.85, 0.4),
            1.0,
            0.85,
        ),
        (
            PrimitiveType::Capsule,
            Vector3::new(1.5, 1.0, 0.0),
            Color::rgb(0.2, 0.5, 0.95),
            0.0,
            0.7,
        ),
        (
            PrimitiveType::Cylinder,
            Vector3::new(4.5, 1.0, 0.0),
            Color::rgb(0.3, 0.8, 0.35),
            0.5,
            0.5,
        ),
    ];
    for (p, pos, color, metallic, smoothness) in items {
        let id = create_primitive(world, p);
        world.transform_mut(id).unwrap().local_position = pos;
        let m = mat(world, p.name(), color, metallic, smoothness);
        set_material(world, id, m);
    }
    create_point_light(
        world,
        Vector3::new(0.0, 2.0, -2.5),
        Color::rgb(1.0, 0.5, 0.2),
        8.0,
        4.0,
    );
}

/// Builds a drivable car from primitives: Rigidbody + box collider body, four WheelColliders
/// with wheel models, and a CarController.
pub fn spawn_car(world: &mut World, position: Vector3, paint: Color) -> GameObjectId {
    let body_mat = mat(world, "Car Paint", paint, 0.6, 0.8);
    let glass = mat(world, "Glass", Color::rgb(0.05, 0.07, 0.1), 0.0, 0.95);
    let tire = mat(world, "Tire", Color::rgb(0.05, 0.05, 0.05), 0.0, 0.3);
    let rim = mat(world, "Rim", Color::rgb(0.8, 0.8, 0.85), 1.0, 0.8);
    let car = world.create_at("Car", position, Quaternion::IDENTITY);
    world.get_mut(car).unwrap().layer = 6; // Vehicle
    world.add_component(
        car,
        Rigidbody {
            mass: 1300.0,
            center_of_mass: Vector3::new(0.0, 0.25, 0.1),
            interpolation: RigidbodyInterpolation::Interpolate,
            collision_detection: CollisionDetectionMode::Continuous,
            angular_damping: 0.3,
            ..Default::default()
        },
    );
    world.add_component(
        car,
        BoxCollider {
            center: Vector3::new(0.0, 0.65, 0.0),
            size: Vector3::new(1.8, 0.6, 4.3),
            friction: 0.3,
            ..Default::default()
        },
    );
    world.add_component(
        car,
        BoxCollider {
            center: Vector3::new(0.0, 1.15, -0.3),
            size: Vector3::new(1.5, 0.45, 2.1),
            friction: 0.3,
            ..Default::default()
        },
    );
    world.add_component(car, CarController::default());
    world.add_component(car, CarAudio::default());
    let body = create_visual(world, "Body", PrimitiveType::Cube, Some(body_mat.clone()));
    world.set_parent(body, Some(car), false).unwrap();
    set_local(
        world,
        body,
        Vector3::new(0.0, 0.65, 0.0),
        Vector3::ZERO,
        Vector3::new(1.8, 0.6, 4.3),
    );
    let cabin = create_visual(world, "Cabin", PrimitiveType::Cube, Some(glass));
    world.set_parent(cabin, Some(car), false).unwrap();
    set_local(
        world,
        cabin,
        Vector3::new(0.0, 1.15, -0.3),
        Vector3::ZERO,
        Vector3::new(1.5, 0.45, 2.1),
    );
    let spoiler = create_visual(world, "Spoiler", PrimitiveType::Cube, Some(body_mat));
    world.set_parent(spoiler, Some(car), false).unwrap();
    set_local(
        world,
        spoiler,
        Vector3::new(0.0, 1.1, -2.0),
        Vector3::ZERO,
        Vector3::new(1.7, 0.06, 0.35),
    );
    for (name, x, z) in [
        ("FL", -0.85, 1.4),
        ("FR", 0.85, 1.4),
        ("RL", -0.85, -1.35),
        ("RR", 0.85, -1.35),
    ] {
        let model = world.create(&format!("Wheel Model {name}"));
        world.set_parent(model, Some(car), false).unwrap();
        let t = create_visual(world, "Tire", PrimitiveType::Cylinder, Some(tire.clone()));
        world.set_parent(t, Some(model), false).unwrap();
        set_local(
            world,
            t,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 90.0),
            Vector3::new(0.72, 0.13, 0.72),
        );
        let r = create_visual(world, "Rim", PrimitiveType::Cylinder, Some(rim.clone()));
        world.set_parent(r, Some(t), false).unwrap();
        set_local(
            world,
            r,
            Vector3::ZERO,
            Vector3::ZERO,
            Vector3::new(0.65, 1.05, 0.65),
        );
        let w = world.create(&format!("Wheel {name}"));
        world.set_parent(w, Some(car), false).unwrap();
        set_local(
            world,
            w,
            Vector3::new(x, 0.55, z),
            Vector3::ZERO,
            Vector3::ONE,
        );
        world.add_component(
            w,
            WheelCollider {
                radius: 0.36,
                suspension_distance: 0.25,
                wheel_model: Some(model),
                ..Default::default()
            },
        );
    }
    car
}

/// Vehicle test track: a 1 km ground, ramps, a slalom of cones and a drivable car.
pub fn vehicle(world: &mut World) {
    let cam = sun_and_camera(world);
    grid_ground(world, 1000.0);
    let car = spawn_car(
        world,
        Vector3::new(0.0, 0.5, 0.0),
        Color::rgb(0.85, 0.08, 0.06),
    );
    world.add_component(
        cam,
        FollowCamera {
            target: Some(car),
            ..Default::default()
        },
    );
    world.set_position(cam, Vector3::new(0.0, 3.0, -8.0));

    let concrete = mat(world, "Concrete", Color::rgb(0.6, 0.6, 0.62), 0.0, 0.2);
    for (i, z) in [40.0f32, 120.0].into_iter().enumerate() {
        let ramp = create_primitive(world, PrimitiveType::Cube);
        world.get_mut(ramp).unwrap().name = format!("Ramp {i}");
        set_local(
            world,
            ramp,
            Vector3::new(0.0, 0.9, z),
            Vector3::new(-12.0, 0.0, 0.0),
            Vector3::new(8.0, 0.4, 10.0),
        );
        set_material(world, ramp, concrete.clone());
    }
    let cone = mat(world, "Cone", Color::rgb(1.0, 0.45, 0.05), 0.0, 0.5);
    for i in 0..10 {
        let c = create_primitive(world, PrimitiveType::Cylinder);
        world.get_mut(c).unwrap().name = format!("Cone {i}");
        let x = if i % 2 == 0 { -3.0 } else { 3.0 };
        set_local(
            world,
            c,
            Vector3::new(x + 12.0, 0.4, 20.0 + i as f32 * 12.0),
            Vector3::ZERO,
            Vector3::new(0.4, 0.4, 0.4),
        );
        set_material(world, c, cone.clone());
        world.add_component(c, Rigidbody::with_mass(4.0));
    }
    // Barrier walls around a 200 m square.
    let wall = mat(world, "Barrier", Color::rgb(0.9, 0.9, 0.9), 0.0, 0.3);
    for (p, s) in [
        (Vector3::new(0.0, 0.6, 200.0), Vector3::new(400.0, 1.2, 1.0)),
        (
            Vector3::new(0.0, 0.6, -200.0),
            Vector3::new(400.0, 1.2, 1.0),
        ),
        (Vector3::new(200.0, 0.6, 0.0), Vector3::new(1.0, 1.2, 400.0)),
        (
            Vector3::new(-200.0, 0.6, 0.0),
            Vector3::new(1.0, 1.2, 400.0),
        ),
    ] {
        let w = create_primitive(world, PrimitiveType::Cube);
        world.get_mut(w).unwrap().name = "Barrier".into();
        set_local(world, w, p, Vector3::ZERO, s);
        set_material(world, w, wall.clone());
    }
}

/// Character playground: stairs, a ramp, boxes and a third-person player with orbit camera.
pub fn character(world: &mut World) {
    let cam = sun_and_camera(world);
    grid_ground(world, 200.0);
    let player_mat = mat(world, "Player", Color::rgb(0.2, 0.55, 1.0), 0.0, 0.6);
    let player = world.create_at("Player", Vector3::new(0.0, 0.1, 0.0), Quaternion::IDENTITY);
    world.get_mut(player).unwrap().tag = "Player".into();
    world.get_mut(player).unwrap().layer = 7;
    world.add_component(player, CharacterController::default());
    world.add_component(player, ThirdPersonController::default());
    let body = create_visual(world, "Body", PrimitiveType::Capsule, Some(player_mat));
    world.set_parent(body, Some(player), false).unwrap();
    set_local(
        world,
        body,
        Vector3::new(0.0, 1.0, 0.0),
        Vector3::ZERO,
        Vector3::ONE,
    );
    let visor_mat = mat(world, "Visor", Color::rgb(0.1, 0.1, 0.1), 0.2, 0.9);
    let visor = create_visual(world, "Visor", PrimitiveType::Cube, Some(visor_mat));
    world.set_parent(visor, Some(player), false).unwrap();
    set_local(
        world,
        visor,
        Vector3::new(0.0, 1.55, 0.35),
        Vector3::ZERO,
        Vector3::new(0.6, 0.2, 0.3),
    );
    world.add_component(
        cam,
        OrbitCamera {
            target: Some(player),
            ..Default::default()
        },
    );

    let stone = mat(world, "Stone", Color::rgb(0.55, 0.52, 0.48), 0.0, 0.25);
    for i in 0..10 {
        let step = create_primitive(world, PrimitiveType::Cube);
        world.get_mut(step).unwrap().name = format!("Step {i}");
        let h = 0.2 * (i + 1) as f32;
        set_local(
            world,
            step,
            Vector3::new(6.0, h / 2.0, 4.0 + i as f32 * 0.4),
            Vector3::ZERO,
            Vector3::new(3.0, h, 0.4),
        );
        set_material(world, step, stone.clone());
    }
    let ramp = create_primitive(world, PrimitiveType::Cube);
    world.get_mut(ramp).unwrap().name = "Ramp".into();
    set_local(
        world,
        ramp,
        Vector3::new(-6.0, 1.0, 8.0),
        Vector3::new(-20.0, 0.0, 0.0),
        Vector3::new(3.0, 0.3, 8.0),
    );
    set_material(world, ramp, stone.clone());
    let crate_mat = mat(world, "Crate", Color::rgb(0.65, 0.42, 0.2), 0.0, 0.3);
    for i in 0..6 {
        let c = create_primitive(world, PrimitiveType::Cube);
        world.get_mut(c).unwrap().name = format!("Crate {i}");
        set_local(
            world,
            c,
            Vector3::new(-2.0 + i as f32 * 1.1, 0.5, 12.0),
            Vector3::ZERO,
            Vector3::ONE,
        );
        set_material(world, c, crate_mat.clone());
        world.add_component(c, Rigidbody::with_mass(10.0));
    }
}
