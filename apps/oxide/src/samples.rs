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
    vehicle_hud(world, car);

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
    // A loop with two AI opponents (WaypointCircuit + CarAIControl).
    let circuit = world.create("Circuit");
    for (i, (x, z)) in [
        (-40.0, -40.0),
        (40.0, -60.0),
        (90.0, -20.0),
        (110.0, 60.0),
        (60.0, 140.0),
        (-20.0, 160.0),
        (-90.0, 110.0),
        (-110.0, 20.0),
    ]
    .into_iter()
    .enumerate()
    {
        let wp = world.create(&format!("Waypoint {i}"));
        world.set_parent(wp, Some(circuit), false).unwrap();
        world.set_position(wp, Vector3::new(x, 0.5, z));
    }
    world.add_component(circuit, WaypointCircuit::default());
    for (i, paint) in [Color::rgb(0.1, 0.3, 0.9), Color::rgb(0.1, 0.7, 0.2)]
        .into_iter()
        .enumerate()
    {
        let ai = spawn_car(
            world,
            Vector3::new(-45.0 + i as f32 * 6.0, 0.5, -35.0),
            paint,
        );
        world.get_mut(ai).unwrap().name = format!("AI Car {i}");
        world.set_euler_angles(ai, Vector3::new(0.0, 100.0, 0.0));
        world.add_component(
            ai,
            CarAIControl {
                circuit: Some(circuit),
                top_speed_kmh: 90.0 + i as f32 * 15.0,
                ..Default::default()
            },
        );
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

/// Sample HUD script: shows the target car's speed on Canvas components and a small
/// immediate-mode (OnGUI-style) help panel.
#[derive(Clone, Debug, Default, Reflect)]
pub struct CarHud {
    pub car: Option<GameObjectId>,
    pub speed_text: Option<GameObjectId>,
    pub rpm_bar: Option<GameObjectId>,
}

impl Component for CarHud {
    fn update(&mut self, ctx: &mut Ctx) {
        let Some(car) = self.car else { return };
        let (speed, gear, rpm) = ctx
            .world
            .get_component::<CarController>(car)
            .map_or((0.0, 1, 0.0), |c| {
                (c.speed_kmh(), c.current_gear(), c.engine_rpm())
            });
        if let Some(t) = self
            .speed_text
            .and_then(|id| ctx.world.get_component_mut::<UIText>(id))
        {
            let gear = if gear < 0 {
                "R".to_owned()
            } else {
                gear.to_string()
            };
            t.text = format!("{:>3.0} km/h  {gear}  {:>4.0} rpm", speed.abs(), rpm);
        }
        let pitch = ctx
            .world
            .get_component::<AudioSource>(car)
            .map_or(0.0, |a| a.pitch);
        if let Some(img) = self
            .rpm_bar
            .and_then(|id| ctx.world.get_component_mut::<UIImage>(id))
        {
            img.fill_amount = ((pitch - 0.7) / 1.9).clamp(0.0, 1.0);
            img.color = Color::lerp(
                Color::rgb(0.2, 0.9, 0.3),
                Color::rgb(1.0, 0.25, 0.1),
                img.fill_amount,
            );
        }
        if let Some(gui) = ctx.gui() {
            egui::Window::new("Controls")
                .anchor(egui::Align2::LEFT_TOP, [16.0, 16.0])
                .resizable(false)
                .collapsible(true)
                .show(&gui, |ui| {
                    ui.label("W/S: throttle / brake & reverse");
                    ui.label("A/D: steer    Space: handbrake");
                    ui.label(format!(
                        "FPS: {:.0}",
                        1.0 / ctx.time().smooth_delta_time().max(1e-4)
                    ));
                });
        }
    }
}

fn vehicle_hud(world: &mut World, car: GameObjectId) {
    let canvas = world.create("Canvas");
    world.add_component(canvas, Canvas::default());
    let speed = world.create("Speed");
    world.set_parent(speed, Some(canvas), false).unwrap();
    world.add_component(
        speed,
        UIText {
            text: "0 km/h".into(),
            font_size: 64.0,
            bold: true,
            anchor: Anchor::BottomRight,
            position: Vector2::new(-60.0, 90.0),
            alignment: TextAlign::Right,
            ..Default::default()
        },
    );
    let bar = world.create("RPM Bar");
    world.set_parent(bar, Some(canvas), false).unwrap();
    world.add_component(
        bar,
        UIImage {
            anchor: Anchor::BottomRight,
            position: Vector2::new(-60.0, 50.0),
            size: Vector2::new(320.0, 18.0),
            background: Color::new(0.0, 0.0, 0.0, 0.5),
            corner_radius: 6.0,
            fill_amount: 0.0,
            ..Default::default()
        },
    );
    let hud = world.create("HUD");
    world.add_component(
        hud,
        CarHud {
            car: Some(car),
            speed_text: Some(speed),
            rpm_bar: Some(bar),
        },
    );
}

/// Open world: a 2 km generated terrain with forests, a flattened clearing, the drivable
/// car and fog — the starting point for open-world / battle-royale maps.
pub fn open_world(world: &mut World) {
    let cam = sun_and_camera(world);
    {
        let rs = world.resource_mut::<RenderSettings>();
        rs.fog = true;
        rs.fog_density = 0.0012;
        rs.fog_color = Color::rgb(0.66, 0.74, 0.84);
    }
    let size = Vector3::new(2000.0, 220.0, 2000.0);
    let terrain = create_terrain(world, "Terrain", size, 1025, 7);
    world.set_position(terrain, Vector3::new(-1000.0, 0.0, -1000.0));
    let data_h = world
        .get_component::<Terrain>(terrain)
        .unwrap()
        .data
        .clone()
        .unwrap();
    let tree_mesh = world
        .resource_mut::<AssetServer>()
        .meshes
        .add(simple_tree_mesh(9.0));
    let bark = mat(world, "Bark", Color::rgb(0.33, 0.22, 0.14), 0.0, 0.2);
    let leaves = mat(world, "Leaves", Color::rgb(0.16, 0.36, 0.12), 0.0, 0.25);
    {
        let d = world
            .resource_mut::<AssetServer>()
            .terrains
            .get_mut(&data_h)
            .unwrap();
        // A flat clearing at the center for spawning.
        let h = d.interpolated_height(0.5, 0.5) / d.size.y;
        d.flatten(0.5, 0.5, 60.0, 50.0, h);
        d.auto_splat(0.06, 0.82, 32.0);
        d.tree_prototypes
            .push(TreePrototype::new(tree_mesh, vec![bark, leaves]));
        d.scatter_trees(0, 6000, 42, 28.0, (0.08, 0.7));
    }
    let ground_y = world
        .resource::<AssetServer>()
        .terrains
        .get(&data_h)
        .unwrap()
        .interpolated_height(0.5, 0.5);
    let car = spawn_car(
        world,
        Vector3::new(0.0, ground_y + 1.0, 0.0),
        Color::rgb(0.95, 0.75, 0.1),
    );
    vehicle_hud(world, car);
    world.add_component(cam, AudioListener);
    world.add_component(
        cam,
        FollowCamera {
            target: Some(car),
            ..Default::default()
        },
    );
    world.set_position(cam, Vector3::new(0.0, ground_y + 4.0, -9.0));
    world
        .get_component_mut::<Camera>(cam)
        .unwrap()
        .far_clip_plane = 3000.0;
    // A campfire near the spawn point.
    let fire = world.create_at(
        "Campfire",
        Vector3::new(6.0, ground_y, 8.0),
        Quaternion::IDENTITY,
    );
    world.add_component(fire, ParticleSystem::fire());
    let smoke = world.create_at(
        "Campfire Smoke",
        Vector3::new(6.0, ground_y + 1.0, 8.0),
        Quaternion::IDENTITY,
    );
    world.add_component(smoke, ParticleSystem::smoke());
    let glow = create_point_light(
        world,
        Vector3::new(6.0, ground_y + 0.8, 8.0),
        Color::rgb(1.0, 0.55, 0.2),
        10.0,
        6.0,
    );
    world.get_mut(glow).unwrap().name = "Campfire Light".into();
}

/// A city grid for occlusion culling: Occluder Static buildings hide the streets behind
/// them, and a closed Occlusion Portal (a gate) hides a walled courtyard.
pub fn city(world: &mut World) {
    let cam = sun_and_camera(world);
    grid_ground(world, 600.0);
    let walls = mat(world, "Concrete", Color::rgb(0.62, 0.6, 0.57), 0.0, 0.3);
    let props = [
        mat(world, "Red", Color::rgb(0.85, 0.2, 0.15), 0.0, 0.5),
        mat(world, "Yellow", Color::rgb(0.95, 0.8, 0.2), 0.0, 0.5),
        mat(world, "Blue", Color::rgb(0.2, 0.45, 0.9), 0.3, 0.6),
    ];
    let occluder = StaticEditorFlags::EVERYTHING;
    // 12×12 blocks of 20 m buildings with 10 m streets, centered on the origin.
    let (blocks, pitch) = (12, 30.0);
    let half = (blocks - 1) as f32 * pitch * 0.5;
    let mut rng = 0x2545_f491_u32;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        (rng % 10_000) as f32 / 10_000.0
    };
    for bz in 0..blocks {
        for bx in 0..blocks {
            let c = Vector3::new(bx as f32 * pitch - half, 0.0, bz as f32 * pitch - half);
            // Leave the center block open for the courtyard.
            if bx == blocks / 2 && bz == blocks / 2 {
                continue;
            }
            let height = 12.0 + rand() * 45.0;
            let b = create_primitive(world, PrimitiveType::Cube);
            let o = world.get_mut(b).unwrap();
            o.name = format!("Building {bx},{bz}");
            o.static_flags = occluder;
            set_local(
                world,
                b,
                c + Vector3::Y * height * 0.5,
                Vector3::ZERO,
                Vector3::new(20.0, height, 20.0),
            );
            set_material(world, b, walls.clone());
            // Props along the street corner of each block.
            for k in 0..4 {
                let p = create_primitive(
                    world,
                    if k % 2 == 0 {
                        PrimitiveType::Sphere
                    } else {
                        PrimitiveType::Cube
                    },
                );
                world.get_mut(p).unwrap().name = format!("Prop {bx},{bz},{k}");
                let at = c + Vector3::new(11.5 + rand() * 2.0, 0.6, -10.0 + k as f32 * 6.0);
                set_local(world, p, at, Vector3::ZERO, Vector3::splat(1.2));
                set_material(world, p, props[k % 3].clone());
            }
        }
    }
    // A walled courtyard in the center block with a gate (an Occlusion Portal) facing -Z.
    let mid = (blocks / 2) as f32 * pitch - half;
    let yard = Vector3::new(mid, 0.0, mid);
    for (offset, size) in [
        (Vector3::new(0.0, 4.0, 10.0), Vector3::new(20.0, 8.0, 0.6)),
        (Vector3::new(-10.0, 4.0, 0.0), Vector3::new(0.6, 8.0, 20.0)),
        (Vector3::new(10.0, 4.0, 0.0), Vector3::new(0.6, 8.0, 20.0)),
        (Vector3::new(-6.5, 4.0, -10.0), Vector3::new(7.0, 8.0, 0.6)),
        (Vector3::new(6.5, 4.0, -10.0), Vector3::new(7.0, 8.0, 0.6)),
    ] {
        let w = create_primitive(world, PrimitiveType::Cube);
        let o = world.get_mut(w).unwrap();
        o.name = "Courtyard Wall".into();
        o.static_flags = occluder;
        set_local(world, w, yard + offset, Vector3::ZERO, size);
        set_material(world, w, walls.clone());
    }
    let gate = world.create_at(
        "Gate",
        yard + Vector3::new(0.0, 4.0, -10.0),
        Quaternion::IDENTITY,
    );
    world.add_component(
        gate,
        OcclusionPortal {
            open: false,
            center: Vector3::ZERO,
            size: Vector3::new(6.0, 8.0, 0.4),
        },
    );
    // The door the portal stands for; open the portal when the door opens.
    let door = create_primitive(world, PrimitiveType::Cube);
    world.get_mut(door).unwrap().name = "Door".into();
    world.set_parent(door, Some(gate), false).unwrap();
    set_local(
        world,
        door,
        Vector3::ZERO,
        Vector3::ZERO,
        Vector3::new(6.0, 8.0, 0.6),
    );
    let wood = mat(world, "Door", Color::rgb(0.45, 0.3, 0.18), 0.0, 0.35);
    set_material(world, door, wood);
    for i in 0..9 {
        let p = create_primitive(world, PrimitiveType::Sphere);
        world.get_mut(p).unwrap().name = format!("Courtyard Prop {i}");
        let at = yard + Vector3::new((i % 3) as f32 * 5.0 - 5.0, 1.0, (i / 3) as f32 * 5.0 - 3.0);
        set_local(world, p, at, Vector3::ZERO, Vector3::splat(2.0));
        set_material(world, p, props[i % 3].clone());
    }
    // Looking down the street towards the gate.
    world.set_position(cam, yard + Vector3::new(0.0, 1.7, -14.0));
    world.look_at(cam, yard + Vector3::Y * 1.7, Vector3::Y);
    world
        .get_component_mut::<Camera>(cam)
        .unwrap()
        .far_clip_plane = 2000.0;
}
