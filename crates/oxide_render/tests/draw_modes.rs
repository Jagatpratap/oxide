//! GPU tests for the Scene view draw modes (debug views and wireframe). Skipped without a GPU.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 64;

/// A lit red cube seen head-on (its front face fills the middle of the image).
fn scene(assets: &mut AssetServer) -> RenderFrame {
    let red = assets
        .materials
        .insert("test:red", Material::lit("red", Color::rgb(1.0, 0.0, 0.0)))
        .id();
    RenderFrame {
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
        draws: vec![DrawItem {
            id: 0,
            mesh: AssetId::from_path(&PrimitiveType::Cube.mesh_path()),
            submesh: 0,
            material: red,
            model: Matrix4x4::trs(Vector3::ZERO, Quaternion::IDENTITY, Vector3::splat(2.0)),
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
    .unwrap();
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    img
}

/// Pixels that aren't the black clear color.
fn covered(img: &CapturedImage) -> usize {
    (0..SIZE * SIZE)
        .filter(|i| {
            let p = img.pixel(i % SIZE, i / SIZE);
            p[0] > 8 || p[1] > 8 || p[2] > 8
        })
        .count()
}

#[test]
fn debug_views_show_their_input() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = scene(&mut assets);
    let c = SIZE / 2 + 3;

    f.debug_view = DebugView::Albedo;
    let p = render(&gpu, &mut r, &assets, &f).pixel(c, c);
    // No lights casting shadows and no SSAO: only the objects pass runs.
    assert_eq!(r.scene_passes, vec!["Draw Objects".to_owned()]);
    assert!(
        p[0] > 250 && p[1] < 5 && p[2] < 5,
        "albedo is the material color: {p:?}"
    );

    // The front face points at the camera (-Z): 0.5 * n + 0.5 = (0.5, 0.5, 0).
    f.debug_view = DebugView::Normals;
    let p = render(&gpu, &mut r, &assets, &f).pixel(c, c);
    assert!(
        p[0].abs_diff(p[1]) < 4 && p[0] > 150 && p[2] < 5,
        "normal color: {p:?}"
    );

    // Lighting only replaces the red albedo with neutral grey (tinted only by the blue sky ambient).
    f.debug_view = DebugView::LightingOnly;
    let p = render(&gpu, &mut r, &assets, &f).pixel(c, c);
    assert!(
        p[0] > 40 && p[0] <= p[1] && p[1] <= p[2],
        "grey, not red: {p:?}"
    );
}

#[test]
fn wireframe_draws_edges_only() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    if !r.supports_wireframe() {
        eprintln!("GPU can't draw lines; skipped");
        return;
    }
    let mut f = scene(&mut assets);
    // Light grey clear so the dark edges show.
    f.cameras[0].clear = ClearMode::Color(Color::rgb(0.5, 0.5, 0.5));
    let shaded = render(&gpu, &mut r, &assets, &f);

    f.debug_view = DebugView::Wireframe;
    let wire = render(&gpu, &mut r, &assets, &f);
    let differs = |a: &CapturedImage, b: &CapturedImage| {
        (0..SIZE * SIZE)
            .filter(|i| {
                let (x, y) = (i % SIZE, i / SIZE);
                let (p, q) = (a.pixel(x, y), b.pixel(x, y));
                (0..3).any(|k| p[k].abs_diff(q[k]) > 20)
            })
            .count()
    };
    let bg = render(
        &gpu,
        &mut r,
        &assets,
        &RenderFrame {
            draws: Vec::new(),
            ..f.clone()
        },
    );
    // Edges darken some background pixels, fewer than the solid cube covers.
    let edge_px = differs(&wire, &bg);
    let solid_px = differs(&shaded, &bg);
    assert!(
        edge_px > 20 && edge_px < solid_px,
        "edges {edge_px}, solid {solid_px}"
    );
    // Inside the face but between edges, the background shows through.
    let (p, q) = (
        wire.pixel(SIZE / 2, SIZE / 2 - 7),
        bg.pixel(SIZE / 2, SIZE / 2 - 7),
    );
    assert!(
        (0..3).all(|k| p[k].abs_diff(q[k]) < 4),
        "face interior is empty: {p:?} vs {q:?}"
    );
    assert!(differs(&shaded, &bg) > 0 && shaded.pixel(SIZE / 2, SIZE / 2 - 7) != q);

    f.debug_view = DebugView::ShadedWireframe;
    let both = render(&gpu, &mut r, &assets, &f);
    let overlay_px = differs(&both, &shaded);
    assert!(overlay_px > 20, "edges over the shaded cube: {overlay_px}");
    assert!(covered(&both) >= covered(&shaded));
}
