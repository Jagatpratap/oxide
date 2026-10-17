//! Assembly Definitions (Unity's `.asmdef` / `.asmref`): script folders under `Assets/` that
//! compile as their own crates.
//!
//! Each `.asmdef` (Unity's JSON format) makes a crate from the `.rs` files in its folder and
//! subfolders (except subfolders with their own `.asmdef`; an `.asmref` adds its folder to
//! another assembly). `references` become Cargo dependencies, and auto-referenced
//! assemblies become dependencies of the main `Scripts` crate, so changing one assembly only
//! rebuilds it and what depends on it, as in Unity. Platform lists become target-specific
//! dependencies; Editor-only assemblies stay out of player builds.
//! Reference: <https://docs.unity3d.com/Manual/ScriptCompilationAssemblyDefinitionFiles.html>

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

fn yes() -> bool {
    true
}

/// An `.asmdef` file (Unity's fields; unknown ones are kept as-is by Unity, ignored here).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsmDef {
    pub name: String,
    #[serde(default)]
    pub root_namespace: String,
    /// Other assemblies by name (Unity also accepts `GUID:...`, unsupported here).
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub include_platforms: Vec<String>,
    #[serde(default)]
    pub exclude_platforms: Vec<String>,
    #[serde(default)]
    pub allow_unsafe_code: bool,
    #[serde(default)]
    pub override_references: bool,
    #[serde(default)]
    pub precompiled_references: Vec<String>,
    #[serde(default = "yes")]
    pub auto_referenced: bool,
    #[serde(default)]
    pub define_constraints: Vec<String>,
    #[serde(default)]
    pub version_defines: Vec<serde_json::Value>,
    #[serde(default)]
    pub no_engine_references: bool,
}

impl AsmDef {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            root_namespace: String::new(),
            references: Vec::new(),
            include_platforms: Vec::new(),
            exclude_platforms: Vec::new(),
            allow_unsafe_code: false,
            override_references: false,
            precompiled_references: Vec::new(),
            auto_referenced: true,
            define_constraints: Vec::new(),
            version_defines: Vec::new(),
            no_engine_references: false,
        }
    }

    /// Only the Editor is included (Unity's editor assemblies).
    pub fn editor_only(&self) -> bool {
        !self.include_platforms.is_empty() && self.include_platforms.iter().all(|p| p == "Editor")
    }
}

/// An `.asmref` file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AsmRef {
    pub reference: String,
}

/// Unity's platform names and the Rust `cfg` they compile for.
pub const PLATFORMS: &[(&str, &str)] = &[
    ("Editor", ""),
    ("WindowsStandalone64", "target_os = \"windows\""),
    ("macOSStandalone", "target_os = \"macos\""),
    ("LinuxStandalone64", "target_os = \"linux\""),
    ("Android", "target_os = \"android\""),
    ("iOS", "target_os = \"ios\""),
    ("WebGL", "target_arch = \"wasm32\""),
];

/// One discovered assembly.
#[derive(Clone, Debug, PartialEq)]
pub struct Assembly {
    pub def: AsmDef,
    /// The `.asmdef` file, relative to the project.
    pub file: String,
    /// Absolute folder the assembly covers.
    pub folder: PathBuf,
    /// Rust crate name (the name in snake case).
    pub crate_name: String,
    /// Absolute paths of its `.rs` files.
    pub sources: Vec<PathBuf>,
}

impl Assembly {
    /// The `cfg` a dependency on this assembly is gated on (None: every platform).
    pub fn cfg(&self) -> Option<String> {
        let map = |names: &[String]| -> Vec<String> {
            names
                .iter()
                .filter_map(|n| PLATFORMS.iter().find(|(p, _)| p == n).map(|(_, c)| *c))
                .filter(|c| !c.is_empty())
                .map(str::to_owned)
                .collect()
        };
        let inc = map(&self.def.include_platforms);
        let exc = map(&self.def.exclude_platforms);
        if !inc.is_empty() {
            Some(format!("cfg(any({}))", inc.join(", ")))
        } else if !exc.is_empty() {
            Some(format!("cfg(not(any({})))", exc.join(", ")))
        } else {
            None
        }
    }
}

/// A crate name from an assembly name (`Company.Weapons` → `company_weapons`).
pub fn crate_name(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if prev_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower = false;
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_lower = c.is_ascii_lowercase();
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
            prev_lower = false;
        }
    }
    let out = out.trim_matches('_').to_owned();
    match out.as_str() {
        "" => "assembly".into(),
        "scripts" | "oxide" | "player" | "std" | "core" => format!("{out}_assembly"),
        s if s.starts_with(|c: char| c.is_ascii_digit()) => format!("a_{s}"),
        _ => out,
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Finds every assembly under `Assets/` and the files it owns.
pub fn discover(project: &Path) -> (Vec<Assembly>, Vec<String>) {
    let mut files = Vec::new();
    walk(&project.join("Assets"), &mut files);
    files.sort();
    let mut errors = Vec::new();
    let rel = |p: &Path| {
        p.strip_prefix(project)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let mut assemblies: Vec<Assembly> = Vec::new();
    // Folder → assembly name (asmdef folders and asmref folders).
    let mut owners: Vec<(PathBuf, String)> = Vec::new();
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "asmdef"))
    {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let def: AsmDef = match serde_json::from_str(&text) {
            Ok(d) => d,
            Err(e) => {
                errors.push(format!("{}: {e}", rel(f)));
                continue;
            }
        };
        if def.name.trim().is_empty() {
            errors.push(format!("{}: the assembly has no name", rel(f)));
            continue;
        }
        if assemblies.iter().any(|a| a.def.name == def.name) {
            errors.push(format!(
                "{}: another assembly is already named `{}`",
                rel(f),
                def.name
            ));
            continue;
        }
        let folder = f.parent().unwrap_or(project).to_path_buf();
        if owners.iter().any(|(d, _)| *d == folder) {
            errors.push(format!(
                "{}: a folder can only have one assembly definition",
                rel(f)
            ));
            continue;
        }
        owners.push((folder.clone(), def.name.clone()));
        assemblies.push(Assembly {
            crate_name: crate_name(&def.name),
            def,
            file: rel(f),
            folder,
            sources: Vec::new(),
        });
    }
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "asmref"))
    {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        match serde_json::from_str::<AsmRef>(&text) {
            Ok(r) if assemblies.iter().any(|a| a.def.name == r.reference) => {
                owners.push((f.parent().unwrap_or(project).to_path_buf(), r.reference));
            }
            Ok(r) => errors.push(format!("{}: no assembly named `{}`", rel(f), r.reference)),
            Err(e) => errors.push(format!("{}: {e}", rel(f))),
        }
    }
    // Each script belongs to the nearest folder above it with an asmdef or asmref.
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "rs"))
    {
        let owner = owners
            .iter()
            .filter(|(d, _)| f.starts_with(d))
            .max_by_key(|(d, _)| d.components().count());
        if let Some((_, name)) = owner
            && let Some(a) = assemblies.iter_mut().find(|a| a.def.name == *name)
        {
            a.sources.push(f.clone());
        }
    }
    // References must exist and not loop.
    let names: Vec<String> = assemblies.iter().map(|a| a.def.name.clone()).collect();
    for a in &assemblies {
        for r in &a.def.references {
            if !names.contains(r) {
                errors.push(format!("{}: references unknown assembly `{r}`", a.file));
            }
        }
    }
    let deps: HashMap<&str, &Vec<String>> = assemblies
        .iter()
        .map(|a| (a.def.name.as_str(), &a.def.references))
        .collect();
    for a in &assemblies {
        let mut stack: Vec<&str> = a.def.references.iter().map(String::as_str).collect();
        let mut seen = Vec::new();
        while let Some(n) = stack.pop() {
            if n == a.def.name {
                errors.push(format!("{}: assembly references form a loop", a.file));
                break;
            }
            if seen.contains(&n) {
                continue;
            }
            seen.push(n);
            if let Some(d) = deps.get(n) {
                stack.extend(d.iter().map(String::as_str));
            }
        }
    }
    (assemblies, errors)
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn write_if_changed(path: &Path, text: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(text) {
        return Ok(());
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Module names for an assembly's files: the file stem, prefixed by its folders when stems
/// repeat (`guns/rifle.rs` → `guns_rifle`).
fn module_names(a: &Assembly) -> Vec<(String, PathBuf)> {
    let stem = |p: &Path| {
        crate::project_files::snake_case(&p.file_stem().unwrap_or_default().to_string_lossy())
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in &a.sources {
        *counts.entry(stem(s)).or_default() += 1;
    }
    a.sources
        .iter()
        .filter(|s| {
            s.file_name()
                .is_some_and(|n| n != "lib.rs" && n != "mod.rs")
        })
        .map(|s| {
            let st = stem(s);
            let name = if counts[&st] > 1 {
                let rel = s.strip_prefix(&a.folder).unwrap_or(s);
                crate::project_files::snake_case(
                    &rel.with_extension("")
                        .to_string_lossy()
                        .replace(['/', '\\'], "_"),
                )
            } else {
                st
            };
            let name = if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                format!("m_{name}")
            } else {
                name
            };
            (name, s.clone())
        })
        .collect()
}

/// The engine crate an assembly depends on: the editor's dylib or the player.
#[derive(Clone, Debug)]
pub struct EngineDep {
    pub package: &'static str,
    pub path: PathBuf,
}

/// Writes one crate per assembly into `out` (`<out>/<crate>/Cargo.toml` and `lib.rs`).
/// Editor-only assemblies are skipped when `player`. Returns the assemblies written.
pub fn generate(
    assemblies: &[Assembly],
    out: &Path,
    engine: &EngineDep,
    player: bool,
) -> Result<Vec<Assembly>, String> {
    let written: Vec<Assembly> = assemblies
        .iter()
        .filter(|a| !(player && a.def.editor_only()))
        .cloned()
        .collect();
    // Remove crates of assemblies that are gone.
    for e in std::fs::read_dir(out).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if e.path().join("Cargo.toml").is_file() && !written.iter().any(|a| a.crate_name == name) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    for a in &written {
        let dir = out.join(&a.crate_name);
        let custom_root = a.folder.join("lib.rs");
        let lib_path = if custom_root.is_file() {
            path_str(&custom_root)
        } else {
            "lib.rs".to_owned()
        };
        let mut deps = String::new();
        if !a.def.no_engine_references {
            deps.push_str(&format!(
                "oxide = {{ package = \"{}\", path = \"{}\" }}\n",
                engine.package,
                path_str(&engine.path)
            ));
        }
        for r in &a.def.references {
            if let Some(d) = written.iter().find(|x| x.def.name == *r) {
                deps.push_str(&format!(
                    "{} = {{ path = \"../{}\" }}\n",
                    d.crate_name, d.crate_name
                ));
            }
        }
        let manifest = format!(
            "# Generated by Oxide from {}. Edit the .asmdef, not this file.\n[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\npath = \"{lib_path}\"\n\n[dependencies]\n{deps}",
            a.file, a.crate_name
        );
        write_if_changed(&dir.join("Cargo.toml"), &manifest)?;
        if !custom_root.is_file() {
            let mut lib = format!(
                "// Generated by Oxide from {}: every script in the assembly's folder.\n",
                a.file
            );
            if !a.def.allow_unsafe_code {
                lib.push_str("#![forbid(unsafe_code)]\n");
            }
            for (m, p) in module_names(a) {
                lib.push_str(&format!(
                    "\n#[path = \"{}\"]\npub mod {m};\n#[allow(unused_imports)]\npub use {m}::*;\n",
                    path_str(p.as_path())
                ));
            }
            write_if_changed(&dir.join("lib.rs"), &lib)?;
        } else {
            let _ = std::fs::remove_file(dir.join("lib.rs"));
        }
    }
    Ok(written)
}

const BEGIN: &str = "# >>> Oxide Assembly Definitions (generated; edit the .asmdef files instead)";
const END: &str = "# <<< Oxide Assembly Definitions";

/// The generated dependency block for a `Scripts` manifest: auto-referenced assemblies,
/// platform-gated like their `.asmdef`.
pub fn dependency_block(assemblies: &[Assembly], rel_dir: &str) -> String {
    let mut out = String::new();
    for a in assemblies.iter().filter(|a| a.def.auto_referenced) {
        let table = match a.cfg() {
            Some(cfg) => format!("[target.'{cfg}'.dependencies.{}]", a.crate_name),
            None => format!("[dependencies.{}]", a.crate_name),
        };
        out.push_str(&format!("{table}\npath = \"{rel_dir}/{}\"\n", a.crate_name));
    }
    out
}

/// Replaces (or adds, or removes) the generated block at the end of a Cargo manifest.
pub fn set_block(manifest: &str, block: &str) -> String {
    let base = match (manifest.find(BEGIN), manifest.find(END)) {
        (Some(b), Some(e)) if e > b => {
            let end = manifest[e..]
                .find('\n')
                .map_or(manifest.len(), |n| e + n + 1);
            format!("{}{}", &manifest[..b], &manifest[end..])
        }
        _ => manifest.to_owned(),
    };
    let base = base.trim_end().to_owned() + "\n";
    if block.is_empty() {
        return base;
    }
    format!("{base}\n{BEGIN}\n{block}{END}\n")
}

/// Before a script build: regenerates the assembly crates under
/// `Library/ScriptAssemblies/`, the Scripts manifest's dependency block and the
/// workspace members. Returns problems to report.
pub fn sync(project: &Path, engine_root: &Path) -> Vec<String> {
    let (assemblies, mut errors) = discover(project);
    let out = project.join("Library/ScriptAssemblies");
    let engine = EngineDep {
        package: "oxide_dylib",
        path: engine_root.join("crates/oxide_dylib"),
    };
    let written = match generate(&assemblies, &out, &engine, false) {
        Ok(w) => w,
        Err(e) => {
            errors.push(e);
            return errors;
        }
    };
    let manifest_path = project.join("Scripts/Cargo.toml");
    if let Ok(m) = std::fs::read_to_string(&manifest_path) {
        let new = set_block(
            &m,
            &dependency_block(&written, "../Library/ScriptAssemblies"),
        );
        if let Err(e) = write_if_changed(&manifest_path, &new) {
            errors.push(e);
        }
    }
    // Workspace members.
    let root = project.join("Cargo.toml");
    if let Ok(text) = std::fs::read_to_string(&root) {
        let members: Vec<String> = ["Scripts".to_owned(), "Library/Build/editor".to_owned()]
            .into_iter()
            .chain(
                written
                    .iter()
                    .map(|a| format!("Library/ScriptAssemblies/{}", a.crate_name)),
            )
            .map(|m| format!("\"{m}\""))
            .collect();
        let line = format!("members = [{}]", members.join(", "));
        let new: String = text
            .lines()
            .map(|l| {
                if l.starts_with("members = ") {
                    line.clone()
                } else {
                    l.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        if let Err(e) = write_if_changed(&root, &new) {
            errors.push(e);
        }
    }
    errors
}

/// The `.asmdef`/`.asmref` and assembly sources, for the script host's change detection.
pub fn watched_files(project: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(&project.join("Assets"), &mut files);
    files
        .into_iter()
        .filter(|f| {
            f.extension()
                .is_some_and(|e| e == "asmdef" || e == "asmref" || e == "rs")
        })
        .collect()
}

/// The `assembly_definitions` MCP tool: lists assemblies, or creates and edits `.asmdef`
/// / `.asmref` files.
pub fn mcp(
    project: &Path,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use serde_json::{Value as Json, json};
    let fields = |d: &mut AsmDef, o: &serde_json::Map<String, Json>| -> Result<(), String> {
        let mut v = serde_json::to_value(&*d).map_err(|e| e.to_string())?;
        for (k, x) in o {
            if !matches!(k.as_str(), "folder" | "file") {
                v[k] = x.clone();
            }
        }
        *d = serde_json::from_value(v).map_err(|e| e.to_string())?;
        Ok(())
    };
    if let Some(c) = args.get("create").and_then(Json::as_object) {
        let folder = c
            .get("folder")
            .and_then(Json::as_str)
            .ok_or("create.folder (under Assets/)")?;
        let name = c.get("name").and_then(Json::as_str).ok_or("create.name")?;
        if !folder.starts_with("Assets") {
            return Err("assemblies live under Assets/".into());
        }
        let mut d = AsmDef::new(name);
        fields(&mut d, c)?;
        let file = project.join(folder).join(format!("{name}.asmdef"));
        write_if_changed(
            &file,
            &serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?,
        )?;
    }
    if let Some(c) = args.get("set").and_then(Json::as_object) {
        let name = c
            .get("name")
            .and_then(Json::as_str)
            .ok_or("set.name (the assembly to edit)")?;
        let (all, _) = discover(project);
        let a = all
            .iter()
            .find(|a| a.def.name == name)
            .ok_or_else(|| format!("no assembly `{name}`"))?;
        let mut d = a.def.clone();
        let mut patch = c.clone();
        if let Some(n) = c.get("rename") {
            patch.insert("name".into(), n.clone());
        } else {
            patch.remove("name");
        }
        patch.remove("rename");
        fields(&mut d, &patch)?;
        write_if_changed(
            &project.join(&a.file),
            &serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?,
        )?;
    }
    if let Some(c) = args.get("asmref").and_then(Json::as_object) {
        let folder = c
            .get("folder")
            .and_then(Json::as_str)
            .ok_or("asmref.folder")?;
        let reference = c
            .get("reference")
            .and_then(Json::as_str)
            .ok_or("asmref.reference")?;
        let file = project.join(folder).join("Reference.asmref");
        let r = AsmRef {
            reference: reference.into(),
        };
        write_if_changed(
            &file,
            &serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?,
        )?;
    }
    let (all, errors) = discover(project);
    Ok(json!({
        "assemblies": all.iter().map(|a| json!({
            "name": a.def.name,
            "file": a.file,
            "crate": a.crate_name,
            "references": a.def.references,
            "auto_referenced": a.def.auto_referenced,
            "allow_unsafe_code": a.def.allow_unsafe_code,
            "include_platforms": a.def.include_platforms,
            "exclude_platforms": a.def.exclude_platforms,
            "editor_only": a.def.editor_only(),
            "scripts": a.sources.iter().map(|s| s.strip_prefix(project).unwrap_or(s).to_string_lossy().replace('\\', "/")).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "errors": errors,
    }))
}

/// Assemblies by name → the crate path a script in `folder` would join (for Create →
/// Rust Script inside an assembly's folder).
pub fn owner_of(project: &Path, folder: &Path) -> Option<Assembly> {
    let (assemblies, _) = discover(project);
    let mut owners: BTreeMap<usize, Assembly> = BTreeMap::new();
    for a in assemblies {
        if folder.starts_with(&a.folder) {
            owners.insert(a.folder.components().count(), a);
        }
    }
    owners.into_iter().next_back().map(|(_, a)| a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oxide-asm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets")).unwrap();
        dir
    }

    fn put(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn names_become_crate_names() {
        assert_eq!(crate_name("Company.Weapons"), "company_weapons");
        assert_eq!(crate_name("GameAI"), "game_ai");
        assert_eq!(crate_name("Scripts"), "scripts_assembly");
        assert_eq!(crate_name("3D"), "a_3d");
    }

    #[test]
    fn folders_own_their_scripts() {
        let p = project("own");
        put(
            &p,
            "Assets/Game/Game.asmdef",
            r#"{ "name": "Game", "references": ["Weapons"] }"#,
        );
        put(&p, "Assets/Game/player.rs", "pub struct Player;");
        put(&p, "Assets/Game/ai/brain.rs", "pub struct Brain;");
        put(
            &p,
            "Assets/Game/Weapons/Weapons.asmdef",
            r#"{ "name": "Weapons", "allowUnsafeCode": true }"#,
        );
        put(&p, "Assets/Game/Weapons/gun.rs", "pub struct Gun;");
        put(
            &p,
            "Assets/Shared/Extra.asmref",
            r#"{ "reference": "Weapons" }"#,
        );
        put(&p, "Assets/Shared/ammo.rs", "pub struct Ammo;");
        put(
            &p,
            "Assets/Editor/Tools.asmdef",
            r#"{ "name": "Tools", "includePlatforms": ["Editor"], "autoReferenced": false }"#,
        );
        put(&p, "Assets/Editor/menu.rs", "");
        put(&p, "Assets/loose.rs", "");
        let (a, errors) = discover(&p);
        assert!(errors.is_empty(), "{errors:?}");
        let by = |n: &str| a.iter().find(|x| x.def.name == n).unwrap();
        let names = |x: &Assembly| {
            let mut v: Vec<String> = x
                .sources
                .iter()
                .map(|s| s.file_name().unwrap().to_string_lossy().to_string())
                .collect();
            v.sort();
            v
        };
        assert_eq!(names(by("Game")), vec!["brain.rs", "player.rs"]);
        assert_eq!(names(by("Weapons")), vec!["ammo.rs", "gun.rs"]);
        assert!(by("Tools").def.editor_only());
        // Generation: crates, references, unsafe and platform rules.
        let out = p.join("Library/ScriptAssemblies");
        let engine = EngineDep {
            package: "oxide_dylib",
            path: PathBuf::from("/engine/crates/oxide_dylib"),
        };
        let w = generate(&a, &out, &engine, false).unwrap();
        assert_eq!(w.len(), 3);
        let game = std::fs::read_to_string(out.join("game/Cargo.toml")).unwrap();
        assert!(
            game.contains("weapons = { path = \"../weapons\" }"),
            "{game}"
        );
        let lib = std::fs::read_to_string(out.join("game/lib.rs")).unwrap();
        assert!(
            lib.contains("#![forbid(unsafe_code)]")
                && lib.contains("pub mod player;")
                && lib.contains("pub mod brain;"),
            "{lib}"
        );
        let wl = std::fs::read_to_string(out.join("weapons/lib.rs")).unwrap();
        assert!(!wl.contains("forbid(unsafe_code)"));
        // Players leave Editor assemblies out.
        let pw = generate(&a, &p.join("player"), &engine, true).unwrap();
        assert!(pw.iter().all(|x| x.def.name != "Tools"));
        // Scripts depends on auto-referenced assemblies only.
        let block = dependency_block(&w, "../Library/ScriptAssemblies");
        assert!(
            block.contains("[dependencies.game]")
                && block.contains("[dependencies.weapons]")
                && !block.contains("tools"),
            "{block}"
        );
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn bad_references_are_reported() {
        let p = project("bad");
        put(
            &p,
            "Assets/A/A.asmdef",
            r#"{ "name": "A", "references": ["B"] }"#,
        );
        put(
            &p,
            "Assets/B/B.asmdef",
            r#"{ "name": "B", "references": ["A", "Missing"] }"#,
        );
        put(&p, "Assets/C/C.asmdef", r#"{ "name": "A" }"#);
        let (_, errors) = discover(&p);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("unknown assembly `Missing`")),
            "{errors:?}"
        );
        assert!(errors.iter().any(|e| e.contains("loop")), "{errors:?}");
        assert!(
            errors.iter().any(|e| e.contains("already named")),
            "{errors:?}"
        );
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn platform_cfgs_and_manifest_blocks() {
        let mut a = Assembly {
            def: AsmDef::new("Mobile"),
            file: "Assets/Mobile/Mobile.asmdef".into(),
            folder: PathBuf::from("/p/Assets/Mobile"),
            crate_name: "mobile".into(),
            sources: Vec::new(),
        };
        a.def.include_platforms = vec!["Android".into(), "iOS".into()];
        assert_eq!(
            a.cfg().as_deref(),
            Some("cfg(any(target_os = \"android\", target_os = \"ios\"))")
        );
        a.def.include_platforms.clear();
        a.def.exclude_platforms = vec!["WebGL".into()];
        assert_eq!(
            a.cfg().as_deref(),
            Some("cfg(not(any(target_arch = \"wasm32\")))")
        );
        let m = "[package]\nname = \"scripts\"\n\n[dependencies]\noxide = 1\n";
        let once = set_block(m, &dependency_block(std::slice::from_ref(&a), "../X"));
        assert!(
            once.contains("[target.'cfg(not(any(target_arch = \"wasm32\")))'.dependencies.mobile]")
        );
        // Idempotent, and removable.
        assert_eq!(
            set_block(&once, &dependency_block(std::slice::from_ref(&a), "../X")),
            once
        );
        assert_eq!(set_block(&once, ""), m);
    }
}
