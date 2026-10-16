//! GPU test for screen-space ambient occlusion. Skipped when no GPU is present.

use oxide_assets::{AssetId, AssetServer, PrimitiveType};
use oxide_math::{Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3};
use oxide_render::*;

const W: u32 = 160;
const H: u32 = 120;

fn draw(mesh: PrimitiveType, pos: Vector3, scale: Vector3) -> DrawItem {
    DrawItem {
        id: 0,
        mesh: AssetId::from_path(&mesh.mesh_path()),
        submesh: 0,
        material: AssetId::from_path(oxide_assets::DEFAULT_MATERIAL),
        model: Matrix4x4::trs(pos, Quaternion::IDENTITY, scale),
        cast_shadows: false,
        shadows_only: false,
        receive_shadows: false,
        layer: 0,
        joint_offset: None,
        occluder: false,
        occludee: false,
        reflection_probe_static: false,
    }
}

fn scene(ssao: Option<SsaoSettings>) -> RenderFrame {
    let eye = Vector3::new(0.0, 2.0, -4.0);
    RenderFrame {
        cameras: vec![CameraView {
            position: eye,
            rotation: Quaternion::look_rotation(
                (Vector3::new(0.0, 0.3, 0.0) - eye).normalize(),
                Vector3::Y,
            ),
            projection: ProjectionKind::Perspective { fov: 50.0 },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Sky,
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        // Ambient (sky) light only, so AO is what shades the corner.
        draws: vec![
            draw(PrimitiveType::Plane, Vector3::ZERO, Vector3::ONE),
            draw(
                PrimitiveType::Cube,
                Vector3::new(0.0, 0.5, 0.0),
                Vector3::ONE,
            ),
        ],
        environment: Environment {
            post: PostSettings::neutral(),
            ssao,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Pixel where world point `p` lands.
fn pixel_of(f: &RenderFrame, p: Vector3) -> (u32, u32) {
    let c = f.cameras[0].view_projection(W as f32 / H as f32) * p.extend(1.0);
    let (x, y) = (c.x / c.w, c.y / c.w);
    (
        ((x * 0.5 + 0.5) * W as f32) as u32,
        ((0.5 - y * 0.5) * H as f32) as u32,
    )
}

fn luma(img: &CapturedImage, (x, y): (u32, u32)) -> i32 {
    let p = img.pixel(x, y);
    (p[0] as i32 + p[1] as i32 + p[2] as i32) / 3
}

#[test]
fn ssao_darkens_contact_not_open_ground() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut shot = |f: &RenderFrame| {
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let img = capture_frame(&gpu, fmt, W, H, |view, _| {
            r.render(&gpu, &assets, f, view, fmt, W, H);
        })
        .expect("capture");
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("GPU validation error: {e}");
        }
        img
    };
    let off = scene(None);
    let on = scene(Some(SsaoSettings::default()));
    let corner = pixel_of(&off, Vector3::new(0.0, 0.0, -0.56));
    let open = pixel_of(&off, Vector3::new(2.2, 0.0, -1.5));
    let a = shot(&off);
    let b = shot(&on);
    let (c0, c1) = (luma(&a, corner), luma(&b, corner));
    let (o0, o1) = (luma(&a, open), luma(&b, open));
    assert!(c1 < c0 - 12, "contact shadow: {c0} -> {c1}");
    assert!((o1 - o0).abs() <= 6, "open ground unchanged: {o0} -> {o1}");
    // Stable: the same frame again gives the same AO.
    let b2 = shot(&on);
    assert_eq!(luma(&b2, corner), c1);
}
