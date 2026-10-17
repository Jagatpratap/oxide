//! GPU tests for renderer features (Full Screen Pass, Render Objects). Skipped without a GPU.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType, Shader};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 64;

fn draw(material: AssetId, pos: Vector3, scale: Vector3, layer: u8) -> DrawItem {
    DrawItem {
        id: 0,
        mesh: AssetId::from_path(&PrimitiveType::Cube.mesh_path()),
        submesh: 0,
        material,
        model: Matrix4x4::trs(pos, Quaternion::IDENTITY, scale),
        cast_shadows: false,
        shadows_only: false,
        receive_shadows: false,
        layer,
        joint_offset: None,
        occluder: false,
        occludee: false,
        reflection_probe_static: false,
        lightmap: None,
        light_probe: None,
        sorting: Default::default(),
    }
}

/// A small grey wall in front of a bigger red cube (layer 3), seen head-on.
fn scene(assets: &mut AssetServer) -> RenderFrame {
    let mut unlit = |name: &str, c: Color| {
        assets
            .materials
            .insert(format!("test:{name}"), Material::unlit(name, c))
            .id()
    };
    let (grey, red) = (
        unlit("grey", Color::rgb(0.5, 0.5, 0.5)),
        unlit("red", Color::rgb(1.0, 0.0, 0.0)),
    );
    RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::new(0.0, 0.0, -5.0),
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
        draws: vec![
            // About ±7 px wide on screen, in front of a ±15 px cube.
            draw(
                grey,
                Vector3::new(0.0, 0.0, -1.0),
                Vector3::new(0.6, 0.6, 0.2),
                0,
            ),
            draw(red, Vector3::new(0.0, 0.0, 1.0), Vector3::splat(2.0), 3),
        ],
        environment: Environment {
            post: PostSettings {
                tonemapping: TonemapMode::None,
                ..PostSettings::neutral()
            },
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
    .unwrap();
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    img
}

#[test]
fn full_screen_pass_runs_user_wgsl() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = scene(&mut assets);
    let plain = render(&gpu, &mut r, &assets, &f);

    let invert = assets.shaders.insert(
        "test:invert.wgsl",
        Shader {
            surface: None,
            name: "invert".into(),
            source: "fn fs_main(in: FullscreenIn) -> vec4f {\n    \
                     let c = sample_source(in.uv);\n    \
                     return vec4f(vec3f(1.0) - c.rgb * fx.params0.x, 1.0);\n}"
                .into(),
        },
    );
    f.renderer_features.push(RendererFeature::FullScreen {
        event: RenderPassEvent::AfterRenderingPostProcessing,
        shader: invert.id(),
        params: [[1.0, 0.0, 0.0, 0.0], [0.0; 4]],
    });
    let inverted = render(&gpu, &mut r, &assets, &f);
    // The inversion happens in linear space; compare after decoding.
    let lin = |v: u8| {
        let c = v as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    for (x, y) in [(SIZE / 2, SIZE / 2), (2, 2)] {
        let (a, b) = (plain.pixel(x, y), inverted.pixel(x, y));
        for c in 0..3 {
            assert!(
                (lin(a[c]) + lin(b[c]) - 1.0).abs() < 0.03,
                "({x},{y}) {a:?} vs {b:?}"
            );
        }
    }

    // A broken shader is reported and the image passes through unchanged.
    let broken = assets.shaders.insert(
        "test:broken.wgsl",
        Shader {
            surface: None,
            name: "broken".into(),
            source: "fn fs_main(in: FullscreenIn) -> vec4f { return nope; }".into(),
        },
    );
    f.renderer_features = vec![RendererFeature::FullScreen {
        event: RenderPassEvent::BeforeRenderingPostProcessing,
        shader: broken.id(),
        params: [[0.0; 4]; 2],
    }];
    let through = render(&gpu, &mut r, &assets, &f);
    assert_eq!(through.rgba, plain.rgba);
}

#[test]
fn render_objects_draws_hidden_layer_as_silhouette() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = scene(&mut assets);
    let yellow = assets
        .materials
        .insert(
            "test:xray",
            Material::unlit("X-Ray", Color::rgb(1.0, 1.0, 0.0)),
        )
        .id();
    let center = (SIZE / 2, SIZE / 2);
    let wall = render(&gpu, &mut r, &assets, &f).pixel(center.0, center.1);
    assert!(
        wall[0] > 100 && wall[0] < 200 && wall[2] > 100,
        "grey wall hides the cube: {wall:?}"
    );

    f.renderer_features.push(RendererFeature::RenderObjects {
        event: RenderPassEvent::AfterRenderingOpaques,
        layer_mask: 1 << 3,
        override_material: Some(yellow),
        depth_test: DepthTest::Greater,
        write_depth: false,
    });
    let img = render(&gpu, &mut r, &assets, &f);
    let xray = img.pixel(center.0, center.1);
    assert!(
        xray[0] > 200 && xray[1] > 200 && xray[2] < 50,
        "silhouette through the wall: {xray:?}"
    );
    // Outside the wall the cube is visible normally (red), not yellow.
    let side = img.pixel(center.0 + 11, center.1);
    assert!(
        side[0] > 200 && side[1] < 50,
        "uncovered part stays red: {side:?}"
    );
}
