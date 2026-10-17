//! Shader Graph compiler tests: every node's WGSL validates in the real Lit and shadow
//! shaders, in both stages, and the graph features (properties, Sub Graphs, dynamic
//! vectors, loops, Custom Function, previews) generate the right code.

use oxide_math::{Vector2, Vector4};
use oxide_shadergraph::*;

fn none(_: &str) -> Option<ShaderSubGraph> {
    None
}

fn check(g: &ShaderGraph, subs: SubGraphLookup) -> Compiled {
    let c = compile(g, subs, Target::Material);
    if let Err(e) = oxide_render::validate_surface_hooks(&c.hooks) {
        panic!("invalid WGSL:\n{e}\n\n{}", c.hooks);
    }
    c
}

type SubGraphLookup<'a> = &'a dyn Fn(&str) -> Option<ShaderSubGraph>;

fn stack(g: &ShaderGraph, kind: &str) -> u32 {
    g.graph.find(kind).expect("stack").id
}

#[test]
fn default_graph_compiles() {
    let g = ShaderGraph::default();
    let c = check(&g, &none);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert!(
        !c.shadow,
        "an untouched graph uses the built-in shadow pass"
    );
    // Unity's default Base Color (0.5 grey, linearised).
    assert!(
        c.hooks.contains("s.base_color = sg_k3(vec3f(0.21"),
        "{}",
        c.hooks
    );
}

/// Every node, with default inputs, connected to Base Color (fragment) and, unless it is
/// fragment-only, to Vertex Position.
#[test]
fn every_node_generates_valid_wgsl_in_both_stages() {
    let mut checked = 0;
    for def in library() {
        if matches!(
            def.kind,
            "Vertex" | "Fragment" | "SubGraphOutput" | "Property" | "Sub Graph" | "Keyword"
        ) {
            continue;
        }
        let mut g = ShaderGraph::default();
        let node = GraphNode::new(0, def.kind, Vector2::ZERO);
        let id = g.graph.add(node.clone());
        let cx = PortCx {
            properties: &[],
            keywords: &[],
            sub_outputs: &[],
            subgraphs: &none,
        };
        let outs = ports(&node, &cx).expect("ports").1;
        let Some(out) = outs.iter().find(|s| s.ty != Ty::Tex) else {
            continue;
        };
        let frag = stack(&g, "Fragment");
        g.graph.connect(id, &out.id, frag, "Base Color");
        if !def.fragment_only {
            let vert = stack(&g, "Vertex");
            g.graph.connect(id, &out.id, vert, "Position");
        }
        let c = compile(&g, &none, Target::Material);
        if let Err(e) = oxide_render::validate_surface_hooks(&c.hooks) {
            panic!("{}: invalid WGSL:\n{e}", def.kind);
        }
        assert!(c.errors.is_empty(), "{}: {:?}", def.kind, c.errors);
        checked += 1;
    }
    assert!(checked > 100, "only {checked} nodes checked");
}

/// Options change the code: every dropdown choice of every node validates.
#[test]
fn every_node_option_validates() {
    for def in library() {
        for o in def.options.iter().filter(|o| !o.choices.is_empty()) {
            for choice in o.choices {
                let mut g = ShaderGraph::default();
                let node = GraphNode::new(0, def.kind, Vector2::ZERO).with_option(o.name, choice);
                let id = g.graph.add(node.clone());
                let cx = PortCx {
                    properties: &[],
                    keywords: &[],
                    sub_outputs: &[],
                    subgraphs: &none,
                };
                let outs = ports(&node, &cx).expect("ports").1;
                let Some(out) = outs.iter().find(|s| s.ty != Ty::Tex) else {
                    continue;
                };
                let frag = stack(&g, "Fragment");
                g.graph.connect(id, &out.id, frag, "Emission");
                let c = compile(&g, &none, Target::Material);
                if let Err(e) = oxide_render::validate_surface_hooks(&c.hooks) {
                    panic!("{} {}={choice}: invalid WGSL:\n{e}", def.kind, o.name);
                }
            }
        }
    }
}

#[test]
fn properties_get_uniform_and_texture_slots() {
    let mut g = ShaderGraph::default();
    g.properties.push(GraphProperty {
        name: "Tint".into(),
        reference: "_Tint".into(),
        kind: PropertyType::Color,
        default: Vector4::new(1.0, 0.0, 0.0, 1.0),
        ..Default::default()
    });
    g.properties.push(GraphProperty {
        name: "Main Tex".into(),
        reference: "_MainTex".into(),
        kind: PropertyType::Texture2D,
        ..Default::default()
    });
    g.properties.push(GraphProperty {
        name: "Strength".into(),
        reference: "_Strength".into(),
        kind: PropertyType::Float,
        default: Vector4::splat(0.25),
        ..Default::default()
    });
    let tint = g
        .graph
        .add(GraphNode::new(0, "Property", Vector2::ZERO).with_option("Property", "_Tint"));
    let tex = g
        .graph
        .add(GraphNode::new(0, "Property", Vector2::ZERO).with_option("Property", "_MainTex"));
    let sample = g
        .graph
        .add(GraphNode::new(0, "Sample Texture 2D", Vector2::ZERO));
    let mul = g.graph.add(GraphNode::new(0, "Multiply", Vector2::ZERO));
    g.graph.connect(tex, "Out", sample, "Texture");
    g.graph.connect(tint, "Out", mul, "A");
    g.graph.connect(sample, "RGBA", mul, "B");
    let frag = stack(&g, "Fragment");
    g.graph.connect(mul, "Out", frag, "Base Color");
    let c = check(&g, &none);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    let p = |n: &str| c.properties.iter().find(|p| p.name == n).expect(n).clone();
    assert_eq!((p("_Tint").kind.as_str(), p("_Tint").slot), ("Color", 0));
    assert_eq!(
        (p("_MainTex").kind.as_str(), p("_MainTex").slot),
        ("Texture2D", 0)
    );
    assert_eq!(p("_Strength").slot, 1);
    assert_eq!(p("_Strength").default[0], 0.25);
    assert!(c.hooks.contains("graph_props[0]"));
    assert!(c.hooks.contains("textureSample(base_map"));
    // Color (vec4) x Color (vec4) stays a vec4, truncated into Base Color.
    assert!(
        c.hooks.contains(&format!("let n{mul}_0: vec4f")),
        "{}",
        c.hooks
    );
}

#[test]
fn dynamic_vectors_take_the_narrowest_connected_vector() {
    let mut g = ShaderGraph::default();
    let pos = g.graph.add(GraphNode::new(0, "Position", Vector2::ZERO));
    let t = g.graph.add(GraphNode::new(0, "Time", Vector2::ZERO));
    let mul = g.graph.add(GraphNode::new(0, "Multiply", Vector2::ZERO));
    g.graph.connect(pos, "Out", mul, "A");
    g.graph.connect(t, "Time", mul, "B");
    let frag = stack(&g, "Fragment");
    g.graph.connect(mul, "Out", frag, "Emission");
    let c = check(&g, &none);
    // Vector3 x Float = Vector3 (the float broadcasts).
    assert!(
        c.hooks.contains(&format!("let n{mul}_0: vec3f")),
        "{}",
        c.hooks
    );
}

#[test]
fn vertex_displacement_uses_the_graph_shadow_pass() {
    let mut g = ShaderGraph::default();
    let pos = g
        .graph
        .add(GraphNode::new(0, "Position", Vector2::ZERO).with_option("Space", "Object"));
    let add = g.graph.add(
        GraphNode::new(0, "Add", Vector2::ZERO).with_value("B", Vector4::new(0.0, 1.0, 0.0, 0.0)),
    );
    g.graph.connect(pos, "Out", add, "A");
    let vert = stack(&g, "Vertex");
    g.graph.connect(add, "Out", vert, "Position");
    let c = check(&g, &none);
    assert!(c.shadow);
    assert!(c.hooks.contains("m.position = n"), "{}", c.hooks);
}

#[test]
fn loops_and_fragment_only_nodes_in_the_vertex_stage_are_errors() {
    let mut g = ShaderGraph::default();
    let a = g.graph.add(GraphNode::new(0, "Add", Vector2::ZERO));
    let b = g.graph.add(GraphNode::new(0, "Add", Vector2::ZERO));
    g.graph.edges.push(GraphEdge {
        from: a,
        output: "Out".into(),
        to: b,
        input: "A".into(),
    });
    g.graph.edges.push(GraphEdge {
        from: b,
        output: "Out".into(),
        to: a,
        input: "A".into(),
    });
    assert!(g.graph.creates_cycle(a, b));
    let frag = stack(&g, "Fragment");
    g.graph.connect(a, "Out", frag, "Metallic");
    let ddx = g.graph.add(GraphNode::new(0, "DDX", Vector2::ZERO));
    let vert = stack(&g, "Vertex");
    g.graph.connect(ddx, "Out", vert, "Position");
    let c = check(&g, &none);
    assert!(
        c.errors.iter().any(|(_, e)| e.contains("loop")),
        "{:?}",
        c.errors
    );
    assert!(
        c.errors
            .iter()
            .any(|(n, e)| *n == ddx && e.contains("Fragment stage")),
        "{:?}",
        c.errors
    );
}

fn sub_graph_multiply() -> ShaderSubGraph {
    // Out = In * Scale, with a texture default inside.
    let mut s = ShaderSubGraph::default();
    s.properties.push(GraphProperty {
        name: "In".into(),
        reference: "_In".into(),
        kind: PropertyType::Vector3,
        default: Vector4::splat(1.0),
        ..Default::default()
    });
    s.properties.push(GraphProperty {
        name: "Scale".into(),
        reference: "_Scale".into(),
        kind: PropertyType::Float,
        default: Vector4::splat(3.0),
        ..Default::default()
    });
    s.outputs = vec![SubGraphOutput {
        name: "Out".into(),
        kind: PropertyType::Vector3,
    }];
    let a = s
        .graph
        .add(GraphNode::new(0, "Property", Vector2::ZERO).with_option("Property", "_In"));
    let b = s
        .graph
        .add(GraphNode::new(0, "Property", Vector2::ZERO).with_option("Property", "_Scale"));
    let m = s.graph.add(GraphNode::new(0, "Multiply", Vector2::ZERO));
    s.graph.connect(a, "Out", m, "A");
    s.graph.connect(b, "Out", m, "B");
    let out = s.graph.find("SubGraphOutput").expect("output").id;
    s.graph.connect(m, "Out", out, "Out");
    s
}

#[test]
fn sub_graphs_inline_with_bound_inputs_and_defaults() {
    let sub = sub_graph_multiply();
    // An outer Sub Graph uses the inner one twice (nesting).
    let mut outer = ShaderSubGraph {
        outputs: vec![SubGraphOutput {
            name: "Out".into(),
            kind: PropertyType::Vector3,
        }],
        ..Default::default()
    };
    let i1 = outer.graph.add(
        GraphNode::new(0, "Sub Graph", Vector2::ZERO)
            .with_option("Sub Graph", "Assets/Mul.shadersubgraph"),
    );
    let i2 = outer.graph.add(
        GraphNode::new(0, "Sub Graph", Vector2::ZERO)
            .with_option("Sub Graph", "Assets/Mul.shadersubgraph"),
    );
    outer.graph.connect(i1, "Out", i2, "_In");
    let oid = outer.graph.find("SubGraphOutput").expect("out").id;
    outer.graph.connect(i2, "Out", oid, "Out");

    let lookup = |p: &str| match p {
        "Assets/Mul.shadersubgraph" => Some(sub.clone()),
        "Assets/Outer.shadersubgraph" => Some(outer.clone()),
        _ => None,
    };
    let mut g = ShaderGraph::default();
    let col = g.graph.add(
        GraphNode::new(0, "Vector 3", Vector2::ZERO)
            .with_value("X", Vector4::splat(0.5))
            .with_value("Y", Vector4::splat(0.5))
            .with_value("Z", Vector4::splat(0.5)),
    );
    let node = g.graph.add(
        GraphNode::new(0, "Sub Graph", Vector2::ZERO)
            .with_option("Sub Graph", "Assets/Mul.shadersubgraph"),
    );
    g.graph.connect(col, "Out", node, "_In");
    let nested = g.graph.add(
        GraphNode::new(0, "Sub Graph", Vector2::ZERO)
            .with_option("Sub Graph", "Assets/Outer.shadersubgraph"),
    );
    let add = g.graph.add(GraphNode::new(0, "Add", Vector2::ZERO));
    g.graph.connect(node, "Out", add, "A");
    g.graph.connect(nested, "Out", add, "B");
    let frag = stack(&g, "Fragment");
    g.graph.connect(add, "Out", frag, "Base Color");
    let c = check(&g, &lookup);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    // The unconnected Scale uses the Sub Graph's default (3).
    assert!(c.hooks.contains("3.0"), "{}", c.hooks);
    assert_eq!(
        sub_graph_paths(&g.graph, &lookup),
        vec![
            "Assets/Outer.shadersubgraph".to_string(),
            "Assets/Mul.shadersubgraph".to_string()
        ]
    );
    let mut deps = c.dependencies.clone();
    deps.sort();
    assert_eq!(
        deps,
        vec!["Assets/Mul.shadersubgraph", "Assets/Outer.shadersubgraph"]
    );
}

#[test]
fn recursive_sub_graphs_are_an_error() {
    let mut s = ShaderSubGraph::default();
    let n = s.graph.add(
        GraphNode::new(0, "Sub Graph", Vector2::ZERO)
            .with_option("Sub Graph", "Assets/Self.shadersubgraph"),
    );
    let out = s.graph.find("SubGraphOutput").expect("out").id;
    s.graph.connect(n, "Out", out, "Out");
    let lookup = |p: &str| (p == "Assets/Self.shadersubgraph").then(|| s.clone());
    let mut g = ShaderGraph::default();
    let n = g.graph.add(
        GraphNode::new(0, "Sub Graph", Vector2::ZERO)
            .with_option("Sub Graph", "Assets/Self.shadersubgraph"),
    );
    let frag = stack(&g, "Fragment");
    g.graph.connect(n, "Out", frag, "Emission");
    let c = check(&g, &lookup);
    assert!(
        c.errors
            .iter()
            .any(|(id, e)| *id == n && e.contains("uses itself")),
        "{:?}",
        c.errors
    );
}

#[test]
fn custom_function_and_keywords() {
    let mut g = ShaderGraph::default();
    g.keywords.push(GraphKeyword {
        name: "Use Red".into(),
        reference: "_USE_RED".into(),
        default: true,
        exposed: true,
        definition: KeywordDefinition::DynamicBranch,
        ..Default::default()
    });
    let cf = g.graph.add(
        GraphNode::new(0, "Custom Function", Vector2::ZERO)
            .with_option("Inputs", "A:Float; B:Vector 3")
            .with_option("Outputs", "Out:Vector 3; Len:Float")
            .with_option("Body", "Out = B * A; Len = length(Out);"),
    );
    let kw = g
        .graph
        .add(GraphNode::new(0, "Keyword", Vector2::ZERO).with_option("Keyword", "_USE_RED"));
    g.graph.connect(cf, "Out", kw, "On");
    let frag = stack(&g, "Fragment");
    g.graph.connect(kw, "Out", frag, "Base Color");
    g.graph.connect(cf, "Len", frag, "Smoothness");
    let c = check(&g, &none);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    let k = c
        .properties
        .iter()
        .find(|p| p.name == "_USE_RED")
        .expect("keyword slot");
    assert_eq!((k.kind.as_str(), k.default[0]), ("Keyword", 1.0));
    assert!(c.hooks.contains("fn sg_cf_n"), "{}", c.hooks);
}

#[test]
fn preview_target_switches_between_every_node() {
    let mut g = ShaderGraph::default();
    let uv = g.graph.add(GraphNode::new(0, "UV", Vector2::ZERO));
    let noise = g
        .graph
        .add(GraphNode::new(0, "Gradient Noise", Vector2::ZERO));
    g.graph.connect(uv, "Out", noise, "UV");
    let tex = g
        .graph
        .add(GraphNode::new(0, "Texture 2D Asset", Vector2::ZERO));
    let c = compile(&g, &none, Target::Preview);
    oxide_render::validate_surface_hooks(&c.hooks).expect("preview WGSL");
    assert_eq!(c.preview_nodes, vec![uv, noise, tex]);
    assert!(c.unlit);
    assert!(
        c.properties
            .iter()
            .any(|p| p.name == PREVIEW_PROPERTY && p.slot == 31)
    );
}

#[test]
fn graphs_round_trip_through_reflection() {
    use oxide_core::reflect::Reflect;
    let mut g = ShaderGraph::default();
    g.properties.push(GraphProperty::default());
    let n = g.graph.add(
        GraphNode::new(0, "Multiply", Vector2::new(3.0, 4.0)).with_value("B", Vector4::splat(5.0)),
    );
    let frag = stack(&g, "Fragment");
    g.graph.connect(n, "Out", frag, "Alpha");
    let mut back = ShaderGraph::default();
    for (k, v) in g.fields() {
        back.set_field(k, v).expect("field");
    }
    assert_eq!(back, g);
}

#[test]
fn variant_keywords_become_override_constants() {
    let mut g = ShaderGraph::default();
    g.keywords.push(GraphKeyword {
        name: "Mode".into(),
        reference: "_MODE".into(),
        exposed: true,
        keyword_type: KeywordType::Enum,
        entries: vec!["Red".into(), "Green".into(), "Blue".into()],
        default_entry: 1,
        ..Default::default()
    });
    let kw = g
        .graph
        .add(GraphNode::new(0, "Keyword", Vector2::ZERO).with_option("Keyword", "_MODE"));
    let frag = stack(&g, "Fragment");
    g.graph.connect(kw, "Out", frag, "Base Color");
    let c = check(&g, &none);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    // A variant constant, not a material value.
    assert!(
        c.hooks.contains("override kw__MODE: u32 = 1u;"),
        "{}",
        c.hooks
    );
    assert!(!c.properties.iter().any(|p| p.name == "_MODE"));
    let k = &c.keywords[0];
    assert_eq!(
        (k.reference.as_str(), k.default, k.global),
        ("_MODE", 1, false)
    );
    // Unity names: REF_ENTRY.
    assert_eq!(k.value(&|n| n == "_MODE_BLUE"), 2);
    assert_eq!(k.value(&|_| false), 1);
    // Materials start with the default entry enabled.
    let mut m = oxide_assets::Material::default();
    g.apply_default_keywords(&mut m);
    assert_eq!(m.keywords, vec!["_MODE_GREEN".to_owned()]);
    m.set_keyword_state("_MODE", &g.keywords[0].entries, 0);
    assert_eq!(m.keywords, vec!["_MODE_RED".to_owned()]);
}
