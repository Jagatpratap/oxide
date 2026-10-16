//! GPU tests for the post-processing stack. Skipped when no GPU is present.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
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
