//! The Profiler's GPU pass times (timestamp queries). Skipped without a GPU or timestamps.

use oxide_assets::AssetServer;
use oxide_core::profiler::Profiler;
use oxide_math::{Color, Quaternion, Vector3};
use oxide_render::*;

#[test]
fn render_passes_get_gpu_times() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    if !gpu
        .device
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        eprintln!("no timestamp queries; skipped");
        return;
    }
    let assets = AssetServer::new(".");
    let frame = RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::new(0.0, 1.0, -5.0),
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Perspective { fov: 60.0 },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Sky,
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        ..Default::default()
    };
    let _ = Color::BLACK;
    let mut r = SceneRenderer::new(&gpu);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    Profiler::set_enabled(true);
    let mut found = None;
    for _ in 0..30 {
        Profiler::begin_frame();
        capture_frame(&gpu, fmt, 64, 64, |view, _| {
            r.render(&gpu, &assets, &frame, view, fmt, 64, 64);
        })
        .unwrap();
        Profiler::end_frame();
        if let Some(f) = Profiler::frames().iter().rev().find(|f| !f.gpu.is_empty()) {
            found = Some(f.clone());
            break;
        }
    }
    Profiler::set_enabled(false);
    let f = found.expect("GPU times arrive within a few frames");
    let names: Vec<&str> = f.gpu.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"Draw Objects"), "{names:?}");
    assert!(
        f.gpu.iter().all(|(_, ms)| *ms >= 0.0 && *ms < 1000.0),
        "{:?}",
        f.gpu
    );
    assert!(f.gpu_ms() > 0.0);
    let draw = f.gpu.iter().find(|(n, _)| n == "Draw Objects").unwrap().1;
    assert!(
        draw > 0.0,
        "Draw Objects is timed by its own render pass: {:?}",
        f.gpu
    );
}
