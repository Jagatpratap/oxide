//! GPU test: materials draw with a compiled Shader Graph (property values, material
//! overrides, vertex displacement, the magenta error shader). Skipped without a GPU.

use oxide_assets::{AssetId, AssetServer, Handle, Material, PrimitiveType};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, Vector2, Vector3, Vector4};
use oxide_render::*;
use oxide_shadergraph::*;

const SIZE: u32 = 64;
const GRAPH: &str = "test:tint.shadergraph";

fn frame(material: AssetId) -> RenderFrame {
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
            material,
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

/// Unlit: Base Color = the `_Tint` property (default green); optional vertex lift.
fn graph(lift: bool, broken: bool) -> ShaderGraph {
    let mut g = ShaderGraph::default();
    g.settings.material = GraphMaterial::Unlit;
    g.properties.push(GraphProperty {
        name: "Tint".into(),
        reference: "_Tint".into(),
        kind: PropertyType::Color,
        default: Vector4::new(0.0, 1.0, 0.0, 1.0),
        ..Default::default()
    });
    let tint = g
        .graph
        .add(GraphNode::new(0, "Property", Vector2::ZERO).with_option("Property", "_Tint"));
    let frag = g.graph.find("Fragment").unwrap().id;
    g.graph.connect(tint, "Out", frag, "Base Color");
    if lift {
        let pos = g
            .graph
            .add(GraphNode::new(0, "Position", Vector2::ZERO).with_option("Space", "Object"));
        let add = g.graph.add(
            GraphNode::new(0, "Add", Vector2::ZERO)
                .with_value("B", Vector4::new(0.0, 1.5, 0.0, 0.0)),
        );
        g.graph.connect(pos, "Out", add, "A");
        let vert = g.graph.find("Vertex").unwrap().id;
        g.graph.connect(add, "Out", vert, "Position");
    }
    if broken {
        let cf = g.graph.add(
            GraphNode::new(0, "Custom Function", Vector2::ZERO)
                .with_option("Body", "Out = this_is_not_wgsl;"),
        );
        g.graph.connect(cf, "Out", frag, "Emission");
    }
    g
}

#[test]
fn materials_draw_with_compiled_shader_graphs() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut assets = AssetServer::new(".");
    register_assets(&mut assets);
    let put = |assets: &mut AssetServer, g: ShaderGraph| {
        assets.data.insert(GRAPH, "ShaderGraph", Box::new(g));
        assert!(compile_asset(assets, GRAPH), "recompiles after a change");
    };
    put(&mut assets, graph(false, false));
    assert!(
        !compile_asset(&mut assets, GRAPH),
        "no recompile without a change"
    );
    let mut m = Material::lit("graph", Color::WHITE);
    m.shader_graph = Some(Handle::from_path(GRAPH.to_owned()));
    let mat = assets.materials.insert("test:graph.mat", m.clone()).id();
    let mut r = SceneRenderer::new(&gpu);
    let render = |r: &mut SceneRenderer, assets: &AssetServer| {
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
        let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
            r.render(&gpu, assets, &frame(mat), view, fmt, SIZE, SIZE);
        })
        .unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("GPU validation error: {e}");
        }
        img.pixel(SIZE / 2, SIZE / 2)
    };

    let c = render(&mut r, &assets);
    eprintln!("default tint {c:?}");
    assert!(
        c[1] > 240 && c[0] < 15 && c[2] < 15,
        "the property default (green): {c:?}"
    );

    // Unity `Material.SetColor("_Tint", ...)`.
    m.set_vector("_Tint", Vector4::new(1.0, 0.0, 0.0, 1.0));
    assets.materials.insert("test:graph.mat", m.clone());
    let c = render(&mut r, &assets);
    eprintln!("material tint {c:?}");
    assert!(c[0] > 240 && c[1] < 15, "the material's value (red): {c:?}");

    // Editing the graph recompiles it: the vertex stage lifts the cube out of the centre.
    put(&mut assets, graph(true, false));
    let c = render(&mut r, &assets);
    eprintln!("lifted {c:?}");
    assert!(
        c[0] < 10 && c[1] < 10 && c[2] < 10,
        "background where the cube was: {c:?}"
    );

    // Code that doesn't compile draws Unity's magenta error shader.
    put(&mut assets, graph(false, true));
    let c = render(&mut r, &assets);
    eprintln!("broken {c:?}");
    assert!(c[0] > 240 && c[2] > 240 && c[1] < 15, "magenta: {c:?}");
}

/// Node previews: each preview index shows its node (a Remap of a Sine of Time here).
#[test]
fn preview_target_renders_each_node() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let mut g = ShaderGraph::default();
    let t = g.graph.add(GraphNode::new(0, "Time", Vector2::ZERO));
    let s = g.graph.add(GraphNode::new(0, "Sine", Vector2::ZERO));
    let r = g.graph.add(GraphNode::new(0, "Remap", Vector2::ZERO));
    g.graph.connect(t, "Time", s, "In");
    g.graph.connect(s, "Out", r, "In");
    let c = compile(&g, &|_| None, Target::Preview);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    let mut assets = AssetServer::new(".");
    assets.shaders.insert(
        "test:preview",
        oxide_assets::Shader {
            name: "preview".into(),
            source: c.hooks.clone(),
            surface: Some(oxide_assets::SurfaceShader {
                unlit: true,
                properties: c.properties.clone(),
                generation: 1,
                stamp: 1,
                ..Default::default()
            }),
        },
    );
    let mut r = SceneRenderer::new(&gpu);
    for (k, id) in c.preview_nodes.iter().enumerate() {
        let mut m = Material::lit("p", Color::WHITE);
        m.shader_graph = Some(Handle::from_path("test:preview".to_owned()));
        m.set_float(PREVIEW_PROPERTY, k as f32);
        let mat = assets.materials.insert(format!("test:p{k}.mat"), m).id();
        let mut f = frame(mat);
        f.time = 0.3;
        let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
        let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
            r.render(&gpu, &assets, &f, view, fmt, SIZE, SIZE);
        })
        .unwrap();
        let p = img.pixel(SIZE / 2, SIZE / 2);
        eprintln!("node {id} preview {p:?}");
        assert_eq!(p[3], 255, "opaque");
    }
    // Remap(sin(0.3)) = (0.2955 + 1) / 2 ≈ 0.648 linear.
    eprintln!(
        "{}",
        c.hooks
            .lines()
            .filter(|l| l.contains("let n"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Shader keywords: an Enum keyword picks the color per material (each its own variant), and
/// a Global keyword (`Shader.EnableKeyword`) turns every material black.
#[test]
fn keyword_variants_pick_per_material_and_global() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    const KW_GRAPH: &str = "test:keywords.shadergraph";
    let mut assets = AssetServer::new(".");
    register_assets(&mut assets);
    let mut g = ShaderGraph::default();
    g.settings.material = GraphMaterial::Unlit;
    g.keywords.push(GraphKeyword {
        name: "Mode".into(),
        reference: "_MODE".into(),
        exposed: true,
        keyword_type: KeywordType::Enum,
        entries: vec!["Red".into(), "Green".into(), "Blue".into()],
        default_entry: 1,
        ..Default::default()
    });
    g.keywords.push(GraphKeyword {
        name: "Dark".into(),
        reference: "_DARK".into(),
        scope: KeywordScope::Global,
        ..Default::default()
    });
    let mode = g
        .graph
        .add(GraphNode::new(0, "Keyword", Vector2::ZERO).with_option("Keyword", "_MODE"));
    let dark = g
        .graph
        .add(GraphNode::new(0, "Keyword", Vector2::ZERO).with_option("Keyword", "_DARK"));
    for (port, node, c) in [
        ("Red", mode, Vector4::new(1.0, 0.0, 0.0, 1.0)),
        ("Green", mode, Vector4::new(0.0, 1.0, 0.0, 1.0)),
        ("Blue", mode, Vector4::new(0.0, 0.0, 1.0, 1.0)),
        ("On", dark, Vector4::new(0.0, 0.0, 0.0, 1.0)),
    ] {
        let col = g
            .graph
            .add(GraphNode::new(0, "Color", Vector2::ZERO).with_value("Color", c));
        g.graph.connect(col, "Out", node, port);
    }
    g.graph.connect(mode, "Out", dark, "Off");
    let frag = g.graph.find("Fragment").unwrap().id;
    g.graph.connect(dark, "Out", frag, "Base Color");
    assets
        .data
        .insert(KW_GRAPH, "ShaderGraph", Box::new(g.clone()));
    assert!(compile_asset(&mut assets, KW_GRAPH));

    let mut m = Material::lit("kw", Color::WHITE);
    m.shader_graph = Some(Handle::from_path(KW_GRAPH.to_owned()));
    g.apply_default_keywords(&mut m);
    let mut blue = m.clone();
    blue.set_keyword_state("_MODE", &g.keywords[0].entries, 2);
    let green_id = assets.materials.insert("test:kw_green.mat", m).id();
    let blue_id = assets.materials.insert("test:kw_blue.mat", blue).id();
    let mut r = SceneRenderer::new(&gpu);
    let render = |r: &mut SceneRenderer, mat: AssetId, global: &[&str]| {
        let mut f = frame(mat);
        f.global_keywords = global.iter().map(|s| s.to_string()).collect();
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
        let img = capture_frame(&gpu, fmt, SIZE, SIZE, |view, _| {
            r.render(&gpu, &assets, &f, view, fmt, SIZE, SIZE);
        })
        .unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("GPU validation error: {e}");
        }
        img.pixel(SIZE / 2, SIZE / 2)
    };
    let c = render(&mut r, green_id, &[]);
    assert!(
        c[1] > 240 && c[0] < 15 && c[2] < 15,
        "default entry (green): {c:?}"
    );
    let c = render(&mut r, blue_id, &[]);
    assert!(
        c[2] > 240 && c[0] < 15 && c[1] < 15,
        "the material's entry (blue): {c:?}"
    );
    // Global keyword on: every material's variant changes.
    for id in [green_id, blue_id] {
        let c = render(&mut r, id, &["_DARK"]);
        assert!(
            c[0] < 10 && c[1] < 10 && c[2] < 10,
            "global keyword (black): {c:?}"
        );
    }
}
