//! GPU test: transparent draw order follows Order in Layer, the render queue and renderer
//! Priority before distance (Unity's CommonTransparent sorting). Skipped without a GPU.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType, ShaderKind, SurfaceType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 32;

fn quad(material: AssetId, z: f32, sorting: DrawSorting) -> DrawItem {
    DrawItem {
        id: 0,
        mesh: AssetId::from_path(&PrimitiveType::Quad.mesh_path()),
        submesh: 0,
        material,
        // The quad faces -Z (towards the camera).
        model: Matrix4x4::trs(
            Vector3::new(0.0, 0.0, z),
            Quaternion::IDENTITY,
            Vector3::splat(4.0),
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
        sorting,
        rendering_layers: u32::MAX,
    }
}

#[test]
fn transparent_order_follows_sorting_then_distance() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let glass = |name: &str, c: Color, queue: i32| Material {
        name: name.into(),
        shader: ShaderKind::Unlit,
        base_color: c,
        surface_type: SurfaceType::Transparent,
        render_queue: queue,
        ..Default::default()
    };
    let red = assets
        .materials
        .insert(
            "test:red.mat",
            glass("red", Color::new(1.0, 0.0, 0.0, 0.6), -1),
        )
        .id();
    let blue = assets
        .materials
        .insert(
            "test:blue.mat",
            glass("blue", Color::new(0.0, 0.0, 1.0, 0.6), -1),
        )
        .id();
    let blue_late = assets
        .materials
        .insert(
            "test:blue_late.mat",
            glass("blue", Color::new(0.0, 0.0, 1.0, 0.6), 3100),
        )
        .id();
    let mut r = SceneRenderer::new(&gpu);
    let mut render = |draws: Vec<DrawItem>| {
        let f = RenderFrame {
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
            draws,
            environment: Environment {
                post: PostSettings {
                    tonemapping: TonemapMode::None,
                    ..PostSettings::neutral()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
        let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
            r.render(&gpu, &assets, &f, view, fmt, SIZE, SIZE);
        })
        .unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("GPU validation error: {e}");
        }
        img.pixel(SIZE / 2, SIZE / 2)
    };
    let d = DrawSorting::default();
    // Red in front of blue: back to front, red is drawn last and dominates.
    let c = render(vec![quad(red, 0.0, d), quad(blue, 1.0, d)]);
    assert!(c[0] > c[2], "red on top by distance: {c:?}");
    // Order in Layer beats distance: the far blue quad is drawn last.
    let late = DrawSorting { order: 1, ..d };
    let c = render(vec![quad(red, 0.0, d), quad(blue, 1.0, late)]);
    assert!(c[2] > c[0], "blue on top by Order in Layer: {c:?}");
    // So does a later render queue (Unity `Material.renderQueue`).
    let c = render(vec![quad(red, 0.0, d), quad(blue_late, 1.0, d)]);
    assert!(c[2] > c[0], "blue on top by render queue: {c:?}");
    // And renderer Priority.
    let prio = DrawSorting { priority: 1, ..d };
    let c = render(vec![quad(red, 0.0, d), quad(blue, 1.0, prio)]);
    assert!(c[2] > c[0], "blue on top by Priority: {c:?}");
}
