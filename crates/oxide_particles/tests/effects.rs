//! Line / Trail Renderers, Decal Projectors and Billboards.

use oxide_assets::{AssetServer, Material};
use oxide_core::{AnimationCurve, Gradient, Time};
use oxide_math::{Color, Quaternion, Vector2, Vector3, Vector4};
use oxide_particles::effects::*;
use oxide_particles::*;
use oxide_scene::{System, World};

fn world() -> (World, Box<dyn System>) {
    let mut w = World::new();
    oxide_scene::register_builtin(&mut w.registry);
    w.resources.insert(Time::new());
    w.resources.insert(AssetServer::new(std::env::temp_dir()));
    let sys = install(&mut w);
    (w, sys)
}

fn frame(w: &mut World, sys: &mut Box<dyn System>, dt: f64) {
    w.resource_mut::<Time>().begin_frame(dt);
    sys.always(w);
}

fn pt(x: f32, width: f32) -> RibbonPoint {
    RibbonPoint {
        position: Vector3::new(x, 0.0, 0.0),
        width,
        color: Vector4::ONE,
    }
}

#[test]
fn ribbons_are_joined_quads_with_caps_and_corners() {
    let style = RibbonStyle {
        facing: RibbonFacing::Normal(Vector3::Z),
        looped: false,
        corner_vertices: 0,
        end_cap_vertices: 0,
        texture_mode: LineTextureMode::Stretch,
        texture_scale: Vector2::ONE,
    };
    let m = ribbon_mesh(&[pt(0.0, 1.0), pt(1.0, 1.0), pt(3.0, 0.5)], &style);
    assert_eq!(m.vertices.len(), 6);
    assert_eq!(m.indices.len(), 12, "two joined quads");
    // Facing +Z, the width runs along Y.
    assert!((m.vertices[0].y - m.vertices[1].y).abs() - 1.0 < 1e-4);
    assert!(((m.vertices[4] - m.vertices[5]).length() - 0.5).abs() < 1e-4);
    let us: Vec<f32> = m.uv.iter().step_by(2).map(|u| u.x).collect();
    assert_eq!(us, [0.0, 1.0 / 3.0, 1.0]);
    // Tile repeats per world unit.
    let tiled = ribbon_mesh(
        &[pt(0.0, 1.0), pt(3.0, 1.0)],
        &RibbonStyle {
            texture_mode: LineTextureMode::Tile,
            ..style
        },
    );
    assert_eq!(tiled.uv[2].x, 3.0);
    // End caps add fans; corners add vertices at a bend.
    let capped = ribbon_mesh(
        &[pt(0.0, 1.0), pt(1.0, 1.0)],
        &RibbonStyle {
            end_cap_vertices: 4,
            ..style
        },
    );
    assert_eq!(capped.vertices.len(), 4 + 2 * (1 + 6));
    let bend = [
        pt(0.0, 1.0),
        RibbonPoint {
            position: Vector3::new(1.0, 0.0, 0.0),
            width: 1.0,
            color: Vector4::ONE,
        },
        RibbonPoint {
            position: Vector3::new(1.0, 1.0, 0.0),
            width: 1.0,
            color: Vector4::ONE,
        },
    ];
    let sharp = ribbon_mesh(&bend, &style);
    let round = ribbon_mesh(
        &bend,
        &RibbonStyle {
            corner_vertices: 3,
            ..style
        },
    );
    assert!(round.vertices.len() > sharp.vertices.len());
    // A loop closes back to the start.
    let square = [
        pt(0.0, 0.1),
        pt(1.0, 0.1),
        RibbonPoint {
            position: Vector3::new(1.0, 1.0, 0.0),
            ..pt(0.0, 0.1)
        },
    ];
    let looped = ribbon_mesh(
        &square,
        &RibbonStyle {
            looped: true,
            ..style
        },
    );
    assert_eq!(looped.indices.len(), 18, "three segments");
}

#[test]
fn line_renderers_build_meshes_and_draws() {
    let (mut w, mut sys) = world();
    let id = w.create("Line");
    w.add_component(
        id,
        LineRenderer {
            positions: vec![Vector3::ZERO, Vector3::new(4.0, 0.0, 0.0)],
            width_multiplier: 0.5,
            color: Gradient::two(Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0)),
            alignment: LineAlignment::TransformZ,
            ..Default::default()
        },
    );
    frame(&mut w, &mut sys, 1.0 / 60.0);
    let mut draws = Vec::new();
    extract_draws(&w, &mut draws);
    assert_eq!(draws.len(), 1);
    assert_eq!(
        draws[0].material,
        oxide_assets::AssetId::from_path(DEFAULT_LINE_MATERIAL)
    );
    let assets = w.resource::<AssetServer>();
    let mesh = assets.meshes.get_by_id(draws[0].mesh).unwrap();
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.colors[0].x, 1.0, "red start");
    assert_eq!(mesh.colors[3].z, 1.0, "blue end");
    // Local positions follow the object.
    w.get_component_mut::<LineRenderer>(id)
        .unwrap()
        .use_world_space = false;
    w.transform_mut(id).unwrap().local_position = Vector3::new(0.0, 5.0, 0.0);
    frame(&mut w, &mut sys, 1.0 / 60.0);
    let assets = w.resource::<AssetServer>();
    let mesh = assets.meshes.get_by_id(draws[0].mesh).unwrap();
    assert!(mesh.vertices.iter().all(|v| (v.y - 5.0).abs() <= 0.26));
}

#[test]
fn trails_follow_and_fade_and_autodestruct() {
    let (mut w, mut sys) = world();
    let id = w.create("Car");
    w.add_component(
        id,
        TrailRenderer {
            time: 0.5,
            min_vertex_distance: 0.1,
            autodestruct: true,
            width_curve: AnimationCurve::linear(0.0, 1.0, 1.0, 0.0),
            ..Default::default()
        },
    );
    for i in 0..30 {
        w.transform_mut(id).unwrap().local_position = Vector3::new(i as f32 * 0.2, 0.0, 0.0);
        frame(&mut w, &mut sys, 1.0 / 60.0);
    }
    let n = w
        .get_component::<TrailRenderer>(id)
        .unwrap()
        .position_count();
    assert!((25..=31).contains(&n), "{n} points within half a second");
    // Edit mode (no new frames): nothing changes.
    sys.always(&mut w);
    sys.always(&mut w);
    assert_eq!(
        w.get_component::<TrailRenderer>(id)
            .unwrap()
            .position_count(),
        n
    );
    // Stop emitting: the trail dies out over `time`, then the object goes.
    w.get_component_mut::<TrailRenderer>(id).unwrap().emitting = false;
    for _ in 0..40 {
        frame(&mut w, &mut sys, 1.0 / 60.0);
    }
    // (Destroy happens at the end of the frame.)
    w.flush_destroyed();
    assert!(w.get(id).is_none(), "autodestructed");
}

#[test]
fn decals_fade_with_distance_and_take_the_material() {
    let (mut w, _) = world();
    let mat = {
        let a = w.resource_mut::<AssetServer>();
        a.materials.insert(
            "test:decal",
            Material::unlit("decal", Color::new(0.0, 1.0, 0.0, 0.5)),
        )
    };
    let id = w.create_at("Decal", Vector3::new(0.0, 0.0, 10.0), Quaternion::IDENTITY);
    w.add_component(
        id,
        DecalProjector {
            material: Some(mat),
            size: Vector3::new(2.0, 3.0, 4.0),
            draw_distance: 20.0,
            fade_scale: 0.5,
            ..Default::default()
        },
    );
    let mut out = Vec::new();
    extract_decals(&w, Vector3::ZERO, &mut out);
    assert_eq!(out.len(), 1);
    let d = &out[0];
    assert_eq!(d.color[1], 1.0);
    assert!((d.color[3] - 0.5).abs() < 1e-4, "inside the fade start");
    // The box is centred on the pivot (URP default: 0.5 m forward) and sized by `size`.
    let c = d.to_world.transform_point3(Vector3::ZERO);
    assert!((c - Vector3::new(0.0, 0.0, 10.5)).length() < 1e-4, "{c}");
    let corner = d.to_world.transform_point3(Vector3::splat(0.5));
    assert!(
        (corner - Vector3::new(1.0, 1.5, 12.5)).length() < 1e-4,
        "{corner}"
    );
    out.clear();
    extract_decals(&w, Vector3::new(0.0, 0.0, -5.0), &mut out);
    assert!(
        (out[0].color[3] - 0.5 * 0.5).abs() < 1e-3,
        "halfway through the fade"
    );
    out.clear();
    extract_decals(&w, Vector3::new(0.0, 0.0, -30.0), &mut out);
    assert!(out.is_empty(), "beyond the draw distance");
}

#[test]
fn billboards_pick_the_image_for_the_view_angle() {
    let images: Vec<Vector4> = (0..4)
        .map(|i| Vector4::new(i as f32 * 0.25, 0.0, 0.25, 1.0))
        .collect();
    assert_eq!(billboard_image(&images, Vector3::Z).x, 0.0, "front");
    assert_eq!(billboard_image(&images, Vector3::X).x, 0.25, "side");
    assert_eq!(billboard_image(&images, -Vector3::Z).x, 0.5, "back");
    let (mut w, _) = world();
    w.resource_mut::<AssetServer>()
        .save_data(
            "Assets/Tree.billboard",
            Box::new(BillboardAsset {
                width: 2.0,
                height: 4.0,
                images,
                ..Default::default()
            }),
        )
        .unwrap();
    let id = w.create("Tree");
    w.add_component(
        id,
        BillboardRenderer {
            billboard: Some(oxide_assets::Handle::from_path("Assets/Tree.billboard")),
        },
    );
    let _ = w.resource_mut::<AssetServer>().data_asset(
        &oxide_assets::Handle::<BillboardAsset>::from_path("Assets/Tree.billboard"),
    );
    let mut out = Vec::new();
    extract_billboards(&w, Vector3::new(10.0, 0.0, 0.0), &mut out);
    let p = &out[0].particles[0];
    assert_eq!(p.position, [0.0, 2.0, 0.0], "standing on the ground");
    assert_eq!((p.size, p.aspect), (2.0, 2.0));
    assert_eq!(p.uv[0], 0.25);
}

#[test]
fn line_renderer_draws_on_the_gpu() {
    use oxide_render::*;
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let (mut w, mut sys) = world();
    let cam = w.create_at(
        "Main Camera",
        Vector3::new(0.0, 0.0, -5.0),
        Quaternion::IDENTITY,
    );
    w.get_mut(cam).unwrap().tag = "MainCamera".into();
    let id = w.create("Line");
    w.add_component(
        id,
        LineRenderer {
            positions: vec![Vector3::new(-0.8, 0.0, 0.0), Vector3::new(0.8, 0.0, 0.0)],
            width_multiplier: 0.4,
            color: Gradient::two(Color::rgb(1.0, 0.0, 0.0), Color::rgb(1.0, 0.0, 0.0)),
            ..Default::default()
        },
    );
    frame(&mut w, &mut sys, 1.0 / 60.0);
    let mut f = RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::new(0.0, 0.0, -5.0),
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Orthographic { size: 1.0 },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(Color::BLACK),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        environment: Environment {
            post: PostSettings {
                tonemapping: TonemapMode::None,
                ..PostSettings::neutral()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    extract_draws(&w, &mut f.draws);
    let mut r = SceneRenderer::new(&gpu);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let assets = w.resource::<AssetServer>();
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let img = capture_frame(&gpu, fmt, 64, 64, |view, _| {
        r.render(&gpu, assets, &f, view, fmt, 64, 64);
    })
    .unwrap();
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    let red = |x, y| {
        let p = img.pixel(x, y);
        p[0] > 200 && p[1] < 40
    };
    assert!(red(32, 32), "the line's middle: {:?}", img.pixel(32, 32));
    assert!(red(10, 32) && red(54, 32));
    assert!(!red(32, 10) && !red(2, 32), "only where the line is");
    // 0.4 m wide = 12.8 px.
    let tall = (0..64).filter(|&y| red(32, y)).count();
    assert!((11..=15).contains(&tall), "{tall}");
}
