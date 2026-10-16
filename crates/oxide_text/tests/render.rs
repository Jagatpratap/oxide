//! GPU tests for 3D text: a TextMeshPro goes through layout, extraction and the text pass.
//! Skipped without a GPU.

use oxide_assets::AssetServer;
use oxide_math::{Color, Quaternion, Vector2, Vector3};
use oxide_render::*;
use oxide_scene::World;
use oxide_text::{TextMaterial, TextMeshPro};

const W: u32 = 128;
const H: u32 = 64;

fn world(t: TextMeshPro) -> World {
    let mut w = World::new();
    w.resources.insert(AssetServer::new(std::env::temp_dir()));
    let id = w.create("Text");
    w.add_component(id, t);
    oxide_text::components::update(&mut w);
    w
}

fn frame(w: &World) -> RenderFrame {
    let mut f = RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::new(0.0, 0.0, -10.0),
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Orthographic { size: 2.0 },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(Color::BLACK),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        environment: Environment {
            post: PostSettings {
                tonemapping: TonemapMode::None,
                ..PostSettings::neutral()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    oxide_text::components::extract(w, &mut f.texts);
    f
}

fn render(gpu: &Gpu, r: &mut SceneRenderer, w: &World, f: &RenderFrame) -> CapturedImage {
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let assets = w.resource::<AssetServer>();
    let img = capture_frame(gpu, fmt, W, H, |view, _| {
        r.render(gpu, assets, f, view, fmt, W, H);
    })
    .unwrap();
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("GPU validation error: {e}");
    }
    img
}

fn count(img: &CapturedImage, f: impl Fn([u8; 4]) -> bool) -> usize {
    let mut n = 0;
    for y in 0..H {
        for x in 0..W {
            if f(img.pixel(x, y)) {
                n += 1;
            }
        }
    }
    n
}

fn text(material: TextMaterial) -> TextMeshPro {
    TextMeshPro {
        text: "HI".into(),
        font_size: 20.0,
        rect_size: Vector2::new(4.0, 2.0),
        horizontal_alignment: oxide_text::HorizontalAlignment::Center,
        vertical_alignment: oxide_text::VerticalAlignment::Middle,
        material,
        ..Default::default()
    }
}

#[test]
fn text_renders_with_outline_and_underlay() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut r = SceneRenderer::new(&gpu);
    let w = world(text(TextMaterial::default()));
    let f = frame(&w);
    assert!(!f.texts.is_empty(), "the text was extracted");
    let plain = render(&gpu, &mut r, &w, &f);
    let white = count(&plain, |p| p[0] > 200 && p[1] > 200 && p[2] > 200);
    assert!(white > 60, "white letters drawn: {white}");
    // Centred: about as many lit pixels left and right of the middle.
    let left = (0..H)
        .flat_map(|y| (0..W / 2).map(move |x| (x, y)))
        .filter(|&(x, y)| plain.pixel(x, y)[0] > 128)
        .count();
    let right = (0..H)
        .flat_map(|y| (W / 2..W).map(move |x| (x, y)))
        .filter(|&(x, y)| plain.pixel(x, y)[0] > 128)
        .count();
    assert!(left > 20 && right > 20, "{left} / {right}");
    // Sharp: few half-lit pixels compared with fully lit ones.
    let edge = count(&plain, |p| p[0] > 40 && p[0] < 200);
    assert!(edge < white, "edges {edge} vs solid {white}");

    // A red outline adds red pixels around the white face.
    let w2 = world(text(TextMaterial {
        outline_width: 0.4,
        outline_color: Color::rgb(1.0, 0.0, 0.0),
        ..Default::default()
    }));
    let outlined = render(&gpu, &mut r, &w2, &frame(&w2));
    let red = count(&outlined, |p| p[0] > 150 && p[1] < 60);
    assert!(red > 40, "outline pixels: {red}");

    // An underlay (shadow) adds grey pixels down-right of the letters.
    let w3 = world(text(TextMaterial {
        underlay: true,
        underlay_color: Color::new(0.0, 0.0, 1.0, 1.0),
        underlay_offset: Vector2::new(1.0, -1.0),
        underlay_softness: 0.0,
        ..Default::default()
    }));
    let shadowed = render(&gpu, &mut r, &w3, &frame(&w3));
    if let Ok(dir) = std::env::var("OXIDE_DUMP_TEXT") {
        for (name, img) in [
            ("plain", &plain),
            ("outline", &outlined),
            ("underlay", &shadowed),
        ] {
            std::fs::write(format!("{dir}/{name}.png"), img.to_png().unwrap()).unwrap();
        }
    }
    let blue = count(&shadowed, |p| p[2] > 150 && p[0] < 60);
    assert!(blue > 20, "underlay pixels: {blue}");
}
