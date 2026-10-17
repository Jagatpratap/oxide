//! Oxide Shader Graph: Unity's node-based shader authoring for the Lit pipeline.
//!
//! A `.shadergraph` is a data asset ([`ShaderGraph`]); [`sync`] compiles every loaded graph
//! (again whenever it or a Sub Graph it uses changes) into a shader stored in
//! `AssetServer::shaders` under the graph's own path. Materials point at the graph with
//! `Material::shader_graph`, and the renderer draws them with the compiled hooks.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/index.html>

pub mod compile;
pub mod graph;
pub mod nodes;

pub use compile::{
    Compiled, ERROR_HOOKS, PREVIEW_PROPERTY, PortCx, Target, compile, options, ports, property_ty,
    sub_graph_as_graph, sub_graph_paths,
};
pub use graph::*;
pub use nodes::{NodeDef, Slot, Ty, library};

use oxide_assets::{AssetServer, AssetTypeInfo, Handle, Shader, SurfaceShader};
use std::hash::{Hash, Hasher};

/// Registers `.shadergraph` and `.shadersubgraph`.
pub fn register_assets(assets: &mut AssetServer) {
    assets.register_data_asset::<ShaderGraph>(
        AssetTypeInfo::new("ShaderGraph", &["shadergraph"])
            .menu("Shader Graph/URP/Lit Shader Graph", "New Shader Graph", 33)
            .color(150, 220, 160),
    );
    assets.register_data_asset::<ShaderSubGraph>(
        AssetTypeInfo::new("ShaderSubGraph", &["shadersubgraph"])
            .menu("Shader Graph/Sub Graph", "New Shader Sub Graph", 34)
            .color(120, 190, 140),
    );
}

/// A Sub Graph from the asset server (loaded already).
pub fn sub_graph(assets: &AssetServer, path: &str) -> Option<ShaderSubGraph> {
    assets
        .data
        .get(&Handle::<ShaderSubGraph>::from_path(path.to_owned()))
        .cloned()
}

/// Loads every Sub Graph `graph` uses (and theirs).
pub fn load_sub_graphs(assets: &mut AssetServer, graph: &GraphData) {
    loop {
        let paths = sub_graph_paths(graph, &|p| sub_graph(assets, p));
        let missing: Vec<String> = paths
            .into_iter()
            .filter(|p| !assets.data.contains(p))
            .collect();
        let mut loaded = false;
        for p in missing {
            match assets.ensure_loaded(&p) {
                Ok(()) => loaded = true,
                Err(e) => oxide_core::debug_log_warning!("{e}"),
            }
        }
        if !loaded {
            return;
        }
    }
}

fn stamp(assets: &AssetServer, path: &str, deps: &[String]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    assets.data.version(path).hash(&mut h);
    for d in deps {
        d.hash(&mut h);
        assets.data.version(d).hash(&mut h);
    }
    h.finish().max(1)
}

/// Compiles the graph at `path` into `assets.shaders[path]` if it (or a Sub Graph it uses)
/// changed since the last compile. Returns whether it compiled.
pub fn compile_asset(assets: &mut AssetServer, path: &str) -> bool {
    let old = assets
        .shaders
        .handle_of(path)
        .and_then(|h| assets.shaders.get(&h))
        .and_then(|s| s.surface.as_ref())
        .map(|s| (s.stamp, s.dependencies.clone()));
    if let Some((st, deps)) = &old
        && *st == stamp(assets, path, deps)
    {
        return false;
    }
    let Some(graph) = assets
        .data
        .get(&Handle::<ShaderGraph>::from_path(path.to_owned()))
        .cloned()
    else {
        return false;
    };
    load_sub_graphs(assets, &graph.graph);
    let c = compile(&graph, &|p| sub_graph(assets, p), Target::Material);
    let name = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Shader Graph")
        .to_owned();
    for (node, e) in &c.errors {
        oxide_core::debug_log_error!("Shader Graph {name} (node {node}): {e}");
    }
    let st = stamp(assets, path, &c.dependencies);
    let surface = SurfaceShader {
        unlit: c.unlit,
        transparent: c.transparent,
        alpha_clip: c.alpha_clip,
        double_sided: c.double_sided,
        cast_shadows: c.cast_shadows,
        properties: c.properties,
        shadow_source: c.shadow.then(|| c.hooks.clone()),
        generation: st,
        stamp: st,
        dependencies: c.dependencies,
        errors: c.errors,
        keywords: c.keywords,
    };
    assets.shaders.insert(
        path.to_owned(),
        Shader {
            name,
            source: c.hooks,
            surface: Some(surface),
        },
    );
    true
}

/// Compiles every loaded Shader Graph that changed (call once per frame before rendering).
pub fn sync(assets: &mut AssetServer) {
    let graphs: Vec<String> = assets
        .data
        .iter()
        .filter(|(_, t)| *t == "ShaderGraph")
        .map(|(p, _)| p.to_owned())
        .collect();
    for p in graphs {
        compile_asset(assets, &p);
    }
}
