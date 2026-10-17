//! Asset packages (`.oxidepackage`), in Unity's `.unitypackage` layout: a gzipped tar with
//! one folder per asset GUID holding `asset` (the file; absent for folders), `asset.meta`
//! and `pathname` (where it goes). GUIDs travel with the assets, so references between them
//! keep working; `.unitypackage` files can be imported the same way (Oxide reads their
//! textures, models, audio and other plain files).
//! Reference: <https://docs.unity3d.com/Manual/AssetPackages.html>

use crate::database::AssetDatabase;
use std::io::{Read, Write};
use std::path::Path;

/// One asset in a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageItem {
    pub guid: String,
    /// Project path it was exported from / is imported to (`Assets/...`).
    pub path: String,
    /// File contents; `None` for a folder.
    pub asset: Option<Vec<u8>>,
    pub meta: Option<Vec<u8>>,
}

/// What importing one item would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportStatus {
    /// A new asset.
    New,
    /// The same asset (GUID) exists at this path and differs: it will be replaced.
    Changed,
    /// The same asset exists with identical contents.
    Unchanged,
    /// The same asset (GUID) exists at another path: it is updated there.
    Moved,
    /// Another asset already uses this path: the item gets a free name next to it.
    Conflict,
}

// ---- tar -----------------------------------------------------------------------------------

fn tar_header(name: &str, size: usize, dir: bool) -> [u8; 512] {
    let mut h = [0u8; 512];
    let name = name.as_bytes();
    // ustar: long names split into prefix (155) + name (100).
    let (prefix, base) = if name.len() > 100 {
        let cut = name[..name.len().min(155)]
            .iter()
            .rposition(|&b| b == b'/')
            .unwrap_or(0);
        (&name[..cut], &name[cut + 1..])
    } else {
        (&name[..0], name)
    };
    h[..base.len().min(100)].copy_from_slice(&base[..base.len().min(100)]);
    let octal = |field: &mut [u8], v: u64| {
        let s = format!("{:0width$o}\0", v, width = field.len() - 1);
        field.copy_from_slice(s.as_bytes());
    };
    octal(&mut h[100..108], if dir { 0o755 } else { 0o644 });
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size as u64);
    octal(&mut h[136..148], 0);
    h[156] = if dir { b'5' } else { b'0' };
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix);
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| b as u32).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    h
}

fn tar_entry(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    out.extend_from_slice(&tar_header(name, data.len(), false));
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(512) * 512, 0);
}

/// `(name, data)` of every regular file in a tar archive.
fn tar_files(tar: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut long_name: Option<String> = None;
    while pos + 512 <= tar.len() {
        let h = &tar[pos..pos + 512];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let field = |r: std::ops::Range<usize>| {
            let f = &h[r];
            let end = f.iter().position(|&b| b == 0).unwrap_or(f.len());
            String::from_utf8_lossy(&f[..end]).into_owned()
        };
        let size =
            u64::from_str_radix(field(124..136).trim(), 8).map_err(|_| "corrupt package")? as usize;
        let kind = h[156];
        let mut name = field(0..100);
        let prefix = field(345..500);
        if !prefix.is_empty() && &h[257..262] == b"ustar" {
            name = format!("{prefix}/{name}");
        }
        if let Some(n) = long_name.take() {
            name = n;
        }
        let start = pos + 512;
        let end = start
            .checked_add(size)
            .filter(|&e| e <= tar.len())
            .ok_or("corrupt package")?;
        let data = &tar[start..end];
        match kind {
            // GNU long name for the next entry.
            b'L' => {
                long_name = Some(
                    String::from_utf8_lossy(data)
                        .trim_end_matches('\0')
                        .to_owned(),
                )
            }
            b'0' | 0 => out.push((name.trim_start_matches("./").to_owned(), data.to_vec())),
            _ => {}
        }
        pos = start + size.div_ceil(512) * 512;
    }
    Ok(out)
}

// ---- packages ------------------------------------------------------------------------------

/// The items to export for `paths` (files and folders, everything under folders), plus the
/// assets they depend on when `include_dependencies` (Unity's "Include dependencies").
pub fn collect(
    root: &Path,
    db: &AssetDatabase,
    paths: &[String],
    include_dependencies: bool,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let add = |p: &str, out: &mut Vec<String>| {
        if !out.iter().any(|x| x == p) {
            out.push(p.to_owned());
        }
    };
    for p in paths {
        add(p, &mut out);
        if root.join(p).is_dir() {
            let prefix = format!("{p}/");
            for (q, _) in db.all() {
                if q.starts_with(&prefix) {
                    add(q, &mut out);
                }
            }
        }
    }
    if include_dependencies {
        let files: Vec<String> = out.clone();
        for p in files {
            for d in db.dependencies(&p, true) {
                add(&d, &mut out);
            }
        }
    }
    // Parent folders, so folder GUIDs travel too (Unity includes them).
    let mut folders = Vec::new();
    for p in &out {
        let mut cur = p.as_str();
        while let Some((parent, _)) = cur.rsplit_once('/') {
            if parent == "Assets" || parent.is_empty() {
                break;
            }
            folders.push(parent.to_owned());
            cur = parent;
        }
    }
    for f in folders {
        add(&f, &mut out);
    }
    out.sort();
    out
}

/// Reads the given project assets into package items (GUIDs from the AssetDatabase).
pub fn items_for(
    root: &Path,
    db: &mut AssetDatabase,
    paths: &[String],
) -> Result<Vec<PackageItem>, String> {
    let mut items = Vec::new();
    for p in paths {
        let full = root.join(p);
        let guid = db
            .ensure(p)
            .ok_or_else(|| format!("{p} is not an asset under Assets/"))?;
        let asset = if full.is_dir() {
            None
        } else {
            Some(std::fs::read(&full).map_err(|e| format!("{p}: {e}"))?)
        };
        let meta = std::fs::read(root.join(format!("{p}.meta"))).ok();
        items.push(PackageItem {
            guid,
            path: p.clone(),
            asset,
            meta,
        });
    }
    Ok(items)
}

/// Writes a `.oxidepackage` (gzipped tar).
pub fn write(items: &[PackageItem], out: &Path) -> Result<(), String> {
    let mut tar = Vec::new();
    for it in items {
        if let Some(a) = &it.asset {
            tar_entry(&mut tar, &format!("{}/asset", it.guid), a);
        }
        if let Some(m) = &it.meta {
            tar_entry(&mut tar, &format!("{}/asset.meta", it.guid), m);
        }
        tar_entry(
            &mut tar,
            &format!("{}/pathname", it.guid),
            it.path.as_bytes(),
        );
    }
    tar.extend_from_slice(&[0u8; 1024]);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar).map_err(|e| e.to_string())?;
    let bytes = gz.finish().map_err(|e| e.to_string())?;
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(out, bytes).map_err(|e| format!("cannot write {}: {e}", out.display()))
}

/// Reads a `.oxidepackage` or `.unitypackage`.
pub fn read(path: &Path) -> Result<Vec<PackageItem>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(&bytes[..])
        .read_to_end(&mut tar)
        .map_err(|e| format!("{}: not a package ({e})", path.display()))?;
    let mut by_guid: std::collections::BTreeMap<String, PackageItem> = Default::default();
    for (name, data) in tar_files(&tar)? {
        let Some((guid, file)) = name.split_once('/') else {
            continue;
        };
        let it = by_guid
            .entry(guid.to_owned())
            .or_insert_with(|| PackageItem {
                guid: guid.to_owned(),
                path: String::new(),
                asset: None,
                meta: None,
            });
        match file {
            "asset" => it.asset = Some(data),
            "asset.meta" => it.meta = Some(data),
            // Unity writes the path's first line (later lines hold an old GUID).
            "pathname" => {
                it.path = String::from_utf8_lossy(&data)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .replace('\\', "/")
            }
            _ => {}
        }
    }
    let mut items: Vec<PackageItem> = by_guid
        .into_values()
        .filter(|i| i.path.starts_with("Assets/") && !i.path.contains(".."))
        .collect();
    items.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(items)
}

/// What importing `item` into the project would do.
pub fn status(root: &Path, db: &AssetDatabase, item: &PackageItem) -> ImportStatus {
    // Folders merge with an existing folder at the same path (whatever its GUID).
    if item.asset.is_none() && root.join(&item.path).is_dir() {
        return ImportStatus::Unchanged;
    }
    match db.path_of(&item.guid) {
        Some(p) if p == item.path => {
            let same = match &item.asset {
                Some(a) => std::fs::read(root.join(p)).is_ok_and(|b| &b == a),
                None => true,
            };
            if same {
                ImportStatus::Unchanged
            } else {
                ImportStatus::Changed
            }
        }
        Some(_) => ImportStatus::Moved,
        None if root.join(&item.path).exists() => ImportStatus::Conflict,
        None => ImportStatus::New,
    }
}

/// Imports the items (Unity's Import Package window → Import). An asset whose GUID is
/// already in the project is updated where it is; a path taken by another asset gets a
/// free name. The `.meta` (with the GUID) is written with each asset. Returns the paths
/// written; the caller refreshes the AssetDatabase.
pub fn import(
    root: &Path,
    db: &AssetDatabase,
    items: &[PackageItem],
) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for it in items {
        let dest = match status(root, db, it) {
            ImportStatus::Unchanged => continue,
            ImportStatus::Moved => db.path_of(&it.guid).unwrap_or(&it.path).to_owned(),
            ImportStatus::Conflict => free_path(root, &it.path),
            _ => it.path.clone(),
        };
        let full = root.join(&dest);
        match &it.asset {
            None => std::fs::create_dir_all(&full).map_err(|e| format!("{dest}: {e}"))?,
            Some(bytes) => {
                if let Some(d) = full.parent() {
                    std::fs::create_dir_all(d).map_err(|e| format!("{dest}: {e}"))?;
                }
                std::fs::write(&full, bytes).map_err(|e| format!("{dest}: {e}"))?;
            }
        }
        // The GUID line is what matters; Unity's own meta fields are kept as they are.
        let meta = match &it.meta {
            Some(m) => m.clone(),
            None => format!("guid: {}\n", it.guid).into_bytes(),
        };
        std::fs::write(root.join(format!("{dest}.meta")), meta)
            .map_err(|e| format!("{dest}.meta: {e}"))?;
        written.push(dest);
    }
    Ok(written)
}

fn free_path(root: &Path, rel: &str) -> String {
    if !root.join(rel).exists() {
        return rel.to_owned();
    }
    let (stem, ext) = match rel.rsplit_once('.') {
        Some((s, e)) if !s.ends_with('/') => (s.to_owned(), format!(".{e}")),
        _ => (rel.to_owned(), String::new()),
    };
    (1..)
        .map(|n| format!("{stem} {n}{ext}"))
        .find(|p| !root.join(p).exists())
        .expect("unbounded")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("oxide_pkg_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets/Art/Materials")).unwrap();
        dir
    }

    #[test]
    fn export_then_import_keeps_guids_and_dependencies() {
        let src = project("src");
        std::fs::write(src.join("Assets/Art/rock.png"), b"PNGDATA").unwrap();
        std::fs::write(
            src.join("Assets/Art/Materials/Rock.mat"),
            "fields:\n  base_map: !Asset Assets/Art/rock.png\n",
        )
        .unwrap();
        std::fs::write(src.join("Assets/Other.txt"), "unrelated").unwrap();
        let mut db = AssetDatabase::new(&src);
        db.refresh();
        let paths = collect(&src, &db, &["Assets/Art/Materials/Rock.mat".into()], true);
        assert_eq!(
            paths,
            vec![
                "Assets/Art",
                "Assets/Art/Materials",
                "Assets/Art/Materials/Rock.mat",
                "Assets/Art/rock.png"
            ]
        );
        let items = items_for(&src, &mut db, &paths).unwrap();
        let pkg = src.join("rock.oxidepackage");
        write(&items, &pkg).unwrap();

        let back = read(&pkg).unwrap();
        assert_eq!(back, items);

        // Into another project: new; then again: unchanged.
        let dst = project("dst");
        let mut db2 = AssetDatabase::new(&dst);
        db2.refresh();
        assert!(
            back.iter()
                .filter(|i| i.asset.is_some())
                .all(|i| status(&dst, &db2, i) == ImportStatus::New)
        );
        let written = import(&dst, &db2, &back).unwrap();
        assert!(written.contains(&"Assets/Art/rock.png".to_owned()));
        let mut db2 = AssetDatabase::new(&dst);
        db2.refresh();
        assert_eq!(
            db2.guid_of("Assets/Art/rock.png"),
            db.guid_of("Assets/Art/rock.png")
        );
        assert!(
            back.iter()
                .all(|i| status(&dst, &db2, i) == ImportStatus::Unchanged)
        );
        // A different asset already at a path: imported under a free name.
        std::fs::remove_file(dst.join("Assets/Art/rock.png.meta")).unwrap();
        std::fs::write(dst.join("Assets/Art/rock.png"), b"MINE").unwrap();
        let mut db3 = AssetDatabase::new(&dst);
        db3.refresh();
        let rock = back
            .iter()
            .find(|i| i.path == "Assets/Art/rock.png")
            .unwrap();
        assert_eq!(status(&dst, &db3, rock), ImportStatus::Conflict);
        let written = import(&dst, &db3, std::slice::from_ref(rock)).unwrap();
        assert_eq!(written, vec!["Assets/Art/rock 1.png".to_owned()]);
    }

    #[test]
    fn long_paths_survive_the_tar() {
        let long = format!("{}/asset", "a".repeat(40) + &"/b".repeat(60));
        let mut tar = Vec::new();
        tar_entry(&mut tar, &long, b"x");
        tar.extend_from_slice(&[0u8; 1024]);
        let files = tar_files(&tar).unwrap();
        assert_eq!(files, vec![(long, b"x".to_vec())]);
    }
}
