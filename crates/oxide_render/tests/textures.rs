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

/// Decodes compressed blocks on the GPU (`textureLoad` into an RGBA8 target).
fn gpu_decode(
    gpu: &Gpu,
    format: wgpu::TextureFormat,
    w: u32,
    h: u32,
    data: &[u8],
) -> CapturedImage {
    let device = &gpu.device;
    let tex = wgpu::util::DeviceExt::create_texture_with_data(
        device,
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        data,
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(
            "@group(0) @binding(0) var t: texture_2d<f32>;
            @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
                let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
                return vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
            }
            @fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
                return textureLoad(t, vec2i(p.xy), 0);
            }"
            .into(),
        ),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&tex.create_view(&Default::default())),
        }],
    });
    capture_frame(gpu, wgpu::TextureFormat::Rgba8Unorm, w, h, |view, _| {
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Default::default(),
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([enc.finish()]);
    })
    .unwrap()
}

#[test]
fn mobile_formats_decode_on_the_gpu() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    use oxide_assets::texture_compress::compress_level;
    use wgpu::TextureFormat as T;
    // 48×48 divides into 4, 6 and 8 pixel blocks. Gradients, stripes and an alpha ramp.
    let (w, h) = (48u32, 48u32);
    let rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            let stripe = if (x / 12) % 2 == 0 { 60 } else { 0 };
            [
                (x * 5) as u8,
                (y * 5) as u8,
                (100 + stripe) as u8,
                (255 - x * 3) as u8,
            ]
        })
        .collect();
    let features = gpu.device.features();
    let astc = |block| T::Astc {
        block,
        channel: wgpu::AstcChannel::Unorm,
    };
    let cases = [
        (
            CompressedFormat::Astc4x4,
            astc(wgpu::AstcBlock::B4x4),
            4,
            6.0,
        ),
        (
            CompressedFormat::Astc6x6,
            astc(wgpu::AstcBlock::B6x6),
            4,
            12.0,
        ),
        (
            CompressedFormat::Astc8x8,
            astc(wgpu::AstcBlock::B8x8),
            4,
            16.0,
        ),
        (CompressedFormat::Etc2Rgb8, T::Etc2Rgb8Unorm, 3, 6.0),
        (CompressedFormat::Etc2Rgba8, T::Etc2Rgba8Unorm, 4, 6.0),
        (CompressedFormat::EacR11, T::EacR11Unorm, 1, 4.0),
        (CompressedFormat::EacRg11, T::EacRg11Unorm, 2, 4.0),
    ];
    let mut tested = 0;
    for (f, gpu_format, channels, max_mean) in cases {
        let needed = gpu_format.required_features();
        if !features.contains(needed) {
            eprintln!("{f:?}: not supported here; skipped");
            continue;
        }
        let data = compress_level(f, TextureCompression::NormalQuality, &rgba, w, h);
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let img = gpu_decode(&gpu, gpu_format, w, h, &data);
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("{f:?}: GPU validation error: {e}");
        }
        let mut sum = 0u64;
        let mut worst = 0u8;
        for i in 0..(w * h) as usize {
            for c in 0..channels {
                let d = img.rgba[i * 4 + c].abs_diff(rgba[i * 4 + c]);
                sum += d as u64;
                worst = worst.max(d);
            }
        }
        let mean = sum as f64 / (w * h) as f64 / channels as f64;
        eprintln!("{f:?}: mean error {mean:.2}, worst {worst}");
        assert!(mean < max_mean, "{f:?}: mean error {mean:.2}");
        tested += 1;
    }
    eprintln!("{tested} formats decoded");
}

#[test]
fn cube_and_array_shapes_import_and_upload() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("oxide-shapes-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Assets")).unwrap();
    // A 4×3 cross of 16 px faces, each face a different grey.
    let s = 16u32;
    let (w, h) = (4 * s, 3 * s);
    let cells = [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)];
    let mut img = vec![0u8; (w * h * 4) as usize];
    for (f, (cx, cy)) in cells.iter().enumerate() {
        for y in 0..s {
            for x in 0..s {
                let i = (((cy * s + y) * w + cx * s + x) * 4) as usize;
                img[i..i + 4].copy_from_slice(&[f as u8 * 40, 0, 0, 255]);
            }
        }
    }
    write_png(&dir.join("Assets/sky_cross.png"), w, h, &img);
    // A 2×2 grid of layers.
    let grid: Vec<u8> = (0..32 * 32)
        .flat_map(|i| {
            let (x, y) = (i % 32, i / 32);
            [((x / 16) + 2 * (y / 16)) as u8 * 60, 0, 0, 255]
        })
        .collect();
    write_png(&dir.join("Assets/layers.png"), 32, 32, &grid);

    let mut assets = AssetServer::new(&dir);
    let mut imp = assets.texture_importer("Assets/sky_cross.png");
    imp.texture_shape = oxide_assets::TextureShape::Cube;
    assets
        .set_texture_importer("Assets/sky_cross.png", &imp)
        .unwrap();
    let cube = assets
        .cubemaps
        .get(&Handle::from_path("Assets/sky_cross.png".to_owned()))
        .expect("cubemap made");
    assert_eq!((cube.faces.len(), cube.size()), (6, s));
    for (f, face) in cube.faces.iter().enumerate() {
        assert_eq!(face.mips[0][0], f as u8 * 40, "face {f}");
        assert!(face.compressed.is_some(), "faces are compressed");
    }
    let mut imp = assets.texture_importer("Assets/layers.png");
    imp.texture_shape = oxide_assets::TextureShape::Texture2DArray;
    imp.columns = 2;
    imp.rows = 2;
    assets
        .set_texture_importer("Assets/layers.png", &imp)
        .unwrap();
    let arr = assets
        .texture_arrays
        .get(&Handle::from_path("Assets/layers.png".to_owned()))
        .expect("array made");
    assert_eq!(arr.depth(), 4);
    assert_eq!(arr.layers[3].mips[0][0], 180);
    // Back to 2D: the cube goes away.
    let mut imp = assets.texture_importer("Assets/sky_cross.png");
    imp.texture_shape = oxide_assets::TextureShape::Texture2D;
    assets
        .set_texture_importer("Assets/sky_cross.png", &imp)
        .unwrap();
    assert!(assets.cubemaps.is_empty());

    // Both upload as cube / 2D-array views.
    imp.texture_shape = oxide_assets::TextureShape::Cube;
    assets
        .set_texture_importer("Assets/sky_cross.png", &imp)
        .unwrap();
    let mut ga = gpu_assets::GpuAssets::new(&gpu.device, &gpu.queue);
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let cube_id = AssetId::from_path("Assets/sky_cross.png");
    assert!(
        ga.cubemap_view(&gpu.device, &gpu.queue, &assets, cube_id)
            .is_some()
    );
    assert!(
        ga.texture_array_view(
            &gpu.device,
            &gpu.queue,
            &assets,
            AssetId::from_path("Assets/layers.png")
        )
        .is_some()
    );
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
