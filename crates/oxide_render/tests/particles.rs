//! GPU test for the particle pass: billboard, stretched, horizontal and segment quads and
//! texture sheet rectangles. Skipped without a GPU.

use oxide_assets::{AssetServer, Texture2D};
use oxide_math::{Color, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 64;

fn frame(particles: Vec<ParticleBatch>) -> RenderFrame {
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
        particles,
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

/// Bounding box (x0, y0, x1, y1) of pixels matching `f` inside a region.
fn bbox(
    img: &CapturedImage,
    region: (u32, u32, u32, u32),
    f: impl Fn([u8; 4]) -> bool,
) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for y in region.1..region.3 {
        for x in region.0..region.2 {
            if f(img.pixel(x, y)) {
                b = Some(match b {
                    None => (x, y, x, y),
                    Some((a, c, d, e)) => (a.min(x), c.min(y), d.max(x), e.max(y)),
                });
            }
        }
    }
    b
}

#[test]
fn particle_modes_and_sheet_tiles() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(".");
    let white = assets
        .textures
        .insert("test:white", Texture2D::solid("white", [255; 4], true))
        .id();
    // Two tiles: red | green.
    let mut px = Vec::new();
    for _ in 0..8 {
        for x in 0..16 {
            px.extend_from_slice(if x < 8 {
                &[255, 0, 0, 255]
            } else {
                &[0, 255, 0, 255]
            });
        }
    }
    let sheet = assets
        .textures
        .insert(
            "test:sheet",
            Texture2D::from_rgba8("sheet", 16, 8, px, true),
        )
        .id();
    let c = [1.0, 1.0, 1.0, 1.0];
    // 32 px per unit; +x right, +y up on screen (camera looks along +Z).
    let frame = frame(vec![
        ParticleBatch {
            texture: Some(white),
            blend: ParticleBlend::Alpha,
            particles: vec![
                // Top-left: a 0.5 m billboard.
                ParticleInstance::billboard([-0.5, 0.5, 0.0], 0.5, c, 0.0),
                // Top-right: stretched up, 0.25 wide and 1 m long.
                ParticleInstance::billboard([0.5, 0.5, 0.0], 0.25, c, 0.0)
                    .with_mode(ParticleMode::Stretched, [0.0, 0.5, 0.0]),
                // Bottom: a segment 1.5 m long, 0.25 m wide.
                ParticleInstance::billboard([0.0, -0.75, 0.0], 0.25, c, 0.0)
                    .with_mode(ParticleMode::Segment, [0.75, 0.0, 0.0]),
            ],
        },
        ParticleBatch {
            texture: Some(sheet),
            blend: ParticleBlend::Alpha,
            // Middle-left: the green (second) tile only.
            particles: vec![
                ParticleInstance::billboard([-0.5, -0.25, 0.0], 0.25, c, 0.0)
                    .with_uv([0.5, 0.0, 0.5, 1.0]),
            ],
        },
    ]);
    let mut r = SceneRenderer::new(&gpu);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
        r.render(&gpu, &assets, &frame, view, fmt, SIZE, SIZE);
    })
    .unwrap();
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    if let Ok(dir) = std::env::var("OXIDE_DUMP_PARTICLES") {
        std::fs::write(format!("{dir}/particles.png"), img.to_png().unwrap()).unwrap();
    }
    let lit = |p: [u8; 4]| p[0] > 200 && p[1] > 200 && p[2] > 200;
    let size = |b: (u32, u32, u32, u32)| (b.2 - b.0 + 1, b.3 - b.1 + 1);
    let bill = bbox(&img, (0, 0, 32, 24), lit).expect("billboard");
    assert!(matches!(size(bill), (15..=17, 15..=17)), "{:?}", size(bill));
    let stretched = bbox(&img, (32, 0, 64, 32), lit).expect("stretched");
    let (w, h) = size(stretched);
    assert!(
        (7..=9).contains(&w) && (31..=33).contains(&h),
        "stretched {w}x{h}"
    );
    let seg = bbox(&img, (0, 44, 64, 64), lit).expect("segment");
    let (w, h) = size(seg);
    assert!(
        (47..=49).contains(&w) && (7..=9).contains(&h),
        "segment {w}x{h}"
    );
    let green = bbox(&img, (0, 32, 32, 48), |p| p[1] > 200 && p[0] < 60).expect("green tile");
    assert!(matches!(size(green), (7..=9, 7..=9)), "{:?}", size(green));
    assert!(
        bbox(&img, (0, 32, 32, 48), |p| p[0] > 200 && p[1] < 60).is_none(),
        "no red tile"
    );
}
