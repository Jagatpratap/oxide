//! GPU test: Mixed lights against baked data — Shadowmask (static shadows from the mask),
//! Subtractive (direct light already in the lightmap) and Directional lightmaps — in the
//! Forward+ and Deferred paths. Skipped without a GPU.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType, Texture2D};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3, Vector4};
use oxide_render::*;

const SIZE: u32 = 32;

fn solid(assets: &mut AssetServer, path: &str, px: [u8; 4]) -> AssetId {
    assets
        .textures
        .insert(
            path.to_owned(),
            Texture2D::from_rgba8(path, 4, 4, px.repeat(16), false),
        )
        .id()
}

#[test]
fn mixed_lights_use_shadowmasks_subtractive_and_directional_data() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let white = assets
        .materials
        .insert("test:white.mat", Material::lit("White", Color::WHITE))
        .id();
    // Lightmap: RGBM of 0.25 (gamma 0.5 / range 2.83).
    let lightmap = solid(&mut assets, "test:lm.png", [45, 45, 45, 255]);
    let mask_lit = solid(&mut assets, "test:mask_lit.png", [255; 4]);
    let mask_dark = solid(&mut assets, "test:mask_dark.png", [0, 255, 255, 255]);
    // A bright lightmap (≈ 1.0, as if the sun were baked in).
    let solid_bright = solid(&mut assets, "test:lm_bright.png", [90, 90, 90, 255]);
    // Direction straight up, rebalanced for an upward normal: decodes unchanged.
    let dir_up = solid(&mut assets, "test:dir.png", [128, 255, 128, 255]);
    let eye = Vector3::new(0.0, 6.0, -0.01);
    let base = RenderFrame {
        cameras: vec![CameraView {
            position: eye,
            rotation: Quaternion::look_rotation(Vector3::NEG_Y, Vector3::Z),
            projection: ProjectionKind::Perspective { fov: 30.0 },
            near: 0.1,
            far: 50.0,
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
            material: white,
            model: Matrix4x4::trs(Vector3::ZERO, Quaternion::IDENTITY, Vector3::ONE),
            cast_shadows: false,
            shadows_only: false,
            receive_shadows: true,
            layer: 0,
            joint_offset: None,
            occluder: false,
            occludee: false,
            reflection_probe_static: false,
            lightmap: Some((0, Vector4::new(1.0, 1.0, 0.0, 0.0))),
            light_probe: None,
            sorting: Default::default(),
            rendering_layers: u32::MAX,
        }],
        sun: Some(SunLight {
            direction: Vector3::NEG_Y,
            color: [1.0; 3],
            mixed: true,
            shadowmask_channel: Some(0),
            ..Default::default()
        }),
        lightmaps: vec![lightmap],
        environment: Environment {
            sky_exposure: 0.0,
            reflection_intensity: 0.0,
            post: PostSettings {
                tonemapping: TonemapMode::None,
                ..PostSettings::neutral()
            },
            ..Default::default()
        },
        ..Default::default()
    };
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
        img.pixel(SIZE / 2, SIZE / 2)[0] as i32
    };
    for path in [RenderingPath::ForwardPlus, RenderingPath::Deferred] {
        let frame = |mode, mask: Option<AssetId>, dir: Option<AssetId>| RenderFrame {
            rendering_path: path,
            mixed_lighting: MixedLighting {
                mode,
                ..Default::default()
            },
            shadowmasks: mask.into_iter().collect(),
            lightmap_directions: dir.into_iter().collect(),
            ..base.clone()
        };
        // Indirect Only: lightmap + realtime sun.
        let full = shoot(&frame(MixedMode::IndirectOnly, None, None));
        // No sun at all: the lightmap alone.
        let mut dark = frame(MixedMode::IndirectOnly, None, None);
        dark.sun = None;
        let baked_only = shoot(&dark);
        assert!(
            full > baked_only + 30,
            "{path:?}: the sun adds light: {full} vs {baked_only}"
        );
        // Shadowmask: an unshadowed mask keeps the sun, a shadowed one removes it.
        let open = shoot(&frame(MixedMode::Shadowmask, Some(mask_lit), None));
        let masked = shoot(&frame(MixedMode::Shadowmask, Some(mask_dark), None));
        assert!(
            (open - full).abs() <= 3,
            "{path:?}: open mask {open} vs {full}"
        );
        assert!(
            (masked - baked_only).abs() <= 3,
            "{path:?}: masked {masked} vs {baked_only}"
        );
        // Subtractive: the Mixed sun is in the lightmap already, so it isn't added again.
        let sub = shoot(&frame(MixedMode::Subtractive, None, None));
        assert!(
            (sub - baked_only).abs() <= 3,
            "{path:?}: subtractive {sub} vs {baked_only}"
        );
        // A dynamic object's realtime shadow is subtracted from the lightmap (not below
        // the shadow colour).
        let mut shadowed = frame(MixedMode::Subtractive, None, None);
        shadowed.lightmaps = vec![solid_bright];
        if let Some(sun) = &mut shadowed.sun {
            sun.shadows = Some(ShadowSettings {
                soft: false,
                strength: 1.0,
                bias: 0.05,
                normal_bias: 0.4,
            });
        }
        let lit_lm = shoot(&shadowed);
        shadowed.draws.push(DrawItem {
            id: 2,
            mesh: AssetId::from_path(&PrimitiveType::Cube.mesh_path()),
            model: Matrix4x4::trs(
                Vector3::new(0.0, 1.0, 0.0),
                Quaternion::IDENTITY,
                Vector3::splat(0.6),
            ),
            cast_shadows: true,
            shadows_only: true,
            lightmap: None,
            ..shadowed.draws[0].clone()
        });
        let under = shoot(&shadowed);
        assert!(
            under < lit_lm - 20,
            "{path:?}: realtime shadow subtracted: {under} vs {lit_lm}"
        );
        assert!(under > 40, "{path:?}: not below the shadow colour: {under}");
        // Directional lightmap whose direction matches the normal: unchanged.
        let directional = shoot(&frame(MixedMode::IndirectOnly, None, Some(dir_up)));
        assert!(
            (directional - full).abs() <= 3,
            "{path:?}: directional {directional} vs {full}"
        );
    }
}
