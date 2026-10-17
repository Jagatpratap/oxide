//! GPU tests for lens flares: elements land on the source and its mirror, and geometry in
//! front of the source hides them (depth occlusion). Skipped without a GPU.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_render::*;

const SIZE: u32 = 64;

fn element(position: f32, color: [f32; 4]) -> FlareElement {
    FlareElement {
        texture: None,
        shape: FlareShape::Circle,
        blend: FlareBlend::Additive,
        color,
        modulate_by_light_color: false,
        position,
        position_offset: [0.0; 2],
        angular_offset: 0.0,
        translation_scale: [1.0; 2],
        size: [2.0; 2],
        preserve_aspect_ratio: false,
        rotation: 0.0,
        auto_rotate: false,
        shape_params: [1.0, 0.5, 0.0, 6.0],
        inverse: false,
        distortion: None,
    }
}

fn source(occlusion: bool) -> LensFlareSource {
    LensFlareSource {
        position: Vector3::new(1.5, 1.0, 10.0),
        directional: false,
        light_color: [1.0; 3],
        intensity: 1.0,
        scale: 1.0,
        spot: None,
        max_attenuation_distance: 100.0,
        distance_attenuation: [1.0; FLARE_LUT],
        max_attenuation_scale: 100.0,
        scale_by_distance: [1.0; FLARE_LUT],
        radial_screen_attenuation: [1.0; FLARE_LUT],
        occlusion: occlusion.then_some(FlareOcclusion {
            radius: 0.05,
            samples: 16,
            offset: 0.0,
        }),
        allow_off_screen: false,
        elements: vec![
            element(0.0, [1.0, 0.0, 0.0, 1.0]),
            element(2.0, [0.0, 1.0, 0.0, 1.0]),
        ],
    }
}

fn frame(assets: &mut AssetServer, wall: bool, msaa: u32) -> RenderFrame {
    let grey = assets
        .materials
        .insert(
            "test:grey",
            Material::unlit("grey", Color::rgb(0.2, 0.2, 0.2)),
        )
        .id();
    let mut draws = Vec::new();
    if wall {
        draws.push(DrawItem {
            id: 1,
            mesh: AssetId::from_path(&PrimitiveType::Cube.mesh_path()),
            submesh: 0,
            material: grey,
            model: Matrix4x4::trs(
                Vector3::new(0.75, 0.5, 5.0),
                Quaternion::IDENTITY,
                Vector3::new(1.0, 1.0, 0.2),
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
    }
    RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::ZERO,
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Perspective { fov: 40.0 },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(Color::BLACK),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: AntiAliasingSettings {
                allow_msaa: msaa > 1,
                ..Default::default()
            },
        }],
        draws,
        environment: Environment {
            msaa_samples: msaa,
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

/// Pixel of a world point for the test camera (fov 40°, square view).
fn pixel_of(p: Vector3) -> (u32, u32) {
    let t = 20f32.to_radians().tan();
    let (x, y) = (p.x / (p.z * t), p.y / (p.z * t));
    (
        ((x * 0.5 + 0.5) * SIZE as f32) as u32,
        ((0.5 - y * 0.5) * SIZE as f32) as u32,
    )
}

#[test]
fn flares_follow_the_source_and_hide_behind_geometry() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let src = source(true).position;
    let (sx, sy) = pixel_of(src);
    let (mx, my) = (SIZE - 1 - sx, SIZE - 1 - sy);

    for msaa in [1, 4] {
        // No flare: black.
        let mut f = frame(&mut assets, false, msaa);
        let plain = render(&gpu, &mut r, &assets, &f);
        assert_eq!(plain.pixel(sx, sy)[0], 0);

        // Red at the source, green mirrored through the centre, nothing in a corner.
        f.lens_flares.push(source(true));
        let img = render(&gpu, &mut r, &assets, &f);
        let (at_src, at_mirror) = (img.pixel(sx, sy), img.pixel(mx, my));
        assert!(
            at_src[0] > 150 && at_src[1] < 30,
            "msaa {msaa}: source {at_src:?}"
        );
        assert!(
            at_mirror[1] > 150 && at_mirror[0] < 30,
            "msaa {msaa}: mirror {at_mirror:?}"
        );
        assert_eq!(img.pixel(2, 2), [0, 0, 0, 255]);

        // A wall between the camera and the source hides the whole flare...
        let mut f = frame(&mut assets, true, msaa);
        f.lens_flares.push(source(true));
        let img = render(&gpu, &mut r, &assets, &f);
        assert!(
            img.pixel(mx, my)[1] < 5,
            "msaa {msaa}: occluded {:?}",
            img.pixel(mx, my)
        );

        // ...unless occlusion is off.
        f.lens_flares[0] = source(false);
        let img = render(&gpu, &mut r, &assets, &f);
        assert!(
            img.pixel(mx, my)[1] > 150,
            "msaa {msaa}: no occlusion {:?}",
            img.pixel(mx, my)
        );
    }
}

#[test]
fn screen_space_lens_flare_mirrors_bright_spots() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let hot = assets
        .materials
        .insert(
            "test:hot",
            Material::unlit("hot", Color::rgb(60.0, 60.0, 60.0)),
        )
        .id();
    let mut f = frame(&mut assets, false, 1);
    let pos = Vector3::new(1.5, 1.0, 10.0);
    f.draws.push(DrawItem {
        id: 2,
        mesh: AssetId::from_path(&PrimitiveType::Cube.mesh_path()),
        submesh: 0,
        material: hot,
        model: Matrix4x4::trs(pos, Quaternion::IDENTITY, Vector3::splat(0.4)),
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
    let (sx, sy) = pixel_of(pos);
    let (mx, my) = (SIZE - 1 - sx, SIZE - 1 - sy);
    f.environment.post.bloom.threshold = 1.0;
    let off = render(&gpu, &mut r, &assets, &f);
    assert!(off.pixel(sx, sy)[0] > 200, "the hot cube is visible");
    assert!(
        off.pixel(mx, my)[0] < 3,
        "nothing mirrored: {:?}",
        off.pixel(mx, my)
    );

    f.environment.post.screen_space_lens_flare = ScreenSpaceLensFlareSettings {
        intensity: 1.0,
        // Only the mirrored ghost, at the mirrored spot exactly.
        starting_position: 1.0,
        secondary_flare_intensity: 0.0,
        warped_flare_intensity: 0.0,
        bloom_mip: 0,
        resolution_mip: 0,
        ..Default::default()
    };
    let on = render(&gpu, &mut r, &assets, &f);
    assert!(
        on.pixel(mx, my)[0] > 20,
        "ghost in the mirrored corner: {:?}",
        on.pixel(mx, my)
    );
    assert!(
        on.pixel(2, 62)[0] < 3,
        "nothing elsewhere: {:?}",
        on.pixel(2, 62)
    );

    // Streaks smear the spot horizontally.
    f.environment
        .post
        .screen_space_lens_flare
        .first_flare_intensity = 0.0;
    f.environment.post.screen_space_lens_flare.streaks_intensity = 1.0;
    let st = render(&gpu, &mut r, &assets, &f);
    let side = (sx - 12, sy);
    assert!(
        st.pixel(side.0, side.1)[0] > off.pixel(side.0, side.1)[0] + 5,
        "streak beside the spot"
    );
    assert!(
        st.pixel(sx, sy - 12)[0] <= off.pixel(sx, sy - 12)[0] + 2,
        "but not above it"
    );
}
