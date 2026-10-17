//! Frame Debugger: the draw events of a frame and stopping after N of them. Skipped without
//! a GPU.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_render::*;

fn draw(material: &str, x: f32) -> DrawItem {
    DrawItem {
        id: 1,
        mesh: AssetId::from_path(&PrimitiveType::Cube.mesh_path()),
        submesh: 0,
        material: AssetId::from_path(material),
        model: Matrix4x4::trs(
            Vector3::new(x, 0.0, 0.0),
            Quaternion::IDENTITY,
            Vector3::splat(0.8),
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
    }
}

#[test]
fn events_list_draws_and_the_limit_stops_them() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(".");
    assets.materials.insert(
        "test:red",
        Material::unlit("Red", Color::rgb(1.0, 0.0, 0.0)),
    );
    assets.materials.insert(
        "test:blue",
        Material::unlit("Blue", Color::rgb(0.0, 0.0, 1.0)),
    );
    let frame = RenderFrame {
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
        draws: vec![draw("test:red", -0.5), draw("test:blue", 0.5)],
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
    r.frame_debugger = true;
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let shot = |r: &mut SceneRenderer| {
        capture_frame(&gpu, fmt, 64, 64, |view, _| {
            r.render(&gpu, &assets, &frame, view, fmt, 64, 64);
        })
        .unwrap()
    };
    let full = shot(&mut r);
    let events: Vec<FrameDebugEvent> = r.frame_events().to_vec();
    let draws: Vec<&FrameDebugEvent> = events.iter().filter(|e| e.limitable).collect();
    assert_eq!(draws.len(), 2, "{events:#?}");
    assert!(draws.iter().any(|e| e.name.contains("[Red]")));
    assert!(
        draws[0]
            .details
            .iter()
            .any(|(k, v)| k == "Queue" && v == "Opaque")
    );
    assert_eq!(draws[0].triangles, 12);
    let red = |img: &CapturedImage| (0..64).any(|x| img.pixel(x, 32)[0] > 200);
    let blue = |img: &CapturedImage| (0..64).any(|x| img.pixel(x, 32)[2] > 200);
    assert!(red(&full) && blue(&full));
    // Stop after the first draw: only one cube.
    r.frame_debug_limit = Some(1);
    let one = shot(&mut r);
    assert!(red(&one) != blue(&one), "exactly one cube drawn");
    r.frame_debug_limit = Some(0);
    let none = shot(&mut r);
    assert!(!red(&none) && !blue(&none));
}
