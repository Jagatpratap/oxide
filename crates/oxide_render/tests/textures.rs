//! GPU test: imported textures are block-compressed (BC7 / BC5) and render like the
//! uncompressed originals. Skipped when no GPU is present.

use oxide_assets::{
    AssetId, AssetServer, CompressedFormat, Handle, Material, PrimitiveType, TextureCompression,
};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 128;

fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

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

#[test]
fn compressed_textures_render_like_the_originals() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("oxide-tex-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Assets")).unwrap();
    // A smooth gradient with some hard-edged stripes.
    let (w, h) = (256u32, 256u32);
    let rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            let stripe = if (x / 16 + y / 16) % 2 == 0 { 40 } else { 0 };
            [x as u8, y as u8, (128 + stripe) as u8, 255]
        })
        .collect();
    write_png(&dir.join("Assets/pattern.png"), w, h, &rgba);

    let mut assets = AssetServer::new(&dir);
    let tex = assets.load_texture("Assets/pattern.png").unwrap();
    let c = assets
        .textures
        .get(&tex)
        .unwrap()
        .compressed
        .clone()
        .expect("compressed on import");
    assert_eq!(c.format, CompressedFormat::Bc7);
    assert_eq!(
        c.mips[0].len(),
        (w * h) as usize,
        "BC7 is one byte per pixel"
    );
    let cache = std::fs::read_dir(dir.join("Library/TextureCache"))
        .unwrap()
        .count();
    assert_eq!(cache, 1, "compressed data cached");

    let mut mat = Material::unlit("Pattern", Color::WHITE);
    mat.base_map = Some(Handle::from_path("Assets/pattern.png"));
    let mat = assets.materials.insert("test:pattern", mat).id();
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let shot = |assets: &AssetServer| {
        let mut r = SceneRenderer::new(&gpu);
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
            r.render(&gpu, assets, &frame(mat), view, fmt, SIZE, SIZE);
        })
        .unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("GPU validation error: {e}");
        }
        img
    };
    let compressed = shot(&assets);

    // The same BC7 data as a Zstandard-supercompressed KTX2 file renders identically.
    let bc7 = assets.textures.get(&tex).unwrap().clone();
    std::fs::write(
        dir.join("Assets/pattern.ktx2"),
        oxide_assets::ktx2_import::encode(&bc7, true),
    )
    .unwrap();
    assets.load_texture("Assets/pattern.ktx2").unwrap();
    let mut ktx_mat = Material::unlit("Pattern KTX2", Color::WHITE);
    ktx_mat.base_map = Some(Handle::from_path("Assets/pattern.ktx2"));
    assets.materials.insert("test:pattern", ktx_mat);
    let from_ktx2 = shot(&assets);
    assert_eq!(
        from_ktx2.rgba, compressed.rgba,
        "KTX2 matches the BC7 import"
    );
    let mut png_mat = Material::unlit("Pattern", Color::WHITE);
    png_mat.base_map = Some(Handle::from_path("Assets/pattern.png"));
    assets.materials.insert("test:pattern", png_mat);
    // Re-import uncompressed through the .meta file.
    let mut imp = assets.texture_importer("Assets/pattern.png");
    imp.compression = TextureCompression::None;
    assets
        .set_texture_importer("Assets/pattern.png", &imp)
        .unwrap();
    assert!(assets.textures.get(&tex).unwrap().compressed.is_none());
    let original = shot(&assets);
    let n = compressed.rgba.len();
    let diff: u64 = (0..n)
        .filter(|i| i % 4 != 3)
        .map(|i| compressed.rgba[i].abs_diff(original.rgba[i]) as u64)
        .sum();
    let mean = diff as f64 / (n as f64 * 0.75);
    assert!(mean < 3.0, "mean difference {mean:.2} levels");
    assert!(
        compressed.pixel(SIZE / 2, SIZE / 2)[0] > 50,
        "texture visible"
    );

    // A normal map goes BC5 (two channels) and still lights correctly.
    write_png(
        &dir.join("Assets/bumps_normal.png"),
        64,
        64,
        &[128, 128, 255, 255].repeat(64 * 64),
    );
    let n = assets.load_texture("Assets/bumps_normal.png").unwrap();
    let c = assets.textures.get(&n).unwrap().compressed.clone().unwrap();
    assert_eq!(c.format, CompressedFormat::Bc5);
    let _ = std::fs::remove_dir_all(&dir);
}
