//! GPU tests for the post-processing stack. Skipped when no GPU is present.

use oxide_assets::Handle;
use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3};
use oxide_render::*;

const SIZE: u32 = 64;

/// A camera looking at a large flat unlit orange quad that fills the view.
fn frame(assets: &mut AssetServer, post: PostSettings) -> RenderFrame {
    let orange = assets
        .materials
        .insert(
            "test:orange",
            Material::unlit("Orange", Color::rgb(0.9, 0.5, 0.2)),
        )
        .id();
    RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::new(0.0, 0.0, -2.0),
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Perspective { fov: 60.0 },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(Color::BLACK),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        draws: vec![DrawItem {
            id: 1,
            mesh: AssetId::from_path(&PrimitiveType::Cube.mesh_path()),
            submesh: 0,
            material: orange,
            model: Matrix4x4::trs(
                Vector3::ZERO,
                Quaternion::IDENTITY,
                Vector3::new(20.0, 20.0, 0.1),
            ),
            cast_shadows: false,
            shadows_only: false,
            receive_shadows: false,
            layer: 0,
            joint_offset: None,
            occluder: false,
            occludee: false,
            reflection_probe_static: false,
            lightmap: None,
            light_probe: None,
            sorting: Default::default(),
            rendering_layers: u32::MAX,
        }],
        environment: Environment {
            post,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn render(
    gpu: &Gpu,
    r: &mut SceneRenderer,
    assets: &AssetServer,
    f: &RenderFrame,
) -> CapturedImage {
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let img = capture_frame(gpu, fmt, SIZE, SIZE, |view, _| {
        r.render(gpu, assets, f, view, fmt, SIZE, SIZE);
    })
    .expect("capture");
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    img
}

fn center(img: &CapturedImage) -> [i32; 3] {
    let p = img.pixel(SIZE / 2, SIZE / 2);
    [p[0] as i32, p[1] as i32, p[2] as i32]
}

#[test]
fn grading_tonemapping_and_vignette() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut shot = |post: PostSettings| {
        let f = frame(&mut assets, post);
        render(&gpu, &mut r, &assets, &f)
    };
    let base = PostSettings::neutral();

    // No tonemapping: the unlit color comes out as authored (sRGB 0.9, 0.5, 0.2).
    let plain = center(&shot(PostSettings {
        tonemapping: TonemapMode::None,
        ..base.clone()
    }));
    assert!(
        (plain[0] - 230).abs() <= 3 && (plain[1] - 128).abs() <= 3,
        "{plain:?}"
    );
    // ACES and Neutral both compress it differently.
    let aces = center(&shot(base.clone()));
    let neutral = center(&shot(PostSettings {
        tonemapping: TonemapMode::Neutral,
        ..base.clone()
    }));
    assert_ne!(aces, plain);
    assert_ne!(neutral, aces);

    // Saturation -100: grey.
    let grey = center(&shot(PostSettings {
        saturation: -100.0,
        ..base.clone()
    }));
    assert!((grey[0] - grey[2]).abs() <= 2, "desaturated: {grey:?}");

    // Warmer white balance: more red relative to blue.
    let warm = center(&shot(PostSettings {
        white_balance: [60.0, 0.0],
        ..base.clone()
    }));
    assert!(
        warm[0] - warm[2] > aces[0] - aces[2],
        "warm {warm:?} vs {aces:?}"
    );

    // Vignette darkens the corners, not the center.
    let img = shot(PostSettings {
        vignette: VignetteSettings {
            intensity: 0.6,
            ..base.vignette
        },
        ..base.clone()
    });
    let corner = img.pixel(1, 1);
    assert!((center(&img)[0] - aces[0]).abs() <= 3);
    assert!((corner[0] as i32) < aces[0] - 40, "corner {corner:?}");

    // Every effect at once still renders (shader paths and validation).
    let all = shot(PostSettings {
        contrast: 30.0,
        hue_shift: 40.0,
        chromatic_aberration: 0.5,
        film_grain: [0.5, 0.8],
        split_shadows: [0.2, 0.3, 0.8],
        split_highlights: [0.9, 0.6, 0.3],
        lift: [1.0, 0.9, 0.8, 0.1],
        gamma: [0.9, 1.0, 1.1, 0.0],
        gain: [1.1, 1.0, 0.9, 0.0],
        shadows: [0.8, 0.9, 1.2, 0.0],
        channel_mixer: [[80.0, 20.0, 0.0], [0.0, 100.0, 0.0], [0.0, 10.0, 90.0]],
        lens_distortion: LensDistortionSettings {
            intensity: 0.5,
            ..base.lens_distortion
        },
        ..PostSettings::default()
    });
    assert!(center(&all).iter().any(|&c| c > 20));
}

/// Sum of absolute differences between horizontal neighbours (red channel): detail.
fn detail(img: &CapturedImage) -> u64 {
    let mut sum = 0u64;
    for y in 8..SIZE - 8 {
        for x in 8..SIZE - 9 {
            sum += img.pixel(x, y)[0].abs_diff(img.pixel(x + 1, y)[0]) as u64;
        }
    }
    sum
}

/// A big unlit checkerboard `distance` metres in front of the camera.
fn checker_frame(assets: &mut AssetServer, distance: f32, post: PostSettings) -> RenderFrame {
    let mut m = Material::unlit("Checker", Color::WHITE);
    m.base_map = Some(Handle::from_path("builtin:grid"));
    m.tiling = oxide_math::Vector2::new(8.0, 8.0);
    let mat = assets.materials.insert("test:checker", m).id();
    let mut f = frame(assets, post);
    f.draws[0].material = mat;
    f.draws[0].model = Matrix4x4::trs(
        Vector3::new(0.0, 0.0, distance),
        Quaternion::IDENTITY,
        Vector3::new(distance * 2.0, distance * 2.0, 0.1),
    );
    f
}

#[test]
fn depth_of_field_and_motion_blur() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let base = PostSettings::neutral();

    let sharp = detail(&{
        let f = checker_frame(&mut assets, 20.0, base.clone());
        render(&gpu, &mut r, &assets, &f)
    });
    // Gaussian DoF: everything past 10 m is fully blurred.
    let gaussian = PostSettings {
        depth_of_field: DofSettings {
            mode: DofMode::Gaussian,
            gaussian_start: 5.0,
            gaussian_end: 10.0,
            ..Default::default()
        },
        ..base.clone()
    };
    let blurred = detail(&{
        let f = checker_frame(&mut assets, 20.0, gaussian.clone());
        render(&gpu, &mut r, &assets, &f)
    });
    assert!(
        blurred * 2 < sharp,
        "far plane should blur: {blurred} vs sharp {sharp}"
    );
    // Bokeh focused on the plane keeps it sharp.
    let focused = detail(&{
        let f = checker_frame(
            &mut assets,
            20.0,
            PostSettings {
                depth_of_field: DofSettings {
                    mode: DofMode::Bokeh,
                    focus_distance: 20.0,
                    ..Default::default()
                },
                ..base.clone()
            },
        );
        render(&gpu, &mut r, &assets, &f)
    });
    assert!(
        focused * 10 > sharp * 9,
        "in focus stays sharp: {focused} vs {sharp}"
    );

    // Motion blur: the camera turns between frames.
    let blur = PostSettings {
        motion_blur: [1.0, 0.2],
        ..base.clone()
    };
    let mut f = checker_frame(&mut assets, 20.0, blur);
    f.frame_index = 100;
    render(&gpu, &mut r, &assets, &f);
    f.frame_index = 101;
    let still = detail(&render(&gpu, &mut r, &assets, &f));
    assert!(
        still * 10 > sharp * 9,
        "no motion, no blur: {still} vs {sharp}"
    );
    f.frame_index = 102;
    f.cameras[0].rotation = Quaternion::euler(0.0, 3.0, 0.0);
    let moving = detail(&render(&gpu, &mut r, &assets, &f));
    assert!(
        moving * 2 < sharp,
        "turning camera blurs: {moving} vs {sharp}"
    );
}

#[test]
fn color_curves_lookup_panini_and_lens_dirt() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    // Lookup strips (16 slices of 16×16): identity, and one that maps everything to red.
    let n = 16u32;
    let strip = |f: &dyn Fn(f32, f32, f32) -> [u8; 3]| -> Vec<u8> {
        let mut px = Vec::with_capacity((n * n * n * 4) as usize);
        for y in 0..n {
            for x in 0..n * n {
                let (slice, rx) = (x / n, x % n);
                let r = rx as f32 / (n - 1) as f32;
                // Unity strips: green grows upward (rows are stored top first).
                let g = (n - 1 - y) as f32 / (n - 1) as f32;
                let b = slice as f32 / (n - 1) as f32;
                let c = f(r, g, b);
                px.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        px
    };
    let to8 = |v: f32| (v * 255.0).round() as u8;
    let identity = assets
        .textures
        .insert(
            "test:lut_identity.png",
            oxide_assets::Texture2D::from_rgba8(
                "lut",
                n * n,
                n,
                strip(&|r, g, b| [to8(r), to8(g), to8(b)]),
                false,
            ),
        )
        .id();
    let red = assets
        .textures
        .insert(
            "test:lut_red.png",
            oxide_assets::Texture2D::from_rgba8(
                "lut",
                n * n,
                n,
                strip(&|_, _, _| [255, 0, 0]),
                false,
            ),
        )
        .id();
    let dirt = assets
        .textures
        .insert(
            "test:dirt.png",
            oxide_assets::Texture2D::from_rgba8("dirt", 4, 4, [0, 255, 0, 255].repeat(16), true),
        )
        .id();
    let mut r = SceneRenderer::new(&gpu);
    let mut shot = |post: PostSettings| {
        let f = frame(&mut assets, post);
        render(&gpu, &mut r, &assets, &f)
    };
    let base = PostSettings {
        tonemapping: TonemapMode::None,
        ..PostSettings::neutral()
    };
    let plain = center(&shot(base.clone()));

    // Master curve inverted: 1 - x on every channel.
    let mut curves: ColorCurveSamples = [[0.5; CURVE_SAMPLES]; 8];
    for (k, c) in curves.iter_mut().enumerate().take(4) {
        for (i, v) in c.iter_mut().enumerate() {
            let x = i as f32 / (CURVE_SAMPLES - 1) as f32;
            *v = if k == 0 { 1.0 - x } else { x };
        }
    }
    let inverted = center(&shot(PostSettings {
        curves: Some(std::sync::Arc::new(curves)),
        ..base.clone()
    }));
    for k in 0..3 {
        assert!(
            (inverted[k] - (255 - plain[k])).abs() <= 6,
            "inverted {inverted:?} vs {plain:?}"
        );
    }

    // Identity lookup: unchanged; the red one turns everything red; half contribution blends.
    let same = center(&shot(PostSettings {
        lookup: Some((identity, 1.0)),
        ..base.clone()
    }));
    for k in 0..3 {
        assert!(
            (same[k] - plain[k]).abs() <= 4,
            "identity LUT {same:?} vs {plain:?}"
        );
    }
    let reddened = center(&shot(PostSettings {
        lookup: Some((red, 1.0)),
        ..base.clone()
    }));
    assert!(
        reddened[0] > 250 && reddened[1] < 5 && reddened[2] < 5,
        "{reddened:?}"
    );
    let half = center(&shot(PostSettings {
        lookup: Some((red, 0.5)),
        ..base.clone()
    }));
    assert!(
        half[1] > 20 && half[1] < plain[1],
        "half contribution {half:?}"
    );

    // Lens dirt (green) shows where the bloom is.
    let bloom = PostSettings {
        bloom: BloomSettings {
            intensity: 1.0,
            threshold: 0.1,
            ..base.bloom
        },
        ..base.clone()
    };
    let clean = center(&shot(bloom.clone()));
    let dirty = center(&shot(PostSettings {
        bloom: BloomSettings {
            dirt: Some((dirt, 3.0)),
            ..bloom.bloom
        },
        ..bloom.clone()
    }));
    assert!(
        dirty[1] > clean[1] + 10,
        "dirt adds green: {dirty:?} vs {clean:?}"
    );

    // Panini: the checkerboard near the edges is pulled toward the centre (fewer, wider
    // squares cross the screen), and crop-to-fit leaves no black border.
    let mut f = checker_frame(&mut assets, 3.0, base.clone());
    f.cameras[0].projection = ProjectionKind::Perspective { fov: 100.0 };
    // Wide enough to fill the 100° view.
    f.draws[0].model = Matrix4x4::trs(
        Vector3::new(0.0, 0.0, 3.0),
        Quaternion::IDENTITY,
        Vector3::new(30.0, 30.0, 0.1),
    );
    let flat = render(&gpu, &mut r, &assets, &f);
    let flat_edge = flat.pixel(0, SIZE / 2);
    assert!(
        flat_edge[0] > 10 || flat_edge[1] > 10,
        "the plane fills the view: {flat_edge:?}"
    );
    f.environment.post.panini = [1.0, 1.0];
    let panini = render(&gpu, &mut r, &assets, &f);
    assert_ne!(flat.rgba, panini.rgba, "panini changes the image");
    let edge = panini.pixel(0, SIZE / 2);
    assert!(
        edge[0] > 10 || edge[1] > 10,
        "no border with crop to fit: {edge:?}"
    );
}
