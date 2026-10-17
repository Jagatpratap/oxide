//! GPU tests for reflection probes and image-based lighting. Skipped when no GPU is present.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 128;

fn draw(mesh: PrimitiveType, material: AssetId, pos: Vector3, scale: Vector3) -> DrawItem {
    DrawItem {
        id: 0,
        mesh: AssetId::from_path(&mesh.mesh_path()),
        submesh: 0,
        material,
        model: Matrix4x4::trs(pos, Quaternion::IDENTITY, scale),
        cast_shadows: true,
        shadows_only: false,
        receive_shadows: true,
        layer: 0,
        joint_offset: None,
        occluder: false,
        occludee: false,
        reflection_probe_static: true,
        lightmap: None,
        light_probe: None,
        sorting: Default::default(),
        rendering_layers: u32::MAX,
    }
}

/// A mirror ball at (0, 1, 0) with a red block to its right (+X) and a green one to its left.
fn scene(assets: &mut AssetServer) -> RenderFrame {
    let mut mirror = Material::lit("Mirror", Color::WHITE);
    mirror.metallic = 1.0;
    mirror.smoothness = 1.0;
    let mirror = assets.materials.insert("test:mirror", mirror).id();
    let red = assets
        .materials
        .insert(
            "test:red",
            Material::unlit("Red", Color::rgb(1.0, 0.0, 0.0)),
        )
        .id();
    let green = assets
        .materials
        .insert(
            "test:green",
            Material::unlit("Green", Color::rgb(0.0, 1.0, 0.0)),
        )
        .id();
    let wall = Vector3::new(0.5, 6.0, 12.0);
    RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::new(0.0, 1.0, -4.0),
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Perspective { fov: 40.0 },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Sky,
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        draws: vec![
            draw(
                PrimitiveType::Sphere,
                mirror,
                Vector3::new(0.0, 1.0, 0.0),
                Vector3::ONE,
            ),
            draw(PrimitiveType::Cube, red, Vector3::new(3.0, 1.0, 0.0), wall),
            draw(
                PrimitiveType::Cube,
                green,
                Vector3::new(-3.0, 1.0, 0.0),
                wall,
            ),
        ],
        environment: Environment {
            post: PostSettings::neutral(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn probe() -> ReflectionProbeView {
    ReflectionProbeView {
        id: 1,
        position: Vector3::new(0.0, 1.0, 0.0),
        box_min: Vector3::splat(-10.0),
        box_max: Vector3::splat(10.0),
        box_projection: false,
        blend_distance: 0.0,
        importance: 1,
        intensity: 1.0,
        near: 0.3,
        far: 100.0,
        culling_mask: u32::MAX,
        clear: ClearMode::Sky,
        baked: false,
        refresh: ProbeRefresh::OnAwake,
        render_requests: 0,
        source: None,
    }
}

fn render(
    gpu: &Gpu,
    r: &mut SceneRenderer,
    assets: &AssetServer,
    frame: &RenderFrame,
) -> (CapturedImage, RenderStats) {
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut stats = RenderStats::default();
    let img = capture_frame(
        gpu,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        SIZE,
        SIZE,
        |view, _| {
            stats = r.render(
                gpu,
                assets,
                frame,
                view,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                SIZE,
                SIZE,
            );
        },
    )
    .expect("capture");
    // Set OXIDE_TEST_IMAGES=<dir> to save every rendered image for inspection.
    if let Ok(dir) = std::env::var("OXIDE_TEST_IMAGES") {
        static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let i = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::fs::write(format!("{dir}/reflections_{i}.png"), img.to_png().unwrap()).unwrap();
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    (img, stats)
}

/// Pixel on the ball, `dx` pixels right of its center.
fn ball(img: &CapturedImage, dx: i32) -> [u8; 4] {
    img.pixel((SIZE as i32 / 2 + dx) as u32, SIZE / 2)
}

#[test]
fn mirror_ball_reflects_scene_through_probe() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut frame = scene(&mut assets);
    let mut r = SceneRenderer::new(&gpu);

    // Sky only: the ball reflects the sky and the blocks don't show in it.
    let (img, stats) = render(&gpu, &mut r, &assets, &frame);
    assert_eq!(stats.probe_captures, 1, "sky captured once");
    let (left, right) = (ball(&img, -14), ball(&img, 14));
    assert!(
        (left[0] as i32 - left[1] as i32).abs() < 40
            && (right[0] as i32 - right[1] as i32).abs() < 40,
        "sky-only ball should be neutral: left {left:?} right {right:?}"
    );

    // With a probe, the right side mirrors the red block and the left the green one.
    frame.reflection_probes.push(probe());
    let (img, stats) = render(&gpu, &mut r, &assets, &frame);
    assert_eq!(stats.probe_captures, 1, "probe captured, sky unchanged");
    assert_eq!(stats.reflection_probes, 1);
    let (left, right) = (ball(&img, -14), ball(&img, 14));
    assert!(
        right[0] as i32 > right[1] as i32 + 60,
        "right side should reflect red: {right:?}"
    );
    assert!(
        left[1] as i32 > left[0] as i32 + 60,
        "left side should reflect green: {left:?}"
    );

    // OnAwake: nothing is recaptured while nothing changes.
    let (_, stats) = render(&gpu, &mut r, &assets, &frame);
    assert_eq!(stats.probe_captures, 0);

    // Moving the probe recaptures it.
    frame.reflection_probes[0].position.y += 0.1;
    let (_, stats) = render(&gpu, &mut r, &assets, &frame);
    assert_eq!(stats.probe_captures, 1);

    // Outside the probe box the ball falls back to the sky.
    frame.reflection_probes[0].box_min = Vector3::splat(5.0);
    frame.reflection_probes[0].box_max = Vector3::splat(6.0);
    let (img, _) = render(&gpu, &mut r, &assets, &frame);
    let right = ball(&img, 14);
    assert!(
        (right[0] as i32 - right[1] as i32).abs() < 40,
        "outside the box: sky reflection, got {right:?}"
    );
}

#[test]
fn ambient_follows_the_sky() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    // A rough white ball lit only by the environment.
    let mut frame = scene(&mut assets);
    frame.draws.truncate(1);
    frame.draws[0].material = AssetId::from_path(oxide_assets::DEFAULT_MATERIAL);
    let mut r = SceneRenderer::new(&gpu);
    let (img, _) = render(&gpu, &mut r, &assets, &frame);
    let neutral = ball(&img, 0);

    // A red-tinted sky makes the ambient light red too.
    frame.environment.sky_tint = Color::rgb(0.9, 0.2, 0.2);
    frame.environment.ground_color = Color::rgb(0.9, 0.2, 0.2);
    let (img, stats) = render(&gpu, &mut r, &assets, &frame);
    assert_eq!(stats.probe_captures, 1, "sky recaptured after it changed");
    let red = ball(&img, 0);
    assert!(
        red[0] as i32 - red[2] as i32 > neutral[0] as i32 - neutral[2] as i32 + 30,
        "neutral {neutral:?} vs red sky {red:?}"
    );
}

/// The lightmapper's RGBM encoding (gamma, range 8).
fn rgbm(c: Vector3) -> [u8; 4] {
    let s = c.max(Vector3::ZERO).powf(0.5);
    let m = ((s.max_element() / 8.0).clamp(1.0 / 255.0, 1.0) * 255.0).ceil() / 255.0;
    let rgb = (s / (m * 8.0)).clamp(Vector3::ZERO, Vector3::ONE);
    [
        (rgb.x * 255.0).round() as u8,
        (rgb.y * 255.0).round() as u8,
        (rgb.z * 255.0).round() as u8,
        (m * 255.0) as u8,
    ]
}

#[test]
fn custom_cubemaps_and_saved_baked_captures_feed_probes() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut frame = scene(&mut assets);
    // The blocks only exist in the bake: drawn now, removed after.
    let mut r = SceneRenderer::new(&gpu);
    let faces = r
        .bake_reflection_probe(&gpu, &assets, &frame, &probe())
        .unwrap();
    assert_eq!(faces.len(), 6);
    // +X sees the red block, -X the green one (linear HDR values).
    let mid = |f: &Vec<[f32; 4]>| f[f.len() / 2 + 64];
    let (px, nx) = (mid(&faces[0]), mid(&faces[1]));
    assert!(px[0] > 0.5 && px[1] < 0.1, "+X face {px:?}");
    assert!(nx[1] > 0.5 && nx[0] < 0.1, "-X face {nx:?}");

    // Saved like Generate Lighting does (RGBM), the capture drives the probe without the
    // blocks being in the scene any more.
    let size = (faces[0].len() as f32).sqrt() as u32;
    let rgbm_faces = faces
        .iter()
        .map(|f| {
            let px: Vec<u8> = f
                .iter()
                .flat_map(|c| rgbm(Vector3::new(c[0], c[1], c[2])))
                .collect();
            oxide_assets::Texture2D::from_rgba8("baked", size, size, px, false)
        })
        .collect();
    let baked = assets
        .cubemaps
        .insert(
            "test:baked.cubemap",
            oxide_assets::TextureCube {
                name: "baked".into(),
                faces: rgbm_faces,
            },
        )
        .id();
    frame.draws.truncate(1);
    let mut p = probe();
    p.baked = true;
    p.source = Some((baked, assets.cubemaps.version(baked).unwrap(), true));
    frame.reflection_probes = vec![p];
    let (img, _) = render(&gpu, &mut r, &assets, &frame);
    let (left, right) = (ball(&img, -14), ball(&img, 14));
    assert!(
        right[0] as i32 > right[1] as i32 + 60,
        "baked red: {right:?}"
    );
    assert!(
        left[1] as i32 > left[0] as i32 + 60,
        "baked green: {left:?}"
    );

    // Custom: a plain (sRGB) blue cubemap.
    let blue = assets.cubemaps.insert(
        "test:blue.cubemap",
        oxide_assets::TextureCube::solid("blue", [0, 0, 255, 255], true),
    );
    let mut p = probe();
    p.id = 2;
    p.source = Some((
        blue.id(),
        assets.cubemaps.version(blue.id()).unwrap(),
        false,
    ));
    frame.reflection_probes = vec![p];
    let (img, _) = render(&gpu, &mut r, &assets, &frame);
    let c = ball(&img, 0);
    assert!(
        c[2] as i32 > c[0] as i32 + 60 && c[2] as i32 > c[1] as i32 + 60,
        "custom blue: {c:?}"
    );
}
