//! GPU test for the sprite pass: sprites draw through an orthographic camera, ordered by
//! sorting layer and order in layer. Skipped without a GPU.

use oxide_assets::{AssetId, AssetServer, Texture2D};
use oxide_math::{Color, Matrix4x4, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 64;

fn sprite(tex: AssetId, half: f32, layer: i32, order: i32, color: [f32; 4]) -> SpriteDraw {
    // A square of side 2·half centred on the origin.
    let model = Matrix4x4::from_translation(Vector3::new(-half, -half, 0.0))
        * Matrix4x4::from_scale(Vector3::new(2.0 * half, 2.0 * half, 1.0));
    SpriteDraw {
        texture: Some(tex),
        sorting_layer: layer,
        order,
        instance: SpriteInstance {
            model: model.to_cols_array_2d(),
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            color,
        },
    }
}

fn frame(sprites: Vec<SpriteDraw>) -> RenderFrame {
    RenderFrame {
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
        sprites,
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

#[test]
fn sprites_sort_by_layer_and_order() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(".");
    let white = assets
        .textures
        .insert("test:white", Texture2D::solid("white", [255; 4], true))
        .id();
    let mut r = SceneRenderer::new(&gpu);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let render = |r: &mut SceneRenderer, f: &RenderFrame| {
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
            r.render(&gpu, &assets, f, view, fmt, SIZE, SIZE);
        })
        .unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("GPU validation error: {e}");
        }
        img
    };
    let blue = [0.0, 0.0, 1.0, 1.0];
    let red = [1.0, 0.0, 0.0, 1.0];
    // Big blue square (order 0) under a small red one (order 1), listed red first.
    let img = render(
        &mut r,
        &frame(vec![
            sprite(white, 0.25, 0, 1, red),
            sprite(white, 0.75, 0, 0, blue),
        ]),
    );
    let c = img.pixel(SIZE / 2, SIZE / 2);
    let mid = img.pixel(SIZE / 2 + 18, SIZE / 2);
    let corner = img.pixel(1, 1);
    eprintln!("centre {c:?} ring {mid:?} corner {corner:?}");
    assert!(c[0] > 200 && c[2] < 40, "red on top in the centre: {c:?}");
    assert!(mid[2] > 200 && mid[0] < 40, "blue around it: {mid:?}");
    assert!(
        corner[0] < 20 && corner[2] < 20,
        "background outside: {corner:?}"
    );
    // A higher sorting layer beats order in layer: blue on layer 1 now covers red.
    let img = render(
        &mut r,
        &frame(vec![
            sprite(white, 0.25, 0, 99, red),
            sprite(white, 0.75, 1, 0, blue),
        ]),
    );
    let c = img.pixel(SIZE / 2, SIZE / 2);
    assert!(c[2] > 200 && c[0] < 40, "blue (higher layer) wins: {c:?}");
}
