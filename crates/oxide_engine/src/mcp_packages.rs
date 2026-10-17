//! MCP tool for the Package Manager.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool};
use oxide_assets::AssetServer;
use oxide_assets::packages::{self, AddRequest, NetFetcher, PackageInfo, PackageSource};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn package_json(p: &PackageInfo) -> Json {
    let mut v = json!({
        "name": p.name, "version": p.version, "display_name": p.display_name,
        "description": p.description, "author": p.author, "source": p.source.label(),
        "direct": p.direct, "root": p.root.display().to_string(),
        "dependencies": p.dependencies,
        "samples": p.samples.iter().map(|s| json!({
            "display_name": s.display_name, "description": s.description, "path": s.path
        })).collect::<Vec<_>>(),
        "rust_crate": p.rust_crate.as_ref().map(|c| c.0.clone()),
    });
    if let PackageSource::Git { url, revision } = &p.source {
        v["url"] = json!(url);
        v["revision"] = json!(revision);
    }
    v
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "packages",
            "Package Manager",
            "Lists the project's packages (Packages/manifest.json and embedded Packages/<folder>s, resolved \
             with their dependencies): name, version, source (Embedded, Local, Git, Registry), samples, Rust \
             crate. `action`: add (`path` folder with a package.json | `git` URL with optional #revision | \
             `name` and optional `version` from the manifest's registry), remove (`name`), import_sample \
             (`name`, `sample` display name: copied into Assets/Samples), resolve. Package crates are added \
             to the Scripts crate's dependencies.",
            schema(
                json!({
                    "action": { "type": "string", "enum": ["list", "add", "remove", "import_sample", "resolve"] },
                    "path": { "type": "string" }, "git": { "type": "string" },
                    "name": { "type": "string" }, "version": { "type": "string" },
                    "sample": { "type": "string" }
                }),
                &[],
            ),
        ),
        |args, e| {
            let project = e.world.resource::<AssetServer>().root.clone();
            let s = |k: &str| args.get(k).and_then(Json::as_str).map(str::to_owned);
            let mut note = Json::Null;
            match s("action").as_deref().unwrap_or("list") {
                "list" | "resolve" => {}
                "add" => {
                    let req = if let Some(p) = s("path") {
                        AddRequest::Disk(p.into())
                    } else if let Some(g) = s("git") {
                        AddRequest::Git(g)
                    } else if let Some(n) = s("name") {
                        AddRequest::Name {
                            name: n,
                            version: s("version"),
                        }
                    } else {
                        return ToolResult::error("add needs `path`, `git` or `name`");
                    };
                    match packages::add(&project, &req, &mut NetFetcher) {
                        Ok(n) => note = json!({ "added": n }),
                        Err(err) => return ToolResult::error(err),
                    }
                }
                "remove" => {
                    let Some(n) = s("name") else {
                        return ToolResult::error("remove needs `name`");
                    };
                    match packages::remove(&project, &n) {
                        Ok(true) => note = json!({ "removed": n }),
                        Ok(false) => {
                            return ToolResult::error(format!(
                                "{n} isn't in the manifest (embedded packages are folders in Packages/)"
                            ));
                        }
                        Err(err) => return ToolResult::error(err),
                    }
                }
                "import_sample" => {
                    let (list, _) = packages::resolve(&project, &mut NetFetcher);
                    let (Some(n), Some(sample)) = (s("name"), s("sample")) else {
                        return ToolResult::error("import_sample needs `name` and `sample`");
                    };
                    let Some(p) = list.iter().find(|p| p.name == n) else {
                        return ToolResult::error(format!("no package {n}"));
                    };
                    let Some(smp) = p
                        .samples
                        .iter()
                        .find(|x| x.display_name.eq_ignore_ascii_case(&sample))
                    else {
                        return ToolResult::error(format!("{n} has no sample {sample}"));
                    };
                    match packages::import_sample(&project, p, smp) {
                        Ok(rel) => note = json!({ "imported": rel }),
                        Err(err) => return ToolResult::error(err),
                    }
                }
                other => return ToolResult::error(format!("unknown action `{other}`")),
            }
            let (list, errors) = packages::resolve(&project, &mut NetFetcher);
            let scripts = packages::sync_script_dependencies(&project, &list).unwrap_or(false);
            e.world.resource_mut::<AssetServer>().package_roots = packages::roots(&list);
            ToolResult::json(json!({
                "packages": list.iter().map(package_json).collect::<Vec<_>>(),
                "errors": errors,
                "scripts_crate_updated": scripts,
                "result": note,
            }))
        },
    );
}
