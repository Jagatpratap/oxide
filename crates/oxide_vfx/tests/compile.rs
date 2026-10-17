//! VFX Graph compiler tests: generated programs validate as WGSL (every block in every
//! context it allows, every operator and option), systems are found by flow, and the
//! spawn blocks count like Unity's.

use oxide_math::{Vector2, Vector4};
use oxide_shadergraph::{GraphNode, GraphProperty, PropertyType};
use oxide_vfx::blocks::{self, Stage};
use oxide_vfx::*;

fn valid(c: &Compiled) {
    for s in &c.systems {
        if let Err(e) = oxide_render::vfx::validate_program(&s.program) {
            panic!("invalid program:\n{e}\n\n{}", s.program.hooks);
        }
    }
}

fn context(a: &VisualEffectAsset, kind: &str) -> u32 {
    a.graph.find(kind).expect(kind).id
}

#[test]
fn the_default_graph_compiles() {
    let a = VisualEffectAsset::default();
    let c = compile(&a, "Simple Loop");
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(c.systems.len(), 1);
    let s = &c.systems[0];
    assert_eq!(s.spawn.len(), 1);
    assert_eq!(s.program.capacity, 256);
    assert!(s.program.local_space);
    assert!(s.program.soft_dot);
    // Size over Life and Color over Life baked 64 samples each.
    assert_eq!(s.program.luts.len(), 128);
    valid(&c);
}

#[test]
fn every_block_validates_in_every_context_it_allows() {
    let mut checked = 0;
    for def in blocks::library() {
        for stage in def.stages {
            let mut a = VisualEffectAsset::default();
            let ctx = context(
                &a,
                match stage {
                    Stage::Spawn => SPAWN,
                    Stage::Initialize => INITIALIZE,
                    Stage::Update => UPDATE,
                    Stage::Output => OUTPUT,
                },
            );
            let id = a.add_block(ctx, def.kind);
            // Every option choice too.
            let mut variants = vec![Vec::new()];
            for o in def.options.iter().filter(|o| !o.choices.is_empty()) {
                for c in o.choices {
                    variants.push(vec![(o.name, *c)]);
                }
            }
            for v in variants {
                for (n, c) in &v {
                    a.block_mut(id).expect("block").set_option(n, c);
                }
                let c = compile(&a, def.kind);
                assert!(c.errors.is_empty(), "{} {v:?}: {:?}", def.kind, c.errors);
                for s in &c.systems {
                    if let Err(e) = oxide_render::vfx::validate_program(&s.program) {
                        panic!("{} in {stage:?} {v:?}:\n{e}", def.kind);
                    }
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 60, "only {checked} checked");
}

#[test]
fn every_operator_validates_wired_into_a_block() {
    let mut checked = 0;
    let kinds: Vec<&str> = vfx_operators()
        .iter()
        .map(|d| d.kind)
        .chain(
            oxide_shadergraph::library()
                .iter()
                .filter(|d| find_operator(d.kind).is_some())
                .map(|d| d.kind),
        )
        .filter(|k| *k != "Property")
        .collect();
    for kind in kinds {
        let mut a = VisualEffectAsset::default();
        let init = context(&a, INITIALIZE);
        let b = a.add_block(init, "Set Size");
        let node = GraphNode::new(0, kind, Vector2::ZERO);
        let id = a.graph.add(node.clone());
        let (_, outs) = ports(&a, &node);
        let Some(out) = outs.first() else { continue };
        a.graph.connect(id, &out.id, init, &port_id(b, "Size"));
        let c = compile(&a, kind);
        assert!(c.errors.is_empty(), "{kind}: {:?}", c.errors);
        for s in &c.systems {
            if let Err(e) = oxide_render::vfx::validate_program(&s.program) {
                panic!("{kind}:\n{e}");
            }
        }
        checked += 1;
    }
    assert!(checked > 80, "only {checked} operators checked");
}

#[test]
fn properties_feed_blocks_and_spawn() {
    let mut a = VisualEffectAsset::default();
    a.properties.push(GraphProperty {
        name: "Rate".into(),
        reference: "_Rate".into(),
        kind: PropertyType::Float,
        default: Vector4::splat(50.0),
        ..Default::default()
    });
    a.properties.push(GraphProperty {
        name: "Tint".into(),
        reference: "_Tint".into(),
        kind: PropertyType::Color,
        default: Vector4::new(1.0, 0.0, 0.0, 1.0),
        ..Default::default()
    });
    let spawn = context(&a, SPAWN);
    let rate_block = a.blocks_of(spawn).next().expect("rate").id;
    let rate = a
        .graph
        .add(GraphNode::new(0, "Property", Vector2::ZERO).with_option("Property", "_Rate"));
    a.graph
        .connect(rate, "Out", spawn, &port_id(rate_block, "Rate"));
    let init = context(&a, INITIALIZE);
    let color = a.add_block(init, "Set Color");
    let tint = a
        .graph
        .add(GraphNode::new(0, "Property", Vector2::ZERO).with_option("Property", "_Tint"));
    a.graph.connect(tint, "Out", init, &port_id(color, "Color"));
    let c = compile(&a, "props");
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    valid(&c);
    assert_eq!(c.properties.len(), 2);
    assert!(c.systems[0].program.hooks.contains("props[1]"));
    let sb = &c.systems[0].spawn[0];
    assert_eq!(sb.inputs["Rate"].property.as_deref(), Some("_Rate"));
    assert_eq!(sb.get("Rate", &|_| Some(7.0)), 7.0);
}

#[test]
fn systems_follow_flow_and_report_missing_parts() {
    let mut a = VisualEffectAsset::empty();
    let init = a.graph.add(GraphNode::new(0, INITIALIZE, Vector2::ZERO));
    let c = compile(&a, "orphan");
    assert!(
        c.errors
            .iter()
            .any(|(n, e)| *n == init && e.contains("nothing spawns"))
    );
    assert!(
        c.errors
            .iter()
            .any(|(n, e)| *n == init && e.contains("no Output"))
    );
    // Output straight after Initialize (no Update) is a valid system.
    let spawn = a.graph.add(GraphNode::new(0, SPAWN, Vector2::ZERO));
    let out = a
        .graph
        .add(GraphNode::new(0, OUTPUT, Vector2::ZERO).with_option("Blend Mode", "Opaque"));
    a.graph.connect(spawn, "Flow Out", init, "Flow In");
    a.graph.connect(init, "Flow Out", out, "Flow In");
    let c = compile(&a, "direct");
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(c.systems[0].output, Some(out));
    assert_eq!(c.systems[0].program.blend, oxide_render::VfxBlend::Opaque);
    valid(&c);
}

#[test]
fn get_attribute_is_not_allowed_in_spawn() {
    let mut a = VisualEffectAsset::default();
    let spawn = context(&a, SPAWN);
    let b = a.blocks_of(spawn).next().expect("rate").id;
    let ga = a
        .graph
        .add(GraphNode::new(0, "Get Attribute", Vector2::ZERO).with_option("Attribute", "age"));
    a.graph.connect(ga, "age", spawn, &port_id(b, "Rate"));
    let c = compile(&a, "bad");
    assert!(
        c.errors
            .iter()
            .any(|(_, e)| e.contains("values or properties")),
        "{:?}",
        c.errors
    );
}

#[test]
fn spawn_blocks_count_like_unity() {
    use oxide_scene::World;
    let mut w = World::new();
    let dir = std::env::temp_dir().join(format!("oxide-vfx-spawn-{}", std::process::id()));
    let mut assets = oxide_assets::AssetServer::new(&dir);
    register_assets(&mut assets);
    let mut a = VisualEffectAsset::empty();
    let spawn = a.graph.add(GraphNode::new(0, SPAWN, Vector2::ZERO));
    let init = a.graph.add(GraphNode::new(0, INITIALIZE, Vector2::ZERO));
    let out = a.graph.add(GraphNode::new(0, OUTPUT, Vector2::ZERO));
    a.graph.connect(spawn, "Flow Out", init, "Flow In");
    a.graph.connect(init, "Flow Out", out, "Flow In");
    let rate = a.add_block(spawn, "Constant Spawn Rate");
    a.block_mut(rate)
        .unwrap()
        .set_value("Rate", Vector4::splat(10.0));
    let burst = a.add_block(spawn, "Single Burst");
    a.block_mut(burst)
        .unwrap()
        .set_value("Count", Vector4::splat(100.0));
    a.block_mut(burst)
        .unwrap()
        .set_value("Delay", Vector4::splat(0.5));
    assets
        .data
        .insert("test:fx.vfx", "VisualEffectAsset", Box::new(a));
    w.resources.insert(assets);
    install(&mut w);
    let id = w.create("FX");
    w.add_component(id, VisualEffect::new("test:fx.vfx"));
    let mut total = 0u64;
    let mut frames = Vec::new();
    for _ in 0..30 {
        update(&mut w, 0.05);
        let mut draws = Vec::new();
        extract(&w, &mut draws);
        assert_eq!(draws.len(), 1);
        frames.push(draws[0].spawn);
        total += draws[0].spawn as u64;
    }
    // 1.5 s at 10/s = 15, plus one burst of 100 at 0.5 s.
    assert_eq!(total, 115, "{frames:?}");
    assert!(frames.contains(&101) || frames.contains(&100), "{frames:?}");
    // Stop: spawning ends.
    effect_mut(&mut w, id).unwrap().0.stop();
    update(&mut w, 0.5);
    let mut draws = Vec::new();
    extract(&w, &mut draws);
    assert_eq!(draws[0].spawn, 0);
    // Reinit asks the GPU to clear the particles.
    effect_mut(&mut w, id).unwrap().0.reinit();
    update(&mut w, 0.05);
    let mut draws = Vec::new();
    extract(&w, &mut draws);
    assert!(draws[0].reset);
}
