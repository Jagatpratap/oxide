//! GPU test: textured skyboxes (Cubemap, 6 Sided, Panoramic) draw behind the scene, turn with
//! Rotation, and light objects through the captured ambient. Skipped without a GPU.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType, Texture2D, TextureCube};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3};
use oxide_render::*;

const SIZE: u32 = 32;

fn camera(dir: Vector3) -> CameraView {
    let up = if dir.y.abs() > 0.9 {
        Vector3::Z
    } else {
        Vector3::Y
    };
    CameraView {
        position: Vector3::ZERO,
        rotation: Quaternion::look_rotation(dir, up),
        projection: ProjectionKind::Perspective { fov: 30.0 },
        near: 0.1,
        far: 100.0,
        viewport: [0.0, 0.0, 1.0, 1.0],
        clear: ClearMode::Sky,
        culling_mask: u32::MAX,
        occlusion_culling: false,
        antialiasing: Default::default(),
    }
}

fn solid(c: [u8; 4], size: u32) -> Texture2D {
    Texture2D::from_rgba8("face", size, size, c.repeat((size * size) as usize), true)
}

const FACE_COLORS: [[u8; 4]; 6] = [
    [255, 0, 0, 255],   // +X
    [0, 255, 0, 255],   // -X
    [0, 0, 255, 255],   // +Y
    [255, 255, 0, 255], // -Y
    [255, 0, 255, 255], // +Z
    [0, 255, 255, 255], // -Z
];

#[test]
fn skyboxes_draw_turn_and_light_the_scene() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let cube = assets
        .cubemaps
        .insert(
            "test:sky.cubemap",
            TextureCube {
                name: "sky".into(),
                faces: FACE_COLORS.iter().map(|&c| solid(c, 8)).collect(),
            },
        )
        .id();
    let six: Vec<AssetId> = FACE_COLORS
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            assets
                .textures
                .insert(format!("test:face{i}.png"), solid(c, 8))
                .id()
        })
        .collect();
    // Panorama: left half red, right half blue (the centre is +Z, so +X looks at blue).
    let (pw, ph) = (64u32, 32u32);
    let pano: Vec<u8> = (0..pw * ph)
        .flat_map(|i| {
            if i % pw < pw / 2 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        })
        .collect();
    let pano = assets
        .textures
        .insert(
            "test:pano.png",
            Texture2D::from_rgba8("pano", pw, ph, pano, true),
        )
        .id();
    let red = assets.cubemaps.insert(
        "test:red.cubemap",
        TextureCube {
            name: "red".into(),
            faces: (0..6).map(|_| solid([255, 0, 0, 255], 8)).collect(),
        },
    );
    let white = assets
        .materials
        .insert("test:white.mat", Material::lit("White", Color::WHITE))
        .id();
    let mut r = SceneRenderer::new(&gpu);
    let mut shoot = |f: &RenderFrame| {
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
        let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
            r.render(&gpu, &assets, f, view, fmt, SIZE, SIZE);
        })
        .unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("GPU validation error: {e}");
        }
        img
    };
    let frame = |source, dir, rotation| RenderFrame {
        cameras: vec![camera(dir)],
        environment: Environment {
            post: PostSettings {
                tonemapping: TonemapMode::None,
                ..PostSettings::neutral()
            },
            skybox: Some(Skybox {
                source,
                tint: Color::rgb(0.5, 0.5, 0.5),
                exposure: 1.0,
                rotation,
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let centre = |img: &CapturedImage| {
        let p = img.pixel(SIZE / 2, SIZE / 2);
        [p[0], p[1], p[2]]
    };
    let dirs = [
        Vector3::X,
        Vector3::NEG_X,
        Vector3::Y,
        Vector3::NEG_Y,
        Vector3::Z,
        Vector3::NEG_Z,
    ];
    let ids: [AssetId; 6] = six.clone().try_into().unwrap();
    for source in [SkyboxSource::Cubemap(cube), SkyboxSource::SixSided(ids)] {
        for (face, dir) in dirs.iter().enumerate() {
            let c = centre(&shoot(&frame(source, *dir, 0.0)));
            let want = FACE_COLORS[face];
            for k in 0..3 {
                assert!(
                    c[k].abs_diff(want[k]) < 8,
                    "{source:?} looking {dir:?}: {c:?} vs {want:?}"
                );
            }
        }
    }
    let p = SkyboxSource::Panoramic(pano);
    let c = centre(&shoot(&frame(p, Vector3::X, 0.0)));
    assert!(c[2] > 240 && c[0] < 10, "panorama +X is blue: {c:?}");
    let c = centre(&shoot(&frame(p, Vector3::NEG_X, 0.0)));
    assert!(c[0] > 240 && c[2] < 10, "panorama -X is red: {c:?}");
    // Rotation 180° turns the sky around.
    let c = centre(&shoot(&frame(p, Vector3::X, 180.0)));
    assert!(c[0] > 240 && c[2] < 10, "rotated: {c:?}");

    // An all-red sky lights a white Lit sphere red through the captured ambient.
    let mut f = frame(SkyboxSource::Cubemap(red.id()), Vector3::Z, 0.0);
    f.draws.push(DrawItem {
        id: 1,
        mesh: AssetId::from_path(&PrimitiveType::Sphere.mesh_path()),
        submesh: 0,
        material: white,
        model: Matrix4x4::trs(
            Vector3::new(0.0, 0.0, 4.0),
            Quaternion::IDENTITY,
            Vector3::ONE,
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
        rendering_layers: u32::MAX,
    });
    let c = centre(&shoot(&f));
    assert!(
        c[0] > 40 && c[1] < c[0] / 4 && c[2] < c[0] / 4,
        "red ambient: {c:?}"
    );
}
