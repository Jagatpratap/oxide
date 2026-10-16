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
