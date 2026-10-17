//! GPU tests for Quality settings: Render Scale (scaled render stretched to the target) and
//! Texture Quality (largest mips skipped at upload). Skipped when no GPU is present.

use oxide_assets::{AssetId, AssetServer, FilterMode, Handle, Material, PrimitiveType, Texture2D};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 128;

/// A quad filling an orthographic view, textured with `material`.
fn frame(material: AssetId) -> RenderFrame {
    RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::new(0.0, 0.0, -1.0),
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Orthographic { size: 0.5 },
            near: 0.1,
            far: 10.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(Color::BLACK),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        draws: vec![DrawItem {
            id: 0,
            mesh: AssetId::from_path(&PrimitiveType::Quad.mesh_path()),
            submesh: 0,
            material,
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

fn shot(gpu: &Gpu, r: &mut SceneRenderer, assets: &AssetServer, f: &RenderFrame) -> CapturedImage {
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let img = capture_frame(gpu, fmt, SIZE, SIZE, |view, _| {
        r.render(gpu, assets, f, view, fmt, SIZE, SIZE);
    })
    .unwrap();
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    img
}

fn textured(assets: &mut AssetServer, name: &str, tex: Texture2D) -> AssetId {
    let path = format!("test:{name}");
    assets.textures.insert(path.as_str(), tex);
    let mut mat = Material::unlit(name, Color::WHITE);
    mat.base_map = Some(Handle::from_path(path.as_str()));
    assets
        .materials
        .insert(format!("test:{name}.mat").as_str(), mat)
        .id()
}

fn px(img: &CapturedImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    img.rgba[i..i + 4].try_into().unwrap()
}

#[test]
fn render_scale_renders_smaller_and_stretches() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    // A one-pixel checker as large as the target, point filtered.
    let rgba: Vec<u8> = (0..SIZE * SIZE)
        .flat_map(|i| {
            let v = if (i % SIZE + i / SIZE).is_multiple_of(2) {
                255
            } else {
                0
            };
            [v, v, v, 255]
        })
        .collect();
    let mut tex = Texture2D::from_rgba8("checker", SIZE, SIZE, rgba, true);
    tex.filter_mode = FilterMode::Point;
    let mat = textured(&mut assets, "checker", tex);
    let mut r = SceneRenderer::new(&gpu);

    // Full size: neighbours alternate.
    let full = shot(&gpu, &mut r, &assets, &frame(mat));
    assert_ne!(px(&full, 40, 40)[0] > 128, px(&full, 41, 40)[0] > 128);

    // A quarter size with nearest upscaling: every 4×4 block is one rendered pixel.
    let mut f = frame(mat);
    f.render_scale = Some(RenderScale {
        scale: 0.25,
        point: true,
    });
    let quarter = shot(&gpu, &mut r, &assets, &f);
    let mut mismatched = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            if px(&quarter, x, y) != px(&quarter, x / 4 * 4, y / 4 * 4) {
                mismatched += 1;
            }
        }
    }
    assert!(
        mismatched < 64,
        "{mismatched} pixels differ from their 4×4 block"
    );
    // The whole target is still covered (no black border).
    assert_eq!(px(&quarter, SIZE - 1, SIZE - 1)[3], 255);

    // Scale 1 is the same as no scaling.
    f.render_scale = Some(RenderScale {
        scale: 1.0,
        point: false,
    });
    assert_eq!(shot(&gpu, &mut r, &assets, &f).rgba, full.rgba);
}

#[test]
fn texture_quality_skips_top_mips() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    // Mip 0 green, every smaller mip red: magnified, the quad shows mip 0.
    let mut tex = Texture2D::from_rgba8("mips", 16, 16, vec![0; 16 * 16 * 4], true);
    for (i, m) in tex.mips.iter_mut().enumerate() {
        let c = if i == 0 {
            [0, 255, 0, 255]
        } else {
            [255, 0, 0, 255]
        };
        for p in m.as_chunks_mut::<4>().0 {
            p.copy_from_slice(&c);
        }
    }
    tex.filter_mode = FilterMode::Bilinear;
    let mat = textured(&mut assets, "mips", tex);
    let mut r = SceneRenderer::new(&gpu);
    let mut f = frame(mat);
    let full = px(&shot(&gpu, &mut r, &assets, &f), 64, 64);
    assert!(full[1] > 200 && full[0] < 50, "{full:?}");
    // Half Res: mip 1 becomes the top level.
    f.texture_mip_limit = 1;
    let half = px(&shot(&gpu, &mut r, &assets, &f), 64, 64);
    assert!(half[0] > 200 && half[1] < 50, "{half:?}");
    // And back.
    f.texture_mip_limit = 0;
    let back = px(&shot(&gpu, &mut r, &assets, &f), 64, 64);
    assert_eq!(back, full);
}
