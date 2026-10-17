//! GPU test for the Deferred rendering path: it must light a scene like Forward does.
//! Skipped when no GPU is present.

use oxide_assets::{AssetId, AssetServer, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3};
use oxide_render::*;

const W: u32 = 160;
const H: u32 = 96;

fn draw(id: u64, mesh: PrimitiveType, material: AssetId, model: Matrix4x4) -> DrawItem {
    DrawItem {
        id,
        mesh: AssetId::from_path(&mesh.mesh_path()),
        submesh: 0,
        material,
        model,
        cast_shadows: true,
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
    }
}

/// A sunlit floor with a shiny red cube (casting a shadow), a blue unlit sphere, a green
/// point light and the procedural sky behind.
fn scene(assets: &mut AssetServer) -> RenderFrame {
    let mut red = Material::lit("red", Color::rgb(0.8, 0.1, 0.1));
    red.metallic = 0.3;
    red.smoothness = 0.7;
    let red = assets.materials.insert("test:red", red).id();
    let mut blue = Material::lit("blue", Color::rgb(0.1, 0.2, 0.9));
    blue.shader = oxide_assets::ShaderKind::Unlit;
    let blue = assets.materials.insert("test:blue", blue).id();
    let grey = AssetId::from_path(oxide_assets::DEFAULT_MATERIAL);
    let eye = Vector3::new(0.0, 4.0, -9.0);
    RenderFrame {
        cameras: vec![CameraView {
            position: eye,
            rotation: Quaternion::look_rotation(
                (Vector3::new(0.0, 0.5, 0.0) - eye).normalize(),
                Vector3::Y,
            ),
            projection: ProjectionKind::Perspective { fov: 50.0 },
            near: 0.3,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Sky,
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        sun: Some(SunLight {
            direction: Vector3::new(0.4, -1.0, 0.6).normalize(),
            color: [0.6, 0.57, 0.54],
            shadows: Some(ShadowSettings {
                soft: true,
                strength: 1.0,
                bias: 0.05,
                normal_bias: 0.4,
            }),
            ..Default::default()
        }),
        lights: vec![PunctualLight {
            position: Vector3::new(-2.0, 1.0, -1.0),
            range: 5.0,
            color: [0.0, 6.0, 0.0],
            spot: None,
            shadow: None,
            ..Default::default()
        }],
        draws: vec![
            draw(
                1,
                PrimitiveType::Plane,
                grey,
                Matrix4x4::trs(Vector3::ZERO, Quaternion::IDENTITY, Vector3::splat(2.0)),
            ),
            draw(
                2,
                PrimitiveType::Cube,
                red,
                Matrix4x4::trs(
                    Vector3::new(1.0, 0.5, 0.0),
                    Quaternion::euler(0.0, 30.0, 0.0),
                    Vector3::ONE,
                ),
            ),
            draw(
                3,
                PrimitiveType::Sphere,
                blue,
                Matrix4x4::trs(
                    Vector3::new(-1.5, 0.5, 1.5),
                    Quaternion::IDENTITY,
                    Vector3::ONE,
                ),
            ),
        ],
        environment: Environment {
            msaa_samples: 1,
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
    if let Ok(dir) = std::env::var("OXIDE_TEST_IMAGES") {
        let name = format!("{dir}/deferred_{:?}.png", f.rendering_path);
        std::fs::write(name, img.to_png().unwrap()).unwrap();
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    img
}

#[test]
fn deferred_matches_forward() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = scene(&mut assets);
    f.rendering_path = RenderingPath::ForwardPlus;
    let forward = render(&gpu, &mut r, &assets, &f);
    assert!(
        !r.scene_passes.iter().any(|p| p == "GBuffer"),
        "{:?}",
        r.scene_passes
    );
    f.rendering_path = RenderingPath::Deferred;
    let deferred = render(&gpu, &mut r, &assets, &f);
    let at = |name: &str| r.scene_passes.iter().position(|p| p == name);
    assert!(
        at("GBuffer") < at("Deferred Lighting") && at("Deferred Lighting") < at("Draw Objects"),
        "{:?}",
        r.scene_passes
    );
    assert!(at("GBuffer").is_some());

    let mut worst = 0u8;
    let mut differ = 0;
    for y in 0..H {
        for x in 0..W {
            let (a, b) = (forward.pixel(x, y), deferred.pixel(x, y));
            let d = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap();
            worst = worst.max(d);
            if d > 6 {
                differ += 1;
            }
        }
    }
    // The G-buffer stores albedo in 8 bits; everything else matches.
    assert!(
        differ < (W * H / 200) as usize,
        "{differ} pixels differ (worst {worst})"
    );
}

#[test]
fn custom_pipeline_steps_replace_the_universal_order() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = scene(&mut assets);
    let green = assets
        .materials
        .insert("test:green", {
            let mut m = Material::lit("green", Color::rgb(0.0, 1.0, 0.0));
            m.shader = oxide_assets::ShaderKind::Unlit;
            m
        })
        .id();
    // The cube is on layer 3; draw only layer 3, in flat green, on a solid background.
    f.draws[1].layer = 3;
    f.cameras[0].clear = ClearMode::Color(Color::BLACK);
    f.custom_steps = Some(vec![
        RenderStep::ClearRenderTarget {
            color: true,
            depth: true,
        },
        RenderStep::DrawRenderers {
            queue: RenderQueue::Opaque,
            layer_mask: 1 << 3,
            override_material: Some(green),
        },
    ]);
    let img = render(&gpu, &mut r, &assets, &f);
    let (mut greens, mut other) = (0, 0);
    for y in 0..H {
        for x in 0..W {
            let p = img.pixel(x, y);
            if p[1] > 200 && p[0] < 30 && p[2] < 30 {
                greens += 1;
            } else if p[0] > 20 || p[1] > 20 || p[2] > 20 {
                other += 1;
            }
        }
    }
    assert!(greens > 100, "the cube is drawn green ({greens} px)");
    assert_eq!(other, 0, "nothing else is drawn: no floor, sphere or sky");
}

#[test]
fn simple_lit_baked_lit_and_specular_workflow() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(std::env::temp_dir());
    let mut r = SceneRenderer::new(&gpu);
    let mut f = scene(&mut assets);
    // The cube's pixels: the clearly red ones in the Lit render.
    let lit_img = render(&gpu, &mut r, &assets, &f);
    let cube_px: Vec<(u32, u32)> = (0..W * H)
        .map(|i| (i % W, i / W))
        .filter(|&(x, y)| {
            let p = lit_img.pixel(x, y);
            p[0] > 60 && p[0] as u32 > p[1] as u32 * 2 && p[0] as u32 > p[2] as u32 * 2
        })
        .collect();
    assert!(cube_px.len() > 30, "cube found ({} px)", cube_px.len());
    let cube_center = |img: &CapturedImage| {
        let mut sum = [0u32; 3];
        for &(x, y) in &cube_px {
            let p = img.pixel(x, y);
            for c in 0..3 {
                sum[c] += p[c] as u32;
            }
        }
        sum.map(|s| s / cube_px.len() as u32)
    };
    let lit = cube_center(&lit_img);
    fn set(
        assets: &mut AssetServer,
        f: &mut RenderFrame,
        kind: oxide_assets::ShaderKind,
        workflow: oxide_assets::WorkflowMode,
        path: &str,
    ) {
        let mut m = Material::lit("m", Color::rgb(0.8, 0.1, 0.1));
        m.shader = kind;
        m.workflow_mode = workflow;
        m.specular_color = Color::rgb(0.5, 0.5, 0.5);
        m.smoothness = 0.7;
        f.draws[1].material = assets.materials.insert(path, m).id();
    }
    set(
        &mut assets,
        &mut f,
        oxide_assets::ShaderKind::BakedLit,
        oxide_assets::WorkflowMode::Metallic,
        "test:baked",
    );
    let baked = cube_center(&render(&gpu, &mut r, &assets, &f));
    // No sun on Baked Lit: only the ambient remains.
    assert!(baked[0] + 20 < lit[0], "baked {baked:?} vs lit {lit:?}");
    for (kind, workflow, path) in [
        (
            oxide_assets::ShaderKind::SimpleLit,
            oxide_assets::WorkflowMode::Metallic,
            "test:simple",
        ),
        (
            oxide_assets::ShaderKind::Lit,
            oxide_assets::WorkflowMode::Specular,
            "test:specular",
        ),
        (
            oxide_assets::ShaderKind::BakedLit,
            oxide_assets::WorkflowMode::Metallic,
            "test:baked2",
        ),
    ] {
        set(&mut assets, &mut f, kind, workflow, path);
        f.rendering_path = RenderingPath::ForwardPlus;
        let fwd = render(&gpu, &mut r, &assets, &f);
        f.rendering_path = RenderingPath::Deferred;
        let def = render(&gpu, &mut r, &assets, &f);
        let c = cube_center(&fwd);
        assert!(c[0] > c[1] + 20, "{kind:?}: the cube is still red {c:?}");
        let differ = (0..W * H)
            .filter(|i| {
                let (a, b) = (fwd.pixel(i % W, i / W), def.pixel(i % W, i / W));
                (0..3).any(|c| a[c].abs_diff(b[c]) > 6)
            })
            .count();
        assert!(
            differ < (W * H / 200) as usize,
            "{kind:?} {workflow:?}: deferred differs in {differ} px"
        );
    }
}
