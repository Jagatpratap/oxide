//! GPU test: a VFX Graph burst simulated by compute shaders and drawn from the particle
//! buffer: particles move, die at the end of their lifetime and their slots are reused.
//! Skipped without a GPU.

use oxide_math::{Color, Quaternion, Vector2, Vector3, Vector4};
use oxide_render::*;
use oxide_scene::World;
use oxide_shadergraph::GraphNode;
use oxide_vfx::*;

const SIZE: u32 = 64;

fn asset(lifetime: f32, burst: f32, rate: f32) -> VisualEffectAsset {
    let mut a = VisualEffectAsset::empty();
    let spawn = a.graph.add(GraphNode::new(0, SPAWN, Vector2::ZERO));
    let init = a
        .graph
        .add(GraphNode::new(0, INITIALIZE, Vector2::ZERO).with_option("Capacity", "600"));
    let update = a.graph.add(GraphNode::new(0, UPDATE, Vector2::ZERO));
    let out = a
        .graph
        .add(GraphNode::new(0, OUTPUT, Vector2::ZERO).with_option("Blend Mode", "Opaque"));
    a.graph.connect(spawn, "Flow Out", init, "Flow In");
    a.graph.connect(init, "Flow Out", update, "Flow In");
    a.graph.connect(update, "Flow Out", out, "Flow In");
    let b = a.add_block(spawn, "Single Burst");
    a.block_mut(b)
        .unwrap()
        .set_value("Count", Vector4::splat(burst));
    let b = a.add_block(spawn, "Constant Spawn Rate");
    a.block_mut(b)
        .unwrap()
        .set_value("Rate", Vector4::splat(rate));
    let b = a.add_block(init, "Set Position (Sphere)");
    a.block_mut(b)
        .unwrap()
        .set_value("Radius", Vector4::splat(0.3));
    let b = a.add_block(init, "Set Velocity");
    a.block_mut(b)
        .unwrap()
        .set_value("Velocity", Vector4::new(0.0, 3.0, 0.0, 0.0));
    let b = a.add_block(init, "Set Lifetime");
    a.block_mut(b)
        .unwrap()
        .set_value("Lifetime", Vector4::splat(lifetime));
    let b = a.add_block(init, "Set Size");
    a.block_mut(b)
        .unwrap()
        .set_value("Size", Vector4::splat(0.25));
    let b = a.add_block(init, "Set Color");
    a.block_mut(b)
        .unwrap()
        .set_value("Color", Vector4::new(1.0, 0.0, 0.0, 1.0));
    a
}

fn frame(w: &World, index: u64) -> RenderFrame {
    let mut f = RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::new(0.0, 1.5, -6.0),
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Perspective { fov: 40.0 },
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
        frame_index: index,
        ..Default::default()
    };
    extract(w, &mut f.vfx);
    f
}

/// Red pixels in the image.
fn red(img: &CapturedImage) -> usize {
    (0..SIZE * SIZE)
        .filter(|i| {
            let p = img.pixel(i % SIZE, i / SIZE);
            p[0] > 120 && p[1] < 40
        })
        .count()
}

#[test]
fn particles_simulate_and_die_on_the_gpu() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("oxide-vfx-gpu-{}", std::process::id()));
    let mut assets = oxide_assets::AssetServer::new(&dir);
    register_assets(&mut assets);
    assets.data.insert(
        "test:burst.vfx",
        "VisualEffectAsset",
        Box::new(asset(1.0, 500.0, 0.0)),
    );
    let mut w = World::new();
    w.resources.insert(assets);
    install(&mut w);
    let id = w.create("FX");
    w.add_component(id, VisualEffect::new("test:burst.vfx"));
    let mut r = SceneRenderer::new(&gpu);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut render = |w: &World, index: u64| {
        let f = frame(w, index);
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
            r.render(
                &gpu,
                w.resource::<oxide_assets::AssetServer>(),
                &f,
                view,
                fmt,
                SIZE,
                SIZE,
            );
        })
        .unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("GPU validation error: {e}");
        }
        img
    };
    // 0.5 s in 1/20 s steps: the burst rose 1.5 m, to the middle of the view.
    let mut img = None;
    for k in 1..=10u64 {
        update(&mut w, 0.05);
        img = Some(render(&w, k));
    }
    let img = img.unwrap();
    let centre_rows = (SIZE / 2 - 6..SIZE / 2 + 6)
        .map(|y| (0..SIZE).filter(|x| img.pixel(*x, y)[0] > 120).count())
        .sum::<usize>();
    let bottom_rows = (SIZE - 12..SIZE)
        .map(|y| (0..SIZE).filter(|x| img.pixel(*x, y)[0] > 120).count())
        .sum::<usize>();
    eprintln!(
        "after 0.5 s: {} red pixels, {centre_rows} around the centre, {bottom_rows} at the bottom",
        red(&img)
    );
    assert!(centre_rows > 40, "particles reached the middle");
    assert!(bottom_rows < centre_rows / 4, "and left the bottom");

    // Rendering the same frame again (another view) doesn't step the simulation.
    let again = render(&w, 10);
    assert_eq!(red(&again), red(&img), "same frame, same particles");

    // After their 1 s lifetime they're all gone.
    for k in 11..=24u64 {
        update(&mut w, 0.05);
        render(&w, k);
    }
    let img = render(&w, 24);
    eprintln!("after 1.2 s: {} red pixels", red(&img));
    assert_eq!(red(&img), 0, "dead particles aren't drawn");

    // Slots are reused: a new burst after Play draws again.
    effect_mut(&mut w, id).unwrap().0.play();
    let mut img = None;
    for k in 25..=34u64 {
        update(&mut w, 0.05);
        img = Some(render(&w, k));
    }
    eprintln!("second burst: {} red pixels", red(img.as_ref().unwrap()));
    assert!(red(img.as_ref().unwrap()) > 40, "the second burst draws");
}
