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

/// A world with a 2×1 sprite sheet (green | magenta), a Sprite Asset and presets.
fn sprite_world(t: TextMeshPro) -> World {
    let dir = std::env::temp_dir().join(format!("oxide-tmp-sprites-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Assets")).unwrap();
    let mut assets = AssetServer::new(&dir);
    oxide_text::register_assets(&mut assets);
    let mut px = Vec::new();
    for _y in 0..16 {
        for x in 0..32 {
            px.extend_from_slice(if x < 16 {
                &[0, 255, 0, 255]
            } else {
                &[255, 0, 255, 255]
            });
        }
    }
    assets.textures.insert(
        "Assets/Sheet.png",
        oxide_assets::Texture2D::from_rgba8("Sheet", 32, 16, px, true),
    );
    let sheet = oxide_text::SpriteAsset::from_grid(
        oxide_assets::Handle::from_path("Assets/Sheet.png"),
        32,
        16,
        2,
        1,
        "icon",
    );
    assets
        .save_data("Assets/Icons.spriteasset", Box::new(sheet))
        .unwrap();
    assets
        .save_data(
            "Assets/RedOutline.textmaterial",
            Box::new(oxide_text::TextMaterialPreset {
                material: TextMaterial {
                    outline_width: 0.4,
                    outline_color: Color::rgb(1.0, 0.0, 0.0),
                    ..Default::default()
                },
            }),
        )
        .unwrap();
    let mut w = World::new();
    w.resources.insert(assets);
    let id = w.create("Text");
    w.add_component(id, t);
    oxide_text::components::update(&mut w);
    w
}

#[test]
fn sprites_gradients_and_material_presets_render() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut r = SceneRenderer::new(&gpu);
    let base = TextMeshPro {
        font_size: 30.0,
        rect_size: Vector2::new(4.0, 2.0),
        horizontal_alignment: oxide_text::HorizontalAlignment::Center,
        vertical_alignment: oxide_text::VerticalAlignment::Middle,
        ..Default::default()
    };
    // `<sprite>` by index from the text's Sprite Asset, and by name from a named asset.
    let w = sprite_world(TextMeshPro {
        text: "<sprite=0><sprite=\"Icons\" name=\"icon_1\">".into(),
        sprite_asset: Some(oxide_assets::Handle::from_path("Assets/Icons.spriteasset")),
        ..base.clone()
    });
    let f = frame(&w);
    let img = render(&gpu, &mut r, &w, &f);
    let green = count(&img, |p| p[1] > 200 && p[0] < 60 && p[2] < 60);
    let magenta = count(&img, |p| p[0] > 200 && p[2] > 200 && p[1] < 60);
    assert!(
        green > 50 && magenta > 50,
        "sprites drawn: {green} green, {magenta} magenta"
    );

    // A vertical Color Gradient: red tops, blue bottoms on white letters.
    let w = sprite_world(TextMeshPro {
        text: "HI".into(),
        enable_vertex_gradient: true,
        color_gradient: oxide_text::ColorGradient {
            color_mode: oxide_text::ColorMode::VerticalGradient,
            top_left: Color::rgb(1.0, 0.0, 0.0),
            bottom_left: Color::rgb(0.0, 0.0, 1.0),
            ..Default::default()
        },
        ..base.clone()
    });
    let img = render(&gpu, &mut r, &w, &frame(&w));
    // Red minus blue over the lit pixels of the top and bottom lit rows.
    let lit_rows: Vec<u32> = (0..H)
        .filter(|&y| (0..W).any(|x| img.pixel(x, y).iter().take(3).any(|&c| c > 100)))
        .collect();
    assert!(lit_rows.len() > 6, "letters drawn");
    let balance = |y: u32| -> i32 {
        (0..W)
            .map(|x| img.pixel(x, y))
            .filter(|p| p.iter().take(3).any(|&c| c > 100))
            .map(|p| p[0] as i32 - p[2] as i32)
            .sum()
    };
    let (top, bottom) = (balance(lit_rows[1]), balance(lit_rows[lit_rows.len() - 2]));
    assert!(
        top > 0 && bottom < 0,
        "red at the top ({top}), blue at the bottom ({bottom})"
    );

    // A Material preset's red outline replaces the text's own material.
    let w = sprite_world(TextMeshPro {
        text: "HI".into(),
        material_preset: Some(oxide_assets::Handle::from_path(
            "Assets/RedOutline.textmaterial",
        )),
        ..base
    });
    let img = render(&gpu, &mut r, &w, &frame(&w));
    let red = count(&img, |p| p[0] > 150 && p[1] < 60);
    assert!(red > 40, "preset outline pixels: {red}");
}
