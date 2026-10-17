//! GPU test: a camera renders into a Render Texture that a material then shows.

use oxide_assets::{AssetId, AssetServer, Handle, Material, PrimitiveType, RenderTexture};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 64;

fn camera(position: Vector3, culling_mask: u32, clear: Color) -> CameraView {
    CameraView {
        position,
        rotation: Quaternion::IDENTITY,
        projection: ProjectionKind::Perspective { fov: 40.0 },
        near: 0.1,
        far: 100.0,
        viewport: [0.0, 0.0, 1.0, 1.0],
        clear: ClearMode::Color(clear),
        culling_mask,
        occlusion_culling: false,
        antialiasing: Default::default(),
    }
}

fn draw(
    mesh: PrimitiveType,
    material: AssetId,
    pos: Vector3,
    scale: Vector3,
    layer: u8,
) -> DrawItem {
    DrawItem {
        id: 0,
        mesh: AssetId::from_path(&mesh.mesh_path()),
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

#[test]
fn camera_renders_into_texture_shown_by_a_material() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let rt = assets.render_textures.insert(
        "test:monitor.renderTexture",
        RenderTexture {
            name: "Monitor".into(),
            width: 128,
            height: 128,
        },
    );
    let red = assets
        .materials
        .insert(
            "test:red",
            Material::unlit("Red", Color::rgb(1.0, 0.0, 0.0)),
        )
        .id();
    let mut screen = Material::unlit("Screen", Color::WHITE);
    screen.base_map = Some(Handle::from_path("test:monitor.renderTexture"));
    let screen = assets.materials.insert("test:screen", screen).id();

    // Layer 1 holds a red cube only the texture camera sees; the main camera looks at a
    // screen showing that camera's picture.
    let hidden = 1u8;
    let frame = RenderFrame {
        cameras: vec![camera(
            Vector3::new(0.0, 0.0, -3.0),
            !(1 << hidden),
            Color::BLACK,
        )],
        texture_cameras: vec![(
            rt.id(),
            camera(
                Vector3::new(10.0, 0.0, -3.0),
                u32::MAX,
                Color::rgb(0.0, 0.0, 1.0),
            ),
        )],
        draws: vec![
            draw(
                PrimitiveType::Cube,
                red,
                Vector3::new(10.0, 0.0, 0.0),
                Vector3::ONE,
                hidden,
            ),
            draw(
                PrimitiveType::Quad,
                screen,
                Vector3::ZERO,
                Vector3::splat(1.6),
                0,
            ),
        ],
        environment: Environment {
            post: PostSettings {
                tonemapping: TonemapMode::None,
                ..PostSettings::neutral()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let mut r = SceneRenderer::new(&gpu);
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
        r.render(&gpu, &assets, &frame, view, fmt, SIZE, SIZE);
    })
    .expect("capture");
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    // Center of the screen quad: the red cube; its edge: the texture camera's blue clear.
    let center = img.pixel(SIZE / 2, SIZE / 2);
    assert!(
        center[0] > 200 && center[2] < 40,
        "cube on the screen: {center:?}"
    );
    let edge = img.pixel(SIZE / 2 + 15, SIZE / 2);
    assert!(
        edge[2] > 200 && edge[0] < 40,
        "texture camera background: {edge:?}"
    );
    // Outside the quad: the main camera's black clear, and the cube itself is not drawn.
    let outside = img.pixel(2, 2);
    assert!(outside[0] < 10 && outside[2] < 10, "{outside:?}");
}
