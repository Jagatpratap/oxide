//! GPU tests for MSAA settings, FXAA and TAA. Skipped when no GPU is present.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3};
use oxide_render::*;

const SIZE: u32 = 96;

fn camera(mode: AntiAliasing) -> CameraView {
    CameraView {
        position: Vector3::new(0.0, 0.0, -5.0),
        rotation: Quaternion::IDENTITY,
        projection: ProjectionKind::Perspective { fov: 40.0 },
        near: 0.1,
        far: 100.0,
        viewport: [0.0, 0.0, 1.0, 1.0],
        clear: ClearMode::Color(Color::BLACK),
        culling_mask: u32::MAX,
        occlusion_culling: false,
        antialiasing: AntiAliasingSettings {
            mode,
            ..Default::default()
        },
    }
}

/// A white unlit cube rotated 30° around the view axis (slanted, hard edges) on black.
fn frame(assets: &mut AssetServer, mode: AntiAliasing, msaa: u32, x: f32) -> RenderFrame {
    let white = assets
        .materials
        .insert("test:white", Material::unlit("White", Color::WHITE))
        .id();
    RenderFrame {
        cameras: vec![camera(mode)],
        draws: vec![DrawItem {
            id: 7,
            mesh: AssetId::from_path(&PrimitiveType::Cube.mesh_path()),
            submesh: 0,
            material: white,
            model: Matrix4x4::trs(
                Vector3::new(x, 0.0, 0.0),
                Quaternion::euler(0.0, 0.0, 30.0),
                Vector3::splat(1.2),
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
        }],
        environment: Environment {
            post: PostSettings::neutral(),
            msaa_samples: msaa,
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
    .expect("capture");
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    if let Ok(dir) = std::env::var("OXIDE_TEST_IMAGES") {
        static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let i = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::fs::write(format!("{dir}/aa_{i}.png"), img.to_png().unwrap()).unwrap();
    }
    img
}

/// Pixels that are neither (nearly) black nor white: the smoothed edge.
fn edge_pixels(img: &CapturedImage) -> usize {
    (0..SIZE * SIZE)
        .filter(|i| {
            let v = img.rgba[*i as usize * 4];
            (24..232).contains(&v)
        })
        .count()
}

fn gpu() -> Option<Gpu> {
    let g = Gpu::headless();
    if g.is_none() {
        eprintln!("no GPU; skipped");
    }
    g
}

#[test]
fn msaa_and_fxaa_smooth_edges() {
    let Some(gpu) = gpu() else { return };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);

    let f = frame(&mut assets, AntiAliasing::None, 1, 0.0);
    let aliased = edge_pixels(&render(&gpu, &mut r, &assets, &f));
    assert_eq!(r.sample_count(), 1);
    let f = frame(&mut assets, AntiAliasing::None, 4, 0.0);
    let msaa = edge_pixels(&render(&gpu, &mut r, &assets, &f));
    assert_eq!(r.sample_count(), r.sample_count_for(4));
    let f = frame(&mut assets, AntiAliasing::Fxaa, 1, 0.0);
    let fxaa = edge_pixels(&render(&gpu, &mut r, &assets, &f));
    assert!(
        aliased < 10,
        "no AA should give hard edges, got {aliased} grey pixels"
    );
    if r.sample_count_for(4) > 1 {
        assert!(msaa > aliased + 40, "MSAA {msaa} vs none {aliased}");
    }
    assert!(fxaa > aliased + 40, "FXAA {fxaa} vs none {aliased}");
}

#[test]
fn taa_converges_and_does_not_ghost() {
    let Some(gpu) = gpu() else { return };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = frame(&mut assets, AntiAliasing::Taa, 4, 0.0);

    // Static: jittered frames accumulate into smooth edges that stop changing.
    let mut last = None;
    for i in 0..24 {
        f.frame_index = i;
        last = Some(render(&gpu, &mut r, &assets, &f));
    }
    let a = last.take().unwrap();
    assert_eq!(r.sample_count(), 1, "TAA turns MSAA off");
    let smooth = edge_pixels(&a);
    assert!(
        smooth > 40,
        "TAA edges should be smoothed, got {smooth} grey pixels"
    );
    f.frame_index = 24;
    let b = render(&gpu, &mut r, &assets, &f);
    let changed = (0..a.rgba.len())
        .filter(|&i| a.rgba[i].abs_diff(b.rgba[i]) > 24)
        .count();
    assert!(
        changed < 30,
        "a converged static image barely changes, {changed} channels did"
    );

    // Moving: slide the cube right; the area it left must go back to black quickly.
    for step in 0..12 {
        f.frame_index = 25 + step;
        f.draws[0].model = Matrix4x4::trs(
            Vector3::new(step as f32 * 0.15, 0.0, 0.0),
            Quaternion::euler(0.0, 0.0, 30.0),
            Vector3::splat(1.2),
        );
        last = Some(render(&gpu, &mut r, &assets, &f));
    }
    let img = last.unwrap();
    // ~26 px per metre at the cube's distance: its center is now 1.65 m (43 px) right.
    // Just behind its trailing edge was covered a frame ago and must already be empty.
    for x in [SIZE / 2 - 18, SIZE / 2 + 10] {
        let ghost = img.pixel(x, SIZE / 2);
        assert!(ghost[0] < 40, "ghost trail at x={x}: {ghost:?}");
    }
    let body = img.pixel(SIZE / 2 + 43, SIZE / 2);
    assert!(body[0] > 200, "the cube is where it moved to: {body:?}");
}

#[test]
fn smaa_smooths_edges_towards_the_msaa_reference() {
    let Some(gpu) = gpu() else { return };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    // Shallow edges (8°): long stair steps, SMAA's orthogonal patterns.
    let shallow = |assets: &mut AssetServer, mode, msaa| {
        let mut f = frame(assets, mode, msaa, 0.0);
        f.draws[0].model = Matrix4x4::trs(
            Vector3::ZERO,
            Quaternion::euler(0.0, 0.0, 8.0),
            Vector3::splat(1.2),
        );
        f
    };
    let f = shallow(&mut assets, AntiAliasing::None, 1);
    let aliased_img = render(&gpu, &mut r, &assets, &f);
    let aliased = edge_pixels(&aliased_img);
    // Reference: the highest MSAA the adapter has.
    let f = shallow(&mut assets, AntiAliasing::None, 8);
    let reference = render(&gpu, &mut r, &assets, &f);
    let msaa_on = r.sample_count() > 1;
    let error = |img: &CapturedImage| -> u64 {
        img.rgba
            .iter()
            .zip(&reference.rgba)
            .step_by(4)
            .map(|(a, b)| (*a as i64 - *b as i64).unsigned_abs())
            .sum()
    };
    let mut prev_steps = 0;
    for quality in [
        aa::SmaaQuality::Low,
        aa::SmaaQuality::Medium,
        aa::SmaaQuality::High,
    ] {
        let mut f = shallow(&mut assets, AntiAliasing::Smaa, 1);
        f.cameras[0].antialiasing.smaa_quality = quality;
        let img = render(&gpu, &mut r, &assets, &f);
        assert!(
            r.scene_passes.iter().any(|p| p == "SMAA") || r.post_passes.iter().any(|p| p == "SMAA")
        );
        let smaa = edge_pixels(&img);
        eprintln!(
            "{quality:?}: {smaa} edge pixels (none {aliased}); error {} vs aliased {}",
            error(&img),
            error(&aliased_img)
        );
        assert!(
            smaa > aliased + 40,
            "{quality:?}: SMAA {smaa} vs none {aliased}"
        );
        if msaa_on {
            assert!(
                error(&img) * 10 < error(&aliased_img) * 8,
                "{quality:?}: closer to the MSAA reference than no AA"
            );
        }
        // The interior and background stay untouched.
        let c = img.pixel(SIZE / 2, SIZE / 2);
        assert_eq!(c, aliased_img.pixel(SIZE / 2, SIZE / 2));
        assert_eq!(img.pixel(2, 2), aliased_img.pixel(2, 2));
        let steps = quality.params().1;
        assert!(steps > prev_steps);
        prev_steps = steps;
    }
}
