//! GPU test for Decal Projectors (DBuffer): a decal colours the floor inside its box only,
//! in Forward+ and Deferred. Skipped without a GPU.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3};
use oxide_render::*;

const W: u32 = 160;
const H: u32 = 90;

fn scene(decals: Vec<DecalDraw>, path: RenderingPath) -> RenderFrame {
    let eye = Vector3::new(0.0, 8.0, -8.0);
    RenderFrame {
        cameras: vec![CameraView {
            position: eye,
            rotation: Quaternion::look_rotation((Vector3::ZERO - eye).normalize(), Vector3::Y),
            projection: ProjectionKind::Perspective { fov: 60.0 },
            near: 0.3,
            far: 100.0,
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
            material: AssetId::from_path("test:white"),
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
            sorting: Default::default(),
            rendering_layers: u32::MAX,
        }],
        decals,
        rendering_path: path,
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

fn pixel_of(f: &RenderFrame, p: Vector3) -> (u32, u32) {
    let c = f.cameras[0].view_projection(W as f32 / H as f32) * p.extend(1.0);
    (
        ((c.x / c.w * 0.5 + 0.5) * W as f32) as u32,
        ((0.5 - c.y / c.w * 0.5) * H as f32) as u32,
    )
}

#[test]
fn decals_project_onto_the_floor() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(".");
    assets
        .materials
        .insert("test:white", Material::unlit("white", Color::WHITE));
    // A 2 m box at the origin projecting straight down (local +Z = world -Y), fading out
    // only on surfaces that face away.
    let decal = DecalDraw {
        to_world: Matrix4x4::trs(
            Vector3::ZERO,
            Quaternion::from_rotation_x(std::f32::consts::FRAC_PI_2),
            Vector3::splat(2.0),
        ),
        texture: None,
        color: [1.0, 0.0, 0.0, 1.0],
        uv: [1.0, 1.0, 0.0, 0.0],
        angle_fade: [80.0, 100.0],
    };
    let mut r = SceneRenderer::new(&gpu);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    for path in [RenderingPath::Forward, RenderingPath::Deferred] {
        let f = scene(vec![decal.clone()], path);
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let img = capture_frame(&gpu, fmt, W, H, |view, _| {
            r.render(&gpu, &assets, &f, view, fmt, W, H);
        })
        .unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("GPU validation error: {e}");
        }
        if let Ok(dir) = std::env::var("OXIDE_TEST_IMAGES") {
            std::fs::write(format!("{dir}/decal_{path:?}.png"), img.to_png().unwrap()).unwrap();
        }
        let (cx, cy) = pixel_of(&f, Vector3::ZERO);
        let inside = img.pixel(cx, cy);
        assert!(
            inside[0] > 200 && inside[1] < 40,
            "{path:?}: red under the decal: {inside:?}"
        );
        let (ox, oy) = pixel_of(&f, Vector3::new(6.0, 0.0, 4.0));
        let outside = img.pixel(ox, oy);
        assert!(
            outside.iter().take(3).all(|&c| c > 200),
            "{path:?}: white floor outside: {outside:?}"
        );
    }
    // Projecting upward, the floor faces away: faded out.
    let mut up = decal.clone();
    up.to_world = Matrix4x4::trs(
        Vector3::ZERO,
        Quaternion::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        Vector3::splat(2.0),
    );
    let f = scene(vec![up], RenderingPath::Forward);
    let img = capture_frame(&gpu, fmt, W, H, |view, _| {
        r.render(&gpu, &assets, &f, view, fmt, W, H);
    })
    .unwrap();
    let (cx, cy) = pixel_of(&f, Vector3::ZERO);
    assert!(
        img.pixel(cx, cy).iter().take(3).all(|&c| c > 200),
        "angle fade"
    );
    // Without decals the floor is white everywhere.
    let f = scene(vec![], RenderingPath::Forward);
    let img = capture_frame(&gpu, fmt, W, H, |view, _| {
        r.render(&gpu, &assets, &f, view, fmt, W, H);
    })
    .unwrap();
    let (cx, cy) = pixel_of(&f, Vector3::ZERO);
    assert!(img.pixel(cx, cy).iter().take(3).all(|&c| c > 200));
}
