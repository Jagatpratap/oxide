//! GPU tests for punctual lights (clustered Forward+). Skipped when no GPU is present.

use oxide_assets::{AssetId, AssetServer, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3};
use oxide_render::*;

const W: u32 = 192;
const H: u32 = 108;

/// A dark scene: a big white floor seen from above at an angle, no sun, black sky.
fn floor_frame() -> RenderFrame {
    let eye = Vector3::new(0.0, 12.0, -14.0);
    RenderFrame {
        cameras: vec![CameraView {
            position: eye,
            rotation: Quaternion::look_rotation((Vector3::ZERO - eye).normalize(), Vector3::Y),
            projection: ProjectionKind::Perspective { fov: 60.0 },
            near: 0.3,
            far: 200.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(Color::BLACK),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        draws: vec![DrawItem {
            id: 0,
            mesh: AssetId::from_path(&PrimitiveType::Plane.mesh_path()),
            submesh: 0,
            material: AssetId::from_path(oxide_assets::DEFAULT_MATERIAL),
            model: Matrix4x4::trs(Vector3::ZERO, Quaternion::IDENTITY, Vector3::splat(6.0)),
            cast_shadows: false,
            shadows_only: false,
            receive_shadows: true,
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
            sky_exposure: 0.0,
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
    let img = capture_frame(gpu, fmt, W, H, |view, _| {
        r.render(gpu, assets, f, view, fmt, W, H);
    })
    .expect("capture");
    // Set OXIDE_TEST_IMAGES=<dir> to save every rendered image for inspection.
    if let Ok(dir) = std::env::var("OXIDE_TEST_IMAGES") {
        static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let i = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::fs::write(format!("{dir}/lights_{i}.png"), img.to_png().unwrap()).unwrap();
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    img
}

/// Pixel where world point `p` lands, if it is on screen.
fn pixel_of(f: &RenderFrame, p: Vector3) -> Option<(u32, u32)> {
    let c = f.cameras[0].view_projection(W as f32 / H as f32) * p.extend(1.0);
    let x = (c.x / c.w * 0.5 + 0.5) * W as f32;
    let y = (0.5 - c.y / c.w * 0.5) * H as f32;
    (x >= 0.0 && y >= 0.0 && x < W as f32 && y < H as f32).then_some((x as u32, y as u32))
}

fn point(pos: Vector3, range: f32, color: [f32; 3]) -> PunctualLight {
    PunctualLight {
        position: pos,
        range,
        color,
        spot: None,
        shadow: None,
        ..Default::default()
    }
}

#[test]
fn clustered_lights_have_no_tile_seams() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = floor_frame();
    // One wide light spanning many clusters: brightness must fall off smoothly everywhere
    // (dim enough not to clip, so a missing cluster would show as a step).
    f.lights
        .push(point(Vector3::new(0.0, 3.0, 0.0), 25.0, [8.0, 8.0, 8.0]));
    let img = render(&gpu, &mut r, &assets, &f);
    // Cluster tiles are W/16 × H/9 pixels. A light missing from some clusters shows as a
    // jump across tile boundaries bigger than anything inside the tiles.
    let (tw, th) = (W / 16, H / 9);
    let (mut boundary, mut interior) = (0, 0);
    for y in 0..H - 1 {
        for x in 0..W - 1 {
            let c = img.pixel(x, y)[0] as i32;
            let dx = (c - img.pixel(x + 1, y)[0] as i32).abs();
            let dy = (c - img.pixel(x, y + 1)[0] as i32).abs();
            if (x + 1) % tw == 0 {
                boundary = boundary.max(dx);
            } else {
                interior = interior.max(dx);
            }
            if (y + 1) % th == 0 {
                boundary = boundary.max(dy);
            } else {
                interior = interior.max(dy);
            }
        }
    }
    assert!(
        boundary <= interior + 2,
        "seam: {boundary} across tiles vs {interior} inside"
    );
    let lit = pixel_of(&f, Vector3::ZERO).unwrap();
    assert!(img.pixel(lit.0, lit.1)[0] > 120);
    // The light reaches far: lit well away from the center, dark beyond its range.
    let edge = pixel_of(&f, Vector3::new(12.0, 0.0, 6.0)).unwrap();
    assert!(
        img.pixel(edge.0, edge.1)[0] > 20,
        "{:?}",
        img.pixel(edge.0, edge.1)
    );
}

#[test]
fn many_small_lights_each_light_their_own_spot() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = floor_frame();
    // 300 small lights on a grid; red where x+z is even, blue otherwise.
    let mut spots = Vec::new();
    for i in 0..20 {
        for j in 0..15 {
            let p = Vector3::new(-28.0 + i as f32 * 3.0, 0.4, -20.0 + j as f32 * 3.0);
            let red = (i + j) % 2 == 0;
            let c = if red {
                [6.0, 0.0, 0.0]
            } else {
                [0.0, 0.0, 6.0]
            };
            f.lights.push(point(p, 1.2, c));
            spots.push((p, red));
        }
    }
    let img = render(&gpu, &mut r, &assets, &f);
    let mut checked = 0;
    for (p, red) in spots {
        let Some((x, y)) = pixel_of(&f, Vector3::new(p.x, 0.0, p.z)) else {
            continue;
        };
        let px = img.pixel(x, y);
        if red {
            assert!(px[0] > 60 && px[2] < 20, "red light at {p:?}: {px:?}");
        } else {
            assert!(px[2] > 60 && px[0] < 20, "blue light at {p:?}: {px:?}");
        }
        checked += 1;
    }
    assert!(checked > 100, "only {checked} lights on screen");
    // Between lights (1.5 m from any): dark.
    let (x, y) = pixel_of(&f, Vector3::new(-1.0 + 0.5, 0.0, -5.0 + 1.5)).unwrap();
    assert!(img.pixel(x, y)[0] < 10 && img.pixel(x, y)[2] < 10);
}

fn shadow_settings() -> ShadowSettings {
    ShadowSettings {
        soft: true,
        strength: 1.0,
        bias: 0.05,
        normal_bias: 0.4,
    }
}

#[test]
fn spot_and_point_lights_cast_shadows() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = floor_frame();
    // A cube hanging 1.5 m above the floor, under the light.
    let mut cube = f.draws[0].clone();
    cube.mesh = AssetId::from_path(&PrimitiveType::Cube.mesh_path());
    cube.model = Matrix4x4::trs(
        Vector3::new(0.0, 1.5, 0.0),
        Quaternion::IDENTITY,
        Vector3::splat(1.2),
    );
    cube.cast_shadows = true;
    f.draws.push(cube);

    let below = pixel_of(&f, Vector3::new(0.0, 0.0, 0.0)).unwrap();
    let beside = pixel_of(&f, Vector3::new(2.5, 0.0, 0.0)).unwrap();
    let lum = |img: &CapturedImage, (x, y): (u32, u32)| img.pixel(x, y)[0] as i32;
    let lights = [
        (
            "spot",
            PunctualLight {
                position: Vector3::new(0.0, 5.0, 0.0),
                range: 12.0,
                color: [30.0; 3],
                spot: Some((Vector3::NEG_Y, 100.0, 80.0)),
                shadow: None,
                ..Default::default()
            },
        ),
        ("point", point(Vector3::new(0.0, 5.0, 0.0), 12.0, [30.0; 3])),
    ];
    for (name, light) in lights {
        f.lights = vec![light.clone()];
        let unshadowed = render(&gpu, &mut r, &assets, &f);
        f.lights[0].shadow = Some(shadow_settings());
        let shadowed = render(&gpu, &mut r, &assets, &f);
        // The scene ran on the render graph: shadows first, then the objects.
        let at = |name: &str| r.scene_passes.iter().position(|p| p == name);
        assert!(
            at("Shadow Atlas") < at("Draw Objects"),
            "{:?}",
            r.scene_passes
        );
        assert!(at("Shadow Atlas").is_some(), "{:?}", r.scene_passes);
        assert!(lum(&unshadowed, below) > 100, "{name}: lit without shadows");
        assert!(
            lum(&shadowed, below) < lum(&unshadowed, below) / 4,
            "{name}: shadow under the cube: {} vs {}",
            lum(&shadowed, below),
            lum(&unshadowed, below)
        );
        // Open floor stays lit (no acne): within a few levels of the unshadowed render.
        assert!(
            (lum(&shadowed, beside) - lum(&unshadowed, beside)).abs() <= 4,
            "{name}: open floor {} vs {}",
            lum(&shadowed, beside),
            lum(&unshadowed, beside)
        );
    }
}

#[test]
fn rendering_layers_pick_which_lights_reach_objects() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = floor_frame();
    f.draws[0].rendering_layers = 0b01;
    let centre = pixel_of(&f, Vector3::ZERO).unwrap();
    for path in [RenderingPath::ForwardPlus, RenderingPath::Deferred] {
        f.rendering_path = path;
        for (light_layers, lit) in [(0b01, true), (0b10, false), (0b11, true)] {
            f.lights = vec![PunctualLight {
                rendering_layers: light_layers,
                ..point(Vector3::new(0.0, 3.0, 0.0), 25.0, [8.0; 3])
            }];
            f.sun = Some(SunLight {
                rendering_layers: light_layers,
                color: [0.5; 3],
                ..Default::default()
            });
            let img = render(&gpu, &mut r, &assets, &f);
            let v = img.pixel(centre.0, centre.1)[0];
            assert_eq!(v > 60, lit, "{path:?}, light layers {light_layers:b}: {v}");
        }
    }
}

#[test]
fn spot_and_directional_cookies_shape_the_light() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    // Left half black, right half white.
    let half: Vec<u8> = (0..16 * 16)
        .flat_map(|i| if i % 16 < 8 { [0, 0, 0, 255] } else { [255; 4] })
        .collect();
    let cookie = assets
        .textures
        .insert(
            "test:cookie.png",
            oxide_assets::Texture2D::from_rgba8("cookie", 16, 16, half, false),
        )
        .id();
    let mut r = SceneRenderer::new(&gpu);
    let mut f = floor_frame();
    let lum = |img: &CapturedImage, f: &RenderFrame, p: Vector3| {
        let (x, y) = pixel_of(f, p).unwrap();
        img.pixel(x, y)[0] as i32
    };
    // Spot straight down; the light's right is +X, so the cookie's white half lights x > 0.
    f.lights = vec![PunctualLight {
        spot: Some((Vector3::NEG_Y, 100.0, 80.0)),
        cookie: Some(cookie),
        right: Vector3::X,
        ..point(Vector3::new(0.0, 5.0, 0.0), 12.0, [30.0; 3])
    }];
    let img = render(&gpu, &mut r, &assets, &f);
    let (left, right) = (
        lum(&img, &f, Vector3::new(-1.5, 0.0, 0.0)),
        lum(&img, &f, Vector3::new(1.5, 0.0, 0.0)),
    );
    assert!(
        right > 80 && left < 10,
        "spot cookie: left {left}, right {right}"
    );

    // Directional light, cookie tiles 4 m wide: the black half covers x in [0, 2), white
    // [2, 4), repeating.
    f.lights.clear();
    f.sun = Some(SunLight {
        direction: Vector3::NEG_Y,
        color: [1.0; 3],
        cookie: Some(SunCookie {
            texture: cookie,
            size: [4.0, 4.0],
            offset: [0.0, 0.0],
        }),
        right: Vector3::X,
        ..Default::default()
    });
    let img = render(&gpu, &mut r, &assets, &f);
    let z = -1.0;
    let (black, white, black2) = (
        lum(&img, &f, Vector3::new(1.0, 0.0, z)),
        lum(&img, &f, Vector3::new(3.0, 0.0, z)),
        lum(&img, &f, Vector3::new(5.0, 0.0, z)),
    );
    assert!(
        white > 80 && black < 10 && black2 < 10,
        "sun cookie tiles: {black} {white} {black2}"
    );
}
