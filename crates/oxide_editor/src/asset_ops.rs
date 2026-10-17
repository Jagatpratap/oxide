//! Asset file operations for the Project window: rename, move, duplicate, delete and
//! "Reveal in Finder". Each asset's `.meta` file goes with it.
//!
//! Assets have GUIDs in their `.meta` files (Unity's AssetDatabase), but references are
//! written as paths, so moving or renaming an asset rewrites the references to it: in every
//! text asset, in project settings, in the open scene and in the Undo history. Assets moved
//! outside the editor are found by GUID on the next refresh and fixed the same way.
//! Reference: <https://docs.unity3d.com/Manual/AssetWorkflow.html>

use crate::EditorState;
use oxide_assets::AssetServer;
use oxide_engine::Engine;
use std::path::{Path, PathBuf};

/// Binary asset files, which never hold references to other assets; every other file under
/// `Assets/` that parses as YAML is checked.
const BINARY: [&str; 28] = [
    "png",
    "jpg",
    "jpeg",
    "tga",
    "bmp",
    "hdr",
    "exr",
    "ktx2",
    "psd",
    "tif",
    "tiff",
    "glb",
    "fbx",
    "obj",
    "wav",
    "ogg",
    "mp3",
    "flac",
    "ttf",
    "otf",
    "bytes",
    "terraindata",
    "navmesh",
    "lightmap",
    "mp4",
    "webm",
    "dylib",
    "so",
];

/// `p` after `old` moved to `new` (`old` itself, a sub-asset `old#mesh0`, or a file inside
/// the moved folder `old/x.png`), or `None` if `p` doesn't refer to it.
pub fn remap(p: &str, old: &str, new: &str) -> Option<String> {
    if p == old {
        return Some(new.to_owned());
    }
    let rest = p.strip_prefix(old)?;
    (rest.starts_with('#') || rest.starts_with('/')).then(|| format!("{new}{rest}"))
}

/// Rewrites every string in a YAML document that refers to `old`. Returns how many changed.
fn remap_yaml(v: &mut serde_yaml_ng::Value, old: &str, new: &str) -> usize {
    use serde_yaml_ng::Value as Y;
    match v {
        Y::String(s) => match remap(s, old, new) {
            Some(n) => {
                *s = n;
                1
            }
            None => 0,
        },
        Y::Sequence(items) => items.iter_mut().map(|i| remap_yaml(i, old, new)).sum(),
        Y::Mapping(m) => {
            // Keys too: GUID tables and prefab snapshots are keyed by asset path.
            let mut n = 0;
            let entries: Vec<(Y, Y)> = std::mem::take(m).into_iter().collect();
            for (mut k, mut v) in entries {
                n += remap_yaml(&mut k, old, new) + remap_yaml(&mut v, old, new);
                m.insert(k, v);
            }
            n
        }
        Y::Tagged(t) => remap_yaml(&mut t.value, old, new),
        _ => 0,
    }
}

/// A free path for a new or copied asset: `rel` itself, or `Name 1.ext`, `Name 2.ext`...
/// (a trailing number on the name is continued, like Unity).
pub fn unique_path(root: &Path, rel: &str) -> String {
    if !root.join(rel).exists() {
        return rel.to_owned();
    }
    let (dir, file) = rel.rsplit_once('/').unwrap_or(("", rel));
    let is_dir = root.join(rel).is_dir();
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) if !is_dir && !s.is_empty() => (s, format!(".{e}")),
        _ => (file, String::new()),
    };
    let (base, start) = match stem.rsplit_once(' ') {
        Some((b, n)) if !b.is_empty() && n.parse::<u32>().is_ok() => {
            (b, n.parse::<u32>().unwrap_or(0) + 1)
        }
        _ => (stem, 1),
    };
    (start..)
        .map(|n| {
            let name = format!("{base} {n}{ext}");
            if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            }
        })
        .find(|p| !root.join(p).exists())
        .expect("unbounded")
}

/// A file or folder name typed by the user, or why it can't be used.
pub fn check_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err("the name is empty".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err(format!("names can't contain `{c}`"));
    }
    if name.starts_with('.') || name.ends_with(".meta") {
        return Err("that name would hide the asset".into());
    }
    Ok(name)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 16 {
        return;
    }
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out, depth + 1);
        } else {
            out.push(p);
        }
    }
}

/// Rewrites references to `old` in the project's text assets and settings on disk.
/// Returns the changed files (project-relative).
fn fix_files(root: &Path, old: &str, new: &str) -> Vec<String> {
    let mut files = Vec::new();
    walk(&root.join("Assets"), &mut files, 0);
    walk(&root.join("ProjectSettings"), &mut files, 0);
    let mut changed = Vec::new();
    for f in files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if BINARY.contains(&ext.as_str()) || ext == "meta" || ext == "rs" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        if !text.contains(old) {
            continue;
        }
        let Ok(mut doc) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text) else {
            continue;
        };
        if remap_yaml(&mut doc, old, new) == 0 {
            continue;
        }
        match serde_yaml_ng::to_string(&doc) {
            Ok(out) if std::fs::write(&f, &out).is_ok() => {
                let rel = f.strip_prefix(root).unwrap_or(&f);
                changed.push(rel.to_string_lossy().replace('\\', "/"));
            }
            _ => oxide_core::debug_log_warning!("could not update references in {}", f.display()),
        }
    }
    // The scene reopened at launch.
    let last = crate::launch::last_scene_file(root);
    if let Ok(t) = std::fs::read_to_string(&last)
        && let Some(n) = remap(t.trim(), old, new)
    {
        let _ = std::fs::write(&last, n);
    }
    changed
}

/// Moves (or renames) an asset or folder from `old` to `new` (project-relative paths),
/// with its `.meta` file, and updates every reference to it. Returns a status message.
pub fn move_asset(
    st: &mut EditorState,
    engine: &mut Engine,
    old: &str,
    new: &str,
) -> Result<String, String> {
    if st.playing() {
        return Err("stop Play mode before moving assets".into());
    }
    if old == new {
        return Ok(String::new());
    }
    if !old.starts_with("Assets/") || !(new.starts_with("Assets/")) {
        return Err("only assets under Assets/ can be moved".into());
    }
    if new.starts_with(&format!("{old}/")) {
        return Err("a folder can't be moved into itself".into());
    }
    let root = engine.world.resource::<AssetServer>().full_path("");
    let (from, to) = (root.join(old), root.join(new));
    if !from.exists() {
        return Err(format!("{old} doesn't exist"));
    }
    // Case-only renames are the same file on macOS and Windows.
    if to.exists() && !old.eq_ignore_ascii_case(new) {
        return Err(format!("{new} already exists"));
    }
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::rename(&from, &to).map_err(|e| format!("cannot move {old}: {e}"))?;
    let meta = |p: &Path| PathBuf::from(format!("{}.meta", p.display()));
    if meta(&from).exists() {
        let _ = std::fs::rename(meta(&from), meta(&to));
    }
    engine
        .world
        .resource_mut::<AssetServer>()
        .database
        .moved(old, new);
    let (files, refs) = update_references(st, engine, old, new);
    Ok(format!(
        "Moved {old} to {new} ({files} files and {refs} scene references updated)"
    ))
}

/// After `old` moved to `new` (by the editor or outside it): rewrites references in files on
/// disk, loaded materials, the open scenes and the Undo history. Returns (files, scene refs).
pub fn update_references(
    st: &mut EditorState,
    engine: &mut Engine,
    old: &str,
    new: &str,
) -> (usize, usize) {
    let root = engine.world.resource::<AssetServer>().full_path("");
    let files = fix_files(&root, old, new);
    // Loaded materials that pointed at the moved asset pick up the new path.
    let assets = engine.world.resource_mut::<AssetServer>();
    for f in &files {
        if f.ends_with(".mat") && assets.materials.handle_of(f).is_some() {
            let _ = assets.load_material(f);
        }
    }

    // The open scene and the Undo history.
    let mut f = |p: &str| remap(p, old, new);
    st.undo.remap_assets(&mut f);
    let w = &mut engine.world;
    let mut snap = crate::undo::Snapshot::take(w, "", &st.selection);
    let refs = snap.remap_assets(&mut f);
    let scene_moved = w.scene_path.as_deref().and_then(|p| remap(p, old, new));
    if refs > 0 {
        let before = st.remember_paths(w);
        st.selection = snap.restore(w);
        st.remap_paths(w, &before);
        let file = w.serialize(&[], &w.scene_name.clone());
        w.ensure_assets(&file);
    }
    if let Some(p) = scene_moved {
        w.scene_path = Some(p);
    }
    st.project.cache = None;
    if st.project.selected.as_deref() == Some(old) {
        st.project.selected = Some(new.to_owned());
    }
    if let Some(n) = remap(&st.project.folder, old, new) {
        st.project.folder = n;
    }
    (files.len(), refs)
}

/// Scans `Assets/` for new, moved and deleted assets (Unity's AssetDatabase.Refresh) and
/// fixes references to assets moved outside the editor. Returns a summary, or `None` if
/// nothing changed.
pub fn refresh_database(st: &mut EditorState, engine: &mut Engine) -> Option<String> {
    let report = engine
        .world
        .resource_mut::<AssetServer>()
        .database
        .refresh();
    apply_refresh(st, engine, &report)
}

/// Fixes references after a refresh found moved assets; a summary if anything changed.
pub fn apply_refresh(
    st: &mut EditorState,
    engine: &mut Engine,
    report: &oxide_assets::database::RefreshReport,
) -> Option<String> {
    let mut fixed = 0;
    for (old, new) in &report.moved {
        let (f, r) = update_references(st, engine, old, new);
        fixed += f + r;
        oxide_core::debug_log!("{old} was moved to {new}; updated {} references", f + r);
    }
    for p in &report.new_guids {
        oxide_core::debug_log_warning!(
            "{p} had a copy of another asset's .meta; it got a new GUID"
        );
    }
    let changed = !report.moved.is_empty()
        || !report.imported.is_empty()
        || !report.deleted.is_empty()
        || !report.new_guids.is_empty();
    if changed {
        st.project.invalidate();
    }
    changed.then(|| {
        format!(
            "Asset database: {} new, {} moved ({fixed} references updated), {} deleted",
            report.imported.len(),
            report.moved.len(),
            report.deleted.len()
        )
    })
}

/// Copies an asset or folder (with `.meta` files) next to itself. Returns the new path.
pub fn duplicate_asset(root: &Path, path: &str) -> Result<String, String> {
    let new = unique_path(root, path);
    copy(&root.join(path), &root.join(&new))?;
    let meta = root.join(format!("{path}.meta"));
    if meta.is_file() {
        let _ = std::fs::copy(meta, root.join(format!("{new}.meta")));
    }
    // The copy keeps the import settings and labels but needs GUIDs of its own; the next
    // AssetDatabase refresh gives it new ones.
    strip_guids(&root.join(format!("{new}.meta")));
    let mut metas = Vec::new();
    walk(&root.join(&new), &mut metas, 0);
    for m in metas
        .iter()
        .filter(|m| m.extension().is_some_and(|e| e == "meta"))
    {
        strip_guids(m);
    }
    Ok(new)
}

fn strip_guids(meta: &Path) {
    let Ok(text) = std::fs::read_to_string(meta) else {
        return;
    };
    let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with("guid:")).collect();
    if kept.len() != text.lines().count() {
        let _ = std::fs::write(meta, kept.join("\n") + "\n");
    }
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    if from.is_dir() {
        std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
        for e in std::fs::read_dir(from)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            copy(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to)
            .map(drop)
            .map_err(|e| format!("cannot copy {}: {e}", from.display()))
    }
}

/// Deletes an asset or folder and its `.meta` file. On macOS it goes to the Trash, so it
/// can be recovered; elsewhere it is removed (as Unity does).
pub fn delete_asset(root: &Path, path: &str) -> Result<(), String> {
    if !path.starts_with("Assets/") {
        return Err("only assets under Assets/ can be deleted".into());
    }
    let full = root.join(path);
    let meta = root.join(format!("{path}.meta"));
    if cfg!(target_os = "macos")
        && let Some(home) = std::env::var_os("HOME")
    {
        let trash = PathBuf::from(home).join(".Trash");
        if trash.is_dir() {
            let name = full.file_name().map(|n| n.to_string_lossy().into_owned());
            let name = name.unwrap_or_else(|| "asset".into());
            let dest = trash.join(unique_path(&trash, &name));
            if std::fs::rename(&full, &dest).is_ok() {
                let _ = std::fs::remove_file(&meta);
                return Ok(());
            }
        }
    }
    let r = if full.is_dir() {
        std::fs::remove_dir_all(&full)
    } else {
        std::fs::remove_file(&full)
    };
    r.map_err(|e| format!("cannot delete {path}: {e}"))?;
    let _ = std::fs::remove_file(&meta);
    Ok(())
}

/// Shows the file in Finder / Explorer (Unity "Reveal in Finder" / "Show in Explorer").
pub fn reveal(full: &Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(full);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", full.display()));
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(full.parent().unwrap_or(full));
        c
    };
    cmd.spawn()
        .map(drop)
        .map_err(|e| format!("cannot open the file browser: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_files_subassets_and_folder_contents() {
        let r = |p| remap(p, "Assets/Cars/car.glb", "Assets/Vehicles/car.glb");
        assert_eq!(
            r("Assets/Cars/car.glb").as_deref(),
            Some("Assets/Vehicles/car.glb")
        );
        assert_eq!(
            r("Assets/Cars/car.glb#mesh0").as_deref(),
            Some("Assets/Vehicles/car.glb#mesh0")
        );
        assert_eq!(r("Assets/Cars/car.glb.bak"), None);
        assert_eq!(r("Assets/Cars/car.glbx"), None);
        let d = |p| remap(p, "Assets/Cars", "Assets/Vehicles");
        assert_eq!(
            d("Assets/Cars/a.png").as_deref(),
            Some("Assets/Vehicles/a.png")
        );
        assert_eq!(d("Assets/CarsOld/a.png"), None);
    }

    #[test]
    fn unique_names_continue_numbers() {
        let dir = std::env::temp_dir().join(format!("oxide-ops-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Assets/New Folder")).unwrap();
        std::fs::write(dir.join("Assets/Car.prefab"), "x").unwrap();
        std::fs::write(dir.join("Assets/Car 1.prefab"), "x").unwrap();
        assert_eq!(unique_path(&dir, "Assets/Free.mat"), "Assets/Free.mat");
        assert_eq!(
            unique_path(&dir, "Assets/Car.prefab"),
            "Assets/Car 2.prefab"
        );
        assert_eq!(
            unique_path(&dir, "Assets/Car 1.prefab"),
            "Assets/Car 2.prefab"
        );
        assert_eq!(
            unique_path(&dir, "Assets/New Folder"),
            "Assets/New Folder 1"
        );
        assert!(check_name("a/b").is_err());
        assert!(check_name(" ").is_err());
        assert_eq!(check_name(" Car ").unwrap(), "Car");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn moving_rewrites_references_on_disk() {
        let dir = std::env::temp_dir().join(format!("oxide-ops-fix-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Assets/Tex")).unwrap();
        std::fs::write(
            dir.join("Assets/Red.mat"),
            "shader: Oxide/Lit\nbase_map: !Asset Assets/Tex/red.png\nname: Assets/Texture/x.png\n",
        )
        .unwrap();
        std::fs::write(dir.join("Assets/Other.mat"), "base_map: null\n").unwrap();
        let changed = fix_files(&dir, "Assets/Tex", "Assets/Textures");
        assert_eq!(changed, vec!["Assets/Red.mat"]);
        let text = std::fs::read_to_string(dir.join("Assets/Red.mat")).unwrap();
        assert!(text.contains("Assets/Textures/red.png"), "{text}");
        assert!(text.contains("name: Assets/Texture/x.png"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
