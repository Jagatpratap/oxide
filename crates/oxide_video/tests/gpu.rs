//! Video frames written into a Render Texture show on a material (GPU). Skipped without a
//! GPU.

use oxide_assets::{AssetId, AssetServer, Handle, Material, PrimitiveType, RenderTexture};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3};
use oxide_render::*;
use std::sync::Arc;

#[test]
fn texture_writes_reach_materials() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(".");
    let rt = assets.render_textures.insert(
        "runtime:video/test",
        RenderTexture {
            name: "video".into(),
            width: 4,
            height: 4,
        },
    );
    let mut m = Material::unlit("screen", Color::WHITE);
    m.base_map = Some(Handle::from_path("runtime:video/test"));
    assets.materials.insert("test:screen", m);
    let eye = Vector3::new(0.0, 5.0, 0.0);
    let frame = |color: [u8; 4], serial: u64| RenderFrame {
        cameras: vec![CameraView {
            position: eye,
            rotation: Quaternion::look_rotation(Vector3::NEG_Y, Vector3::Z),
            projection: ProjectionKind::Orthographic { size: 1.0 },
            near: 0.1,
            far: 20.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(Color::BLACK),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        draws: vec![DrawItem {
            id: 1,
            mesh: AssetId::from_path(&PrimitiveType::Plane.mesh_path()),
            submesh: 0,
            material: AssetId::from_path("test:screen"),
            model: Matrix4x4::trs(Vector3::ZERO, Quaternion::IDENTITY, Vector3::ONE),
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
        texture_writes: vec![TextureWrite {
            target: rt.id(),
            width: 4,
            height: 4,
            rgba: Arc::new(color.iter().copied().cycle().take(64).collect()),
            serial,
        }],
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
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut shot = |f: &RenderFrame| {
        capture_frame(&gpu, fmt, 32, 32, |view, _| {
            r.render(&gpu, &assets, f, view, fmt, 32, 32);
        })
        .unwrap()
        .pixel(16, 16)
    };
    let red = shot(&frame([255, 0, 0, 255], 1));
    assert!(red[0] > 200 && red[1] < 40, "{red:?}");
    // A new frame (new serial) replaces it in place.
    let green = shot(&frame([0, 255, 0, 255], 2));
    assert!(green[1] > 200 && green[0] < 40, "{green:?}");
    // The same serial isn't written again.
    let still = shot(&frame([0, 0, 255, 255], 2));
    assert!(still[1] > 200, "{still:?}");
}
