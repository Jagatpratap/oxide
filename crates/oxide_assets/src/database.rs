//! Unity's AssetDatabase: every file and folder under `Assets/` has a `.meta` file with a
//! `guid`, which stays the same when the asset is moved or renamed. Asset references are
//! written as paths (readable, diff-friendly) and files that hold references also record
//! the GUID of each path, so a reference still resolves after its asset was moved outside
//! the editor (Finder, git).
//!
//! The last scan is cached in `Library/AssetDatabase.yaml`; comparing a new scan with it
//! finds assets that moved while the editor was closed.
//! Reference: <https://docs.unity3d.com/Manual/AssetDatabase.html>

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A new random 128-bit GUID as 32 lowercase hex digits (Unity's format).
pub fn new_guid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let word = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(nanos ^ salt);
        h.write_u64(n);
        h.write_u32(std::process::id());
        h.finish()
    };
    format!("{:016x}{:016x}", word(0x9e37_79b9), word(0x7f4a_7c15))
}

/// The file part of an asset path (`Assets/m.glb#mesh0` → `Assets/m.glb`).
pub fn file_of(path: &str) -> &str {
    path.split_once('#').map_or(path, |(f, _)| f)
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.') || name.ends_with('~') || name.ends_with(".meta")
}

/// What a [`AssetDatabase::refresh`] found.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RefreshReport {
    /// Assets that moved since the last scan, `(old, new)` (folders and files).
    pub moved: Vec<(String, String)>,
    /// New assets (they got a `.meta` with a new GUID if they had none).
    pub imported: Vec<String>,
    /// Assets that are gone.
    pub deleted: Vec<String>,
    /// Assets whose `.meta` was a copy of another's: they got a new GUID.
    pub new_guids: Vec<String>,
}

#[derive(Default)]
pub struct AssetDatabase {
    root: PathBuf,
    guid_to_path: HashMap<String, String>,
    path_to_guid: HashMap<String, String>,
    scanned: bool,
}

impl AssetDatabase {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            ..Default::default()
        }
    }

    fn cache_file(&self) -> PathBuf {
        self.root.join("Library/AssetDatabase.yaml")
    }

    pub fn is_scanned(&self) -> bool {
        self.scanned
    }

    /// Unity `AssetDatabase.AssetPathToGUID` (sub-assets share their file's GUID).
    pub fn guid_of(&self, path: &str) -> Option<&str> {
        self.path_to_guid.get(file_of(path)).map(String::as_str)
    }

    /// Unity `AssetDatabase.GUIDToAssetPath`.
    pub fn path_of(&self, guid: &str) -> Option<&str> {
        self.guid_to_path.get(guid).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.guid_to_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.guid_to_path.is_empty()
    }

    /// Every `(path, guid)`, sorted by path.
    pub fn all(&self) -> Vec<(&str, &str)> {
        let mut v: Vec<(&str, &str)> = self
            .path_to_guid
            .iter()
            .map(|(p, g)| (p.as_str(), g.as_str()))
            .collect();
        v.sort();
        v
    }

    /// Reads `guid:` from a `.meta` file.
    fn read_guid(meta: &Path) -> Option<String> {
        let text = std::fs::read_to_string(meta).ok()?;
        text.lines().find_map(|l| {
            let g = l
                .strip_prefix("guid:")?
                .trim()
                .trim_matches(|c| c == '"' || c == '\'');
            (g.len() == 32 && g.chars().all(|c| c.is_ascii_hexdigit())).then(|| g.to_owned())
        })
    }

    /// Writes `guid` into a `.meta` file, keeping its other fields (import settings, labels).
    fn write_guid(meta: &Path, guid: &str) -> Result<(), String> {
        let mut map: serde_yaml_ng::Mapping = std::fs::read_to_string(meta)
            .ok()
            .and_then(|t| serde_yaml_ng::from_str(&t).ok())
            .unwrap_or_default();
        // `guid` first, like Unity's meta files.
        map.remove("guid");
        let mut out = serde_yaml_ng::Mapping::new();
        out.insert("guid".into(), guid.into());
        out.extend(map);
        let text = serde_yaml_ng::to_string(&out).map_err(|e| e.to_string())?;
        std::fs::write(meta, text).map_err(|e| format!("cannot write {}: {e}", meta.display()))
    }

    fn walk(&self, dir: &Path, rel: &str, out: &mut Vec<String>, depth: usize) {
        if depth > 32 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_hidden(&name) {
                continue;
            }
            let p = format!("{rel}/{name}");
            out.push(p.clone());
            if e.path().is_dir() {
                self.walk(&e.path(), &p, out, depth + 1);
            }
        }
    }

    /// Scans `Assets/`: gives every asset without a `.meta` (or without a GUID in it) a new
    /// GUID, fixes duplicated GUIDs from copied `.meta` files, and reports what moved since
    /// the last scan. Unity's `AssetDatabase.Refresh`.
    pub fn refresh(&mut self) -> RefreshReport {
        let mut report = RefreshReport::default();
        let mut paths = Vec::new();
        let assets = self.root.join("Assets");
        if assets.is_dir() {
            self.walk(&assets, "Assets", &mut paths, 0);
        }
        let previous: HashMap<String, String> = if self.scanned {
            self.guid_to_path.clone()
        } else {
            std::fs::read_to_string(self.cache_file())
                .ok()
                .and_then(|t| serde_yaml_ng::from_str::<BTreeMap<String, String>>(&t).ok())
                .map(|m| m.into_iter().collect())
                .unwrap_or_default()
        };
        let mut guid_to_path: HashMap<String, String> = HashMap::new();
        let mut path_to_guid: HashMap<String, String> = HashMap::new();
        for p in &paths {
            let meta = self.root.join(format!("{p}.meta"));
            let mut guid = Self::read_guid(&meta);
            if let Some(g) = &guid
                && let Some(other) = guid_to_path.get(g)
            {
                // A copied `.meta`: the one that was here before keeps the GUID.
                if previous.get(g) == Some(p) {
                    path_to_guid.remove(other);
                    let other = other.clone();
                    let fresh = new_guid();
                    if Self::write_guid(&self.root.join(format!("{other}.meta")), &fresh).is_ok() {
                        path_to_guid.insert(other.clone(), fresh.clone());
                        guid_to_path.insert(fresh, other.clone());
                        report.new_guids.push(other);
                    }
                } else {
                    guid = None;
                    report.new_guids.push(p.clone());
                }
            }
            let guid = match guid {
                Some(g) => g,
                None => {
                    let g = new_guid();
                    if let Err(e) = Self::write_guid(&meta, &g) {
                        oxide_core::debug_log_warning!("{e}");
                    }
                    if !report.new_guids.contains(p) {
                        report.imported.push(p.clone());
                    }
                    g
                }
            };
            if let Some(old) = previous.get(&guid)
                && old != p
            {
                report.moved.push((old.clone(), p.clone()));
            } else if !previous.contains_key(&guid) && !report.imported.contains(p) {
                report.imported.push(p.clone());
            }
            guid_to_path.insert(guid.clone(), p.clone());
            path_to_guid.insert(p.clone(), guid);
        }
        for (g, p) in &previous {
            if !guid_to_path.contains_key(g) {
                report.deleted.push(p.clone());
            }
        }
        // A moved folder moves everything in it: report the folder only.
        let folders: Vec<(String, String)> = report
            .moved
            .iter()
            .filter(|(_, n)| self.root.join(n).is_dir())
            .cloned()
            .collect();
        report.moved.retain(|(o, n)| {
            !folders.iter().any(|(fo, fnew)| {
                (o.starts_with(&format!("{fo}/")) && n.starts_with(&format!("{fnew}/")))
                    && o[fo.len()..] == n[fnew.len()..]
            })
        });
        // Stale `.meta` files whose asset is gone (Unity deletes them too).
        Self::remove_orphan_metas(&assets, 0);
        self.guid_to_path = guid_to_path;
        self.path_to_guid = path_to_guid;
        self.scanned = true;
        self.save_cache();
        report
    }

    fn remove_orphan_metas(dir: &Path, depth: usize) {
        if depth > 32 {
            return;
        }
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                Self::remove_orphan_metas(&p, depth + 1);
            } else if let Some(asset) = name.strip_suffix(".meta")
                && !asset.is_empty()
                && !dir.join(asset).exists()
            {
                let _ = std::fs::remove_file(&p);
            }
        }
    }

    fn save_cache(&self) {
        let map: BTreeMap<&String, &String> = self.guid_to_path.iter().collect();
        if let Ok(text) = serde_yaml_ng::to_string(&map) {
            let f = self.cache_file();
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(f, text);
        }
    }

    /// Makes sure an asset just created by the engine has a GUID; returns it.
    pub fn ensure(&mut self, path: &str) -> Option<String> {
        let path = file_of(path);
        if !path.starts_with("Assets/") || !self.root.join(path).exists() {
            return None;
        }
        if let Some(g) = self.path_to_guid.get(path) {
            return Some(g.clone());
        }
        let meta = self.root.join(format!("{path}.meta"));
        let g = match Self::read_guid(&meta) {
            Some(g) if !self.guid_to_path.contains_key(&g) => g,
            _ => {
                let g = new_guid();
                Self::write_guid(&meta, &g).ok()?;
                g
            }
        };
        self.guid_to_path.insert(g.clone(), path.to_owned());
        self.path_to_guid.insert(path.to_owned(), g.clone());
        Some(g)
    }

    /// Records a move done by the editor (the `.meta` went with it): `old` and everything
    /// under it now live at `new`.
    pub fn moved(&mut self, old: &str, new: &str) {
        let affected: Vec<(String, String)> = self
            .path_to_guid
            .iter()
            .filter(|(p, _)| *p == old || p.starts_with(&format!("{old}/")))
            .map(|(p, g)| (p.clone(), g.clone()))
            .collect();
        for (p, g) in affected {
            let np = format!("{new}{}", &p[old.len()..]);
            self.path_to_guid.remove(&p);
            self.path_to_guid.insert(np.clone(), g.clone());
            self.guid_to_path.insert(g, np);
        }
        self.save_cache();
    }

    /// Records a delete done by the editor.
    pub fn deleted(&mut self, path: &str) {
        let gone: Vec<String> = self
            .path_to_guid
            .keys()
            .filter(|p| *p == path || p.starts_with(&format!("{path}/")))
            .cloned()
            .collect();
        for p in gone {
            if let Some(g) = self.path_to_guid.remove(&p) {
                self.guid_to_path.remove(&g);
            }
        }
        self.save_cache();
    }

    /// The current path of a referenced asset: `path` if it exists, otherwise where its
    /// GUID (from the referencing file's table) lives now. Keeps a `#sub-asset` suffix.
    pub fn resolve(&self, path: &str, guid: Option<&str>) -> Option<String> {
        let file = file_of(path);
        if self.root.join(file).exists() || !file.starts_with("Assets/") {
            return None;
        }
        let now = self.path_of(guid?)?;
        Some(format!("{now}{}", &path[file.len()..]))
    }

    /// Asset paths a text asset refers to (scene, prefab, material, data assets...): any
    /// quoted or bare `Assets/...` string that names an existing asset, and any GUID.
    /// Unity `AssetDatabase.GetDependencies` (direct).
    pub fn direct_dependencies(&self, path: &str) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(self.root.join(file_of(path))) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        let mut push = |p: &str, out: &mut Vec<String>| {
            if p != path && seen.insert(p.to_owned()) {
                out.push(p.to_owned());
            }
        };
        for (i, _) in text.match_indices("Assets/") {
            let rest = &text[i..];
            let end = rest
                .find(['"', '\'', '\n', '\r', '#', ',', ']', '}', '|'])
                .unwrap_or(rest.len());
            let cand = rest[..end].trim_end();
            if self.path_to_guid.contains_key(cand) {
                push(cand, &mut out);
            }
        }
        for w in text.split(|c: char| !c.is_ascii_hexdigit()) {
            if w.len() == 32
                && let Some(p) = self.guid_to_path.get(w)
            {
                push(p, &mut out);
            }
        }
        out
    }

    /// Unity `AssetDatabase.GetDependencies(path, recursive)`.
    pub fn dependencies(&self, path: &str, recursive: bool) -> Vec<String> {
        if !recursive {
            return self.direct_dependencies(path);
        }
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::from([path.to_owned()]);
        let mut todo = vec![path.to_owned()];
        while let Some(p) = todo.pop() {
            for d in self.direct_dependencies(&p) {
                if seen.insert(d.clone()) {
                    out.push(d.clone());
                    todo.push(d);
                }
            }
        }
        out.sort();
        out
    }

    /// Assets that refer to `path` (Unity's "Find References In Scene/Project").
    pub fn dependents(&self, path: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .path_to_guid
            .keys()
            .filter(|p| *p != path && self.direct_dependencies(p).iter().any(|d| d == path))
            .cloned()
            .collect();
        out.sort();
        out
    }

    /// Unity `AssetDatabase.FindAssets`: `filter` words match file names (case-insensitive);
    /// `t:Type` matches the extension's type name given by `type_of`; `folders` limits the
    /// search. Returns paths.
    pub fn find_assets(
        &self,
        filter: &str,
        folders: &[String],
        type_of: &dyn Fn(&str) -> Option<String>,
    ) -> Vec<String> {
        let mut types = Vec::new();
        let mut words = Vec::new();
        for tok in filter.split_whitespace() {
            match tok.strip_prefix("t:") {
                Some(t) => types.push(t.to_ascii_lowercase()),
                None => words.push(tok.to_ascii_lowercase()),
            }
        }
        let mut out: Vec<String> = self
            .path_to_guid
            .keys()
            .filter(|p| {
                folders.is_empty()
                    || folders
                        .iter()
                        .any(|f| p.starts_with(&format!("{}/", f.trim_end_matches('/'))))
            })
            .filter(|p| {
                let name = p.rsplit('/').next().unwrap_or(p).to_ascii_lowercase();
                words.iter().all(|w| name.contains(w.as_str()))
            })
            .filter(|p| {
                types.is_empty()
                    || type_of(p).is_some_and(|t| types.iter().any(|x| t.eq_ignore_ascii_case(x)))
            })
            .cloned()
            .collect();
        out.sort();
        out
    }

    /// GUIDs of the asset files `paths` refer to, for a file's reference table.
    pub fn guid_table<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a str>,
    ) -> BTreeMap<String, String> {
        paths
            .into_iter()
            .filter_map(|p| {
                let f = file_of(p);
                self.guid_of(f).map(|g| (f.to_owned(), g.to_owned()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oxide_adb_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets/Materials")).unwrap();
        std::fs::write(
            dir.join("Assets/Materials/Red.mat"),
            "color: [1, 0, 0, 1]\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("Assets/Main.scene"),
            "objects:\n- material: !Asset Assets/Materials/Red.mat\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn scan_writes_guids_and_keeps_meta_fields() {
        let dir = project("scan");
        std::fs::write(dir.join("Assets/Main.scene.meta"), "labels:\n- Level\n").unwrap();
        let mut db = AssetDatabase::new(&dir);
        let r = db.refresh();
        assert_eq!(r.imported.len(), 3, "{r:?}"); // folder + 2 files
        let g = db.guid_of("Assets/Materials/Red.mat").unwrap().to_owned();
        assert_eq!(g.len(), 32);
        assert_eq!(db.path_of(&g), Some("Assets/Materials/Red.mat"));
        let meta = std::fs::read_to_string(dir.join("Assets/Main.scene.meta")).unwrap();
        assert!(
            meta.starts_with("guid:") && meta.contains("Level"),
            "{meta}"
        );
        // A second scan finds nothing new and keeps the GUIDs.
        let mut db2 = AssetDatabase::new(&dir);
        let r = db2.refresh();
        assert!(r.imported.is_empty() && r.moved.is_empty(), "{r:?}");
        assert_eq!(db2.guid_of("Assets/Materials/Red.mat"), Some(g.as_str()));
    }

    #[test]
    fn outside_moves_are_detected_and_references_resolve_by_guid() {
        let dir = project("move");
        let mut db = AssetDatabase::new(&dir);
        db.refresh();
        let g = db.guid_of("Assets/Materials/Red.mat").unwrap().to_owned();
        // Moved in Finder, with its .meta.
        std::fs::create_dir_all(dir.join("Assets/Art")).unwrap();
        std::fs::rename(
            dir.join("Assets/Materials/Red.mat"),
            dir.join("Assets/Art/Crimson.mat"),
        )
        .unwrap();
        std::fs::rename(
            dir.join("Assets/Materials/Red.mat.meta"),
            dir.join("Assets/Art/Crimson.mat.meta"),
        )
        .unwrap();
        let mut db = AssetDatabase::new(&dir);
        let r = db.refresh();
        assert_eq!(
            r.moved,
            vec![(
                "Assets/Materials/Red.mat".into(),
                "Assets/Art/Crimson.mat".into()
            )]
        );
        assert_eq!(
            db.resolve("Assets/Materials/Red.mat", Some(&g)).as_deref(),
            Some("Assets/Art/Crimson.mat")
        );
        assert_eq!(db.resolve("Assets/Art/Crimson.mat", Some(&g)), None);
    }

    #[test]
    fn copied_meta_gets_a_new_guid_and_orphans_are_removed() {
        let dir = project("copy");
        let mut db = AssetDatabase::new(&dir);
        db.refresh();
        std::fs::copy(
            dir.join("Assets/Materials/Red.mat"),
            dir.join("Assets/Materials/Red 1.mat"),
        )
        .unwrap();
        std::fs::copy(
            dir.join("Assets/Materials/Red.mat.meta"),
            dir.join("Assets/Materials/Red 1.mat.meta"),
        )
        .unwrap();
        std::fs::write(
            dir.join("Assets/Gone.png.meta"),
            "guid: 0123456789abcdef0123456789abcdef\n",
        )
        .unwrap();
        let r = db.refresh();
        assert_eq!(r.new_guids, vec!["Assets/Materials/Red 1.mat".to_owned()]);
        assert_ne!(
            db.guid_of("Assets/Materials/Red.mat"),
            db.guid_of("Assets/Materials/Red 1.mat")
        );
        assert!(!dir.join("Assets/Gone.png.meta").exists());
    }

    #[test]
    fn dependencies_and_find_assets() {
        let dir = project("deps");
        let mut db = AssetDatabase::new(&dir);
        db.refresh();
        assert_eq!(
            db.direct_dependencies("Assets/Main.scene"),
            vec!["Assets/Materials/Red.mat"]
        );
        assert_eq!(
            db.dependents("Assets/Materials/Red.mat"),
            vec!["Assets/Main.scene"]
        );
        let ty = |p: &str| p.ends_with(".mat").then(|| "Material".to_owned());
        assert_eq!(
            db.find_assets("t:Material", &[], &ty),
            vec!["Assets/Materials/Red.mat"]
        );
        assert_eq!(db.find_assets("main", &[], &ty), vec!["Assets/Main.scene"]);
        assert!(
            db.find_assets("red", &["Assets/Other".into()], &ty)
                .is_empty()
        );
    }
}
