//! Unity's Package Manager, project side: `Packages/manifest.json` lists the project's
//! packages — registry versions (`"1.2.0"`), local folders (`"file:../MyPackage"`), git
//! repositories (`"https://host/repo.git#v1.0"`) — and folders in `Packages/` with a
//! `package.json` are embedded packages. Resolution follows each package's own dependencies,
//! clones git packages and unpacks registry tarballs into `Library/PackageCache`, and writes
//! `Packages/packages-lock.json`. A package's files are assets at `Packages/<name>/...`.
//! Reference: <https://docs.unity3d.com/Manual/Packages.html>

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a package comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageSource {
    /// A folder inside `Packages/`.
    Embedded,
    /// A folder elsewhere (`file:` in the manifest).
    Local(PathBuf),
    /// A git repository (cloned into the package cache), at a revision.
    Git {
        url: String,
        revision: Option<String>,
    },
    /// A registry version (unpacked into the package cache).
    Registry { registry: String },
}

impl PackageSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Embedded => "Embedded",
            Self::Local(_) => "Local",
            Self::Git { .. } => "Git",
            Self::Registry { .. } => "Registry",
        }
    }
}

/// A package sample (copied into `Assets/Samples` on import).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageSample {
    pub display_name: String,
    pub description: String,
    /// Folder inside the package.
    pub path: String,
}

/// A resolved package (`package.json` plus where it is).
#[derive(Clone, Debug, PartialEq)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub author: String,
    pub keywords: Vec<String>,
    pub dependencies: BTreeMap<String, String>,
    pub samples: Vec<PackageSample>,
    pub source: PackageSource,
    /// The package's folder on disk.
    pub root: PathBuf,
    /// Listed in the manifest (else pulled in by another package).
    pub direct: bool,
    /// A Rust crate in the package (`Cargo.toml` at its root): (crate name, folder).
    pub rust_crate: Option<(String, PathBuf)>,
}

/// A scoped registry: packages whose names start with one of `scopes` come from `url`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScopedRegistry {
    pub name: String,
    pub url: String,
    pub scopes: Vec<String>,
}

/// `Packages/manifest.json`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manifest {
    pub dependencies: BTreeMap<String, String>,
    pub scoped_registries: Vec<ScopedRegistry>,
    /// The default registry (npm protocol).
    pub registry: Option<String>,
}

impl Manifest {
    pub fn path(project: &Path) -> PathBuf {
        project.join("Packages/manifest.json")
    }

    pub fn load(project: &Path) -> Result<Self, String> {
        let p = Self::path(project);
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(_) => return Ok(Self::default()),
        };
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        let mut m = Self::default();
        if let Some(d) = v.get("dependencies").and_then(Value::as_object) {
            for (k, v) in d {
                m.dependencies
                    .insert(k.clone(), v.as_str().unwrap_or_default().to_owned());
            }
        }
        m.registry = v.get("registry").and_then(Value::as_str).map(str::to_owned);
        if let Some(rs) = v.get("scopedRegistries").and_then(Value::as_array) {
            for r in rs {
                m.scoped_registries.push(ScopedRegistry {
                    name: r.get("name").and_then(Value::as_str).unwrap_or("").into(),
                    url: r.get("url").and_then(Value::as_str).unwrap_or("").into(),
                    scopes: r
                        .get("scopes")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default(),
                });
            }
        }
        Ok(m)
    }

    pub fn save(&self, project: &Path) -> Result<(), String> {
        let mut root = Map::new();
        root.insert(
            "dependencies".into(),
            Value::Object(
                self.dependencies
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect(),
            ),
        );
        if let Some(r) = &self.registry {
            root.insert("registry".into(), Value::String(r.clone()));
        }
        if !self.scoped_registries.is_empty() {
            root.insert(
                "scopedRegistries".into(),
                Value::Array(
                    self.scoped_registries
                        .iter()
                        .map(|r| {
                            serde_json::json!({ "name": r.name, "url": r.url, "scopes": r.scopes })
                        })
                        .collect(),
                ),
            );
        }
        let p = Self::path(project);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
        std::fs::write(&p, text + "\n").map_err(|e| format!("{}: {e}", p.display()))
    }

    /// The registry for `name` (a scoped one when a scope matches).
    pub fn registry_for(&self, name: &str) -> Option<String> {
        self.scoped_registries
            .iter()
            .filter(|r| {
                r.scopes
                    .iter()
                    .any(|s| name == s || name.starts_with(&format!("{s}.")))
            })
            .map(|r| r.url.clone())
            .next()
            .or_else(|| self.registry.clone())
    }
}

/// Reads a package's `package.json`.
pub fn read_package(root: &Path, source: PackageSource) -> Result<PackageInfo, String> {
    let p = root.join("package.json");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let name = s("name");
    if name.is_empty() {
        return Err(format!("{}: no \"name\"", p.display()));
    }
    let author = match v.get("author") {
        Some(Value::String(a)) => a.clone(),
        Some(Value::Object(o)) => o
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        _ => String::new(),
    };
    let rust_crate = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|t| crate_name(&t))
        .map(|n| (n, root.to_path_buf()));
    Ok(PackageInfo {
        display_name: {
            let d = s("displayName");
            if d.is_empty() { name.clone() } else { d }
        },
        version: s("version"),
        description: s("description"),
        author,
        keywords: v
            .get("keywords")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|k| k.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        dependencies: v
            .get("dependencies")
            .and_then(Value::as_object)
            .map(|d| {
                d.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned()))
                    .collect()
            })
            .unwrap_or_default(),
        samples: v
            .get("samples")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|x| PackageSample {
                        display_name: x
                            .get("displayName")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .into(),
                        description: x
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .into(),
                        path: x.get("path").and_then(Value::as_str).unwrap_or("").into(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        name,
        source,
        root: root.to_path_buf(),
        direct: false,
        rust_crate,
    })
}

/// The `[package] name` of a Cargo.toml.
fn crate_name(toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_package = l == "[package]";
            continue;
        }
        if in_package
            && let Some(rest) = l.strip_prefix("name")
            && let Some(v) = rest.trim().strip_prefix('=')
        {
            return Some(v.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// Compares dotted versions numerically (`1.10.0` > `1.9.3`).
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<u64> {
        v.split(['.', '-', '+'])
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parts(a).cmp(&parts(b))
}

/// How a dependency is given in a manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DependencySpec {
    Version(String),
    File(String),
    Git {
        url: String,
        revision: Option<String>,
    },
}

pub fn parse_spec(spec: &str) -> DependencySpec {
    if let Some(p) = spec.strip_prefix("file:") {
        return DependencySpec::File(p.to_owned());
    }
    let is_git = spec.ends_with(".git")
        || spec.contains(".git#")
        || spec.starts_with("git+")
        || spec.starts_with("git@")
        || (spec.starts_with("https://") && spec.contains("github.com"));
    if is_git {
        let s = spec.trim_start_matches("git+");
        let (url, rev) = match s.split_once('#') {
            Some((u, r)) => (u.to_owned(), Some(r.to_owned())),
            None => (s.to_owned(), None),
        };
        return DependencySpec::Git { url, revision: rev };
    }
    DependencySpec::Version(spec.to_owned())
}

/// The package cache folder of a git package or registry version.
pub fn cache_dir(project: &Path, name: &str, key: &str) -> PathBuf {
    let safe: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    project
        .join("Library/PackageCache")
        .join(format!("{name}@{safe}"))
}

/// Fetches what a package needs on disk: git clones (and checkouts), registry tarballs.
pub trait Fetcher {
    /// Clones `url` at `revision` into `dir` (which doesn't exist yet).
    fn git(&mut self, url: &str, revision: Option<&str>, dir: &Path) -> Result<(), String>;
    /// Unpacks registry `name@version` from `registry` into `dir`.
    fn registry(
        &mut self,
        registry: &str,
        name: &str,
        version: &str,
        dir: &Path,
    ) -> Result<(), String>;
}

/// The git command line (no registry access).
pub struct GitFetcher;

impl Fetcher for GitFetcher {
    fn git(&mut self, url: &str, revision: Option<&str>, dir: &Path) -> Result<(), String> {
        git_clone(url, revision, dir)
    }
    fn registry(
        &mut self,
        registry: &str,
        name: &str,
        version: &str,
        _dir: &Path,
    ) -> Result<(), String> {
        Err(format!(
            "{name}@{version}: registry {registry} isn't reachable here"
        ))
    }
}

/// git plus npm-protocol registries over HTTP (what the editor uses).
pub struct NetFetcher;

impl Fetcher for NetFetcher {
    fn git(&mut self, url: &str, revision: Option<&str>, dir: &Path) -> Result<(), String> {
        git_clone(url, revision, dir)
    }
    fn registry(
        &mut self,
        registry: &str,
        name: &str,
        version: &str,
        dir: &Path,
    ) -> Result<(), String> {
        let doc = http_get(&format!("{}/{name}", registry.trim_end_matches('/')))?;
        let v: Value = serde_json::from_slice(&doc).map_err(|e| format!("{name}: {e}"))?;
        let tarball = v
            .pointer(&format!(
                "/versions/{}/dist/tarball",
                version.replace('/', "~1")
            ))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{name}@{version}: not in {registry}"))?;
        unpack_tarball(&http_get(tarball)?, dir)
    }
}

/// The versions a registry has of `name` (newest first) and its latest `package.json`.
pub fn registry_versions(registry: &str, name: &str) -> Result<(Vec<String>, Value), String> {
    let doc = http_get(&format!("{}/{name}", registry.trim_end_matches('/')))?;
    let v: Value = serde_json::from_slice(&doc).map_err(|e| format!("{name}: {e}"))?;
    let mut versions: Vec<String> = v
        .get("versions")
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    versions.sort_by(|a, b| compare_versions(b, a));
    let latest = versions
        .first()
        .and_then(|l| v.get("versions").and_then(|vs| vs.get(l)).cloned())
        .unwrap_or(Value::Null);
    Ok((versions, latest))
}

fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let mut resp = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    resp.body_mut()
        .with_config()
        .limit(512 << 20)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))
}

/// `git clone` and checkout.
pub fn git_clone(url: &str, revision: Option<&str>, dir: &Path) -> Result<(), String> {
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let run = |args: &[&str], cwd: Option<&Path>| -> Result<(), String> {
        let mut c = std::process::Command::new("git");
        c.args(args);
        if let Some(d) = cwd {
            c.current_dir(d);
        }
        let out = c.output().map_err(|e| format!("git: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    };
    let d = dir.to_string_lossy().to_string();
    run(&["clone", "--quiet", url, &d], None)?;
    if let Some(r) = revision {
        run(&["checkout", "--quiet", r], Some(dir))?;
    }
    Ok(())
}

/// Unpacks an npm-style package tarball (`package/...` entries) into `dir`.
pub fn unpack_tarball(tgz: &[u8], dir: &Path) -> Result<(), String> {
    let gz = flate2::read::GzDecoder::new(tgz);
    let mut ar = tar::Archive::new(gz);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for entry in ar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        // npm tarballs put everything under one top folder (usually `package/`).
        let rel: PathBuf = path.components().skip(1).collect();
        if rel.as_os_str().is_empty()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        let out = dir.join(&rel);
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        if entry.header().entry_type().is_file() {
            entry.unpack(&out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Resolves the project's packages: embedded ones, the manifest's, and their dependencies
/// (the highest version asked for wins). Fetches what is missing with `fetch`. Returns the
/// packages and the problems found.
pub fn resolve(project: &Path, fetch: &mut dyn Fetcher) -> (Vec<PackageInfo>, Vec<String>) {
    let mut out: BTreeMap<String, PackageInfo> = BTreeMap::new();
    let mut errors = Vec::new();
    let packages_dir = project.join("Packages");
    // Embedded packages win over everything.
    if let Ok(rd) = std::fs::read_dir(&packages_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.join("package.json").is_file() {
                match read_package(&p, PackageSource::Embedded) {
                    Ok(mut info) => {
                        info.direct = true;
                        out.insert(info.name.clone(), info);
                    }
                    Err(e) => errors.push(e),
                }
            }
        }
    }
    let manifest = match Manifest::load(project) {
        Ok(m) => m,
        Err(e) => {
            errors.push(e);
            Manifest::default()
        }
    };
    // (name, spec, direct)
    let mut queue: Vec<(String, String, bool)> = manifest
        .dependencies
        .iter()
        .map(|(k, v)| (k.clone(), v.clone(), true))
        .collect();
    // Embedded packages' own dependencies.
    for p in out.values() {
        queue.extend(
            p.dependencies
                .iter()
                .map(|(k, v)| (k.clone(), v.clone(), false)),
        );
    }
    let mut seen = 0;
    while let Some((name, spec, direct)) = queue.pop() {
        seen += 1;
        if seen > 10_000 {
            break;
        }
        if let Some(have) = out.get_mut(&name) {
            have.direct |= direct;
            // A higher registry version asked for by another package replaces a lower one.
            match (&have.source, parse_spec(&spec)) {
                (PackageSource::Registry { .. }, DependencySpec::Version(v))
                    if compare_versions(&v, &have.version).is_gt() && !have.direct => {}
                _ => continue,
            }
        }
        let resolved = match parse_spec(&spec) {
            DependencySpec::File(rel) => {
                let p = packages_dir.join(&rel);
                let p = p.canonicalize().unwrap_or(p);
                read_package(&p, PackageSource::Local(p.clone()))
            }
            DependencySpec::Git { url, revision } => {
                let key = revision.clone().unwrap_or_else(|| git_key(&url));
                let dir = cache_dir(project, &name, &key);
                let fetched = if dir.join("package.json").is_file() {
                    Ok(())
                } else {
                    let _ = std::fs::remove_dir_all(&dir);
                    fetch.git(&url, revision.as_deref(), &dir)
                };
                fetched.and_then(|()| read_package(&dir, PackageSource::Git { url, revision }))
            }
            DependencySpec::Version(v) => {
                let dir = cache_dir(project, &name, &v);
                let registry = manifest.registry_for(&name).unwrap_or_default();
                let fetched = if dir.join("package.json").is_file() {
                    Ok(())
                } else if registry.is_empty() {
                    Err(format!("{name}@{v}: no registry in the manifest"))
                } else {
                    let _ = std::fs::remove_dir_all(&dir);
                    fetch.registry(&registry, &name, &v, &dir)
                };
                fetched.and_then(|()| read_package(&dir, PackageSource::Registry { registry }))
            }
        };
        match resolved {
            Ok(mut info) => {
                if info.name != name {
                    errors.push(format!("{name}: its package.json says \"{}\"", info.name));
                }
                info.direct = direct;
                for (d, s) in &info.dependencies {
                    queue.push((d.clone(), s.clone(), false));
                }
                out.insert(name, info);
            }
            Err(e) => errors.push(e),
        }
    }
    let list: Vec<PackageInfo> = out.into_values().collect();
    if let Err(e) = write_lock(project, &list) {
        errors.push(e);
    }
    (list, errors)
}

/// `Packages/packages-lock.json`: what each dependency resolved to.
fn write_lock(project: &Path, packages: &[PackageInfo]) -> Result<(), String> {
    if !project.join("Packages").is_dir() {
        return Ok(());
    }
    let deps: Map<String, Value> = packages
        .iter()
        .map(|p| {
            let source = match &p.source {
                PackageSource::Embedded => "embedded".to_owned(),
                PackageSource::Local(_) => "local".into(),
                PackageSource::Git { .. } => "git".into(),
                PackageSource::Registry { .. } => "registry".into(),
            };
            let mut v = serde_json::json!({
                "version": p.version,
                "depth": if p.direct { 0 } else { 1 },
                "source": source,
                "dependencies": p.dependencies,
            });
            if let PackageSource::Git { url, revision } = &p.source {
                v["url"] = Value::String(url.clone());
                if let Some(r) = revision {
                    v["hash"] = Value::String(r.clone());
                }
            }
            (p.name.clone(), v)
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::json!({ "dependencies": deps }))
        .map_err(|e| e.to_string())?;
    std::fs::write(project.join("Packages/packages-lock.json"), text + "\n")
        .map_err(|e| e.to_string())
}

/// Adds (or updates) the packages' Rust crates as dependencies of the project's Scripts
/// crate, in a block the Package Manager owns.
pub fn sync_script_dependencies(project: &Path, packages: &[PackageInfo]) -> Result<bool, String> {
    let cargo = project.join("Scripts/Cargo.toml");
    let Ok(text) = std::fs::read_to_string(&cargo) else {
        return Ok(false);
    };
    const BEGIN: &str = "# --- packages (managed by the Package Manager) ---";
    const END: &str = "# --- end packages ---";
    let mut lines: Vec<String> = Vec::new();
    for p in packages {
        if let Some((name, dir)) = &p.rust_crate {
            let rel = pathdiff(dir, &project.join("Scripts"));
            lines.push(format!(
                "{name} = {{ path = \"{}\" }}",
                rel.replace('\\', "/")
            ));
        }
    }
    // Remove the old block.
    let mut body = String::new();
    let mut skipping = false;
    for l in text.lines() {
        if l.trim() == BEGIN {
            skipping = true;
            continue;
        }
        if l.trim() == END {
            skipping = false;
            continue;
        }
        if !skipping {
            body.push_str(l);
            body.push('\n');
        }
    }
    let new = if lines.is_empty() {
        body
    } else {
        let block = format!("{BEGIN}\n{}\n{END}\n", lines.join("\n"));
        // Into the [dependencies] table (after its header), creating it if needed.
        match body.find("[dependencies]\n") {
            Some(i) => {
                let at = i + "[dependencies]\n".len();
                format!("{}{}{}", &body[..at], block, &body[at..])
            }
            None => format!("{body}\n[dependencies]\n{block}"),
        }
    };
    if new != text {
        std::fs::write(&cargo, new).map_err(|e| e.to_string())?;
        return Ok(true);
    }
    Ok(false)
}

/// `path` relative to `base` (both absolute or both relative).
fn pathdiff(path: &Path, base: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    out.to_string_lossy().into_owned()
}

/// Copies a package sample into `Assets/Samples/<package display name>/<version>/<sample>`.
pub fn import_sample(
    project: &Path,
    package: &PackageInfo,
    sample: &PackageSample,
) -> Result<String, String> {
    let src = package.root.join(&sample.path);
    if !src.is_dir() {
        return Err(format!(
            "{}: no sample folder {}",
            package.name, sample.path
        ));
    }
    let rel = format!(
        "Assets/Samples/{}/{}/{}",
        package.display_name, package.version, sample.display_name
    );
    let dst = project.join(&rel);
    copy_dir(&src, &dst)?;
    Ok(rel)
}

/// What to add with the Package Manager's + menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddRequest {
    /// A folder with a `package.json` ("Add package from disk").
    Disk(PathBuf),
    /// A git URL, optionally `#revision` ("Add package from git URL").
    Git(String),
    /// A registry name and version, or no version for the latest ("Add package by name").
    Name {
        name: String,
        version: Option<String>,
    },
}

/// Adds a package to the manifest; returns its name. Disk folders are stored as `file:`
/// paths relative to `Packages/`; a name without a version asks the registry for the latest.
pub fn add(
    project: &Path,
    request: &AddRequest,
    fetch: &mut dyn Fetcher,
) -> Result<String, String> {
    let mut m = Manifest::load(project)?;
    let (name, spec) = match request {
        AddRequest::Disk(dir) => {
            let dir = if dir.is_absolute() {
                dir.clone()
            } else {
                project.join(dir)
            };
            let info = read_package(&dir, PackageSource::Local(dir.clone()))?;
            let rel = pathdiff(&dir, &project.join("Packages"));
            (info.name, format!("file:{}", rel.replace('\\', "/")))
        }
        AddRequest::Git(url) => {
            // The name is in the repository's package.json: clone to find it.
            let DependencySpec::Git { url: u, revision } = parse_spec(url) else {
                return Err(format!("{url} isn't a git URL"));
            };
            let tmp = project.join("Library/PackageCache/.add");
            let _ = std::fs::remove_dir_all(&tmp);
            fetch.git(&u, revision.as_deref(), &tmp)?;
            let info = read_package(
                &tmp,
                PackageSource::Git {
                    url: u.clone(),
                    revision: revision.clone(),
                },
            );
            let key = revision.clone().unwrap_or_else(|| git_key(&u));
            let cached = info.as_ref().map(|i| cache_dir(project, &i.name, &key));
            match (&info, cached) {
                (Ok(_), Ok(dir)) if !dir.join("package.json").is_file() => {
                    let _ = std::fs::remove_dir_all(&dir);
                    if let Some(parent) = dir.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::rename(&tmp, &dir).map_err(|e| e.to_string())?;
                }
                _ => {
                    let _ = std::fs::remove_dir_all(&tmp);
                }
            }
            (info?.name, url.trim_start_matches("git+").to_owned())
        }
        AddRequest::Name { name, version } => {
            let registry = m
                .registry_for(name)
                .ok_or_else(|| format!("{name}: no registry in the manifest"))?;
            let v = match version {
                Some(v) if !v.is_empty() => v.clone(),
                _ => registry_versions(&registry, name)?
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| format!("{name}: no versions in {registry}"))?,
            };
            (name.clone(), v)
        }
    };
    m.dependencies.insert(name.clone(), spec);
    m.save(project)?;
    Ok(name)
}

/// Removes a package from the manifest (an embedded package is a folder: it stays).
pub fn remove(project: &Path, name: &str) -> Result<bool, String> {
    let mut m = Manifest::load(project)?;
    let had = m.dependencies.remove(name).is_some();
    if had {
        m.save(project)?;
    }
    Ok(had)
}

fn git_key(url: &str) -> String {
    format!(
        "{:x}",
        url.bytes()
            .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64))
    )
}

/// `Packages/<name>` folders by package name, for [`crate::AssetServer::package_roots`].
pub fn roots(packages: &[PackageInfo]) -> BTreeMap<String, PathBuf> {
    packages
        .iter()
        .map(|p| (p.name.clone(), p.root.clone()))
        .collect()
}

/// Resolves the project's packages (if it has any) into `assets.package_roots`; returns
/// the problems found.
pub fn load_into(assets: &mut crate::AssetServer, fetch: &mut dyn Fetcher) -> Vec<String> {
    let project = assets.root.clone();
    if !project.join("Packages").is_dir() {
        return Vec::new();
    }
    let (list, errors) = resolve(&project, fetch);
    assets.package_roots = roots(&list);
    errors
}

/// Copies a package's files for a player build (no `~` folders, `.git` or build output).
pub fn copy_package(package: &PackageInfo, dst: &Path) -> Result<(), String> {
    copy_filtered(&package.root, dst, true)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    copy_filtered(src, dst, false)
}

fn copy_filtered(src: &Path, dst: &Path, filter: bool) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let p = e.path();
        let to = dst.join(e.file_name());
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if filter && (name.ends_with('~') || name.starts_with('.') || name == "target") {
                continue;
            }
            copy_filtered(&p, &to, filter)?;
        } else {
            std::fs::copy(&p, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    struct FakeFetch {
        calls: Vec<String>,
        tarball: Vec<u8>,
    }

    impl Fetcher for FakeFetch {
        fn git(&mut self, url: &str, revision: Option<&str>, dir: &Path) -> Result<(), String> {
            self.calls.push(format!("git {url} {revision:?}"));
            write(
                &dir.join("package.json"),
                r#"{"name":"com.acme.git","version":"0.3.0","dependencies":{"com.acme.reg":"1.1.0"}}"#,
            );
            Ok(())
        }
        fn registry(
            &mut self,
            registry: &str,
            name: &str,
            version: &str,
            dir: &Path,
        ) -> Result<(), String> {
            self.calls.push(format!("reg {registry} {name}@{version}"));
            unpack_tarball(&self.tarball, dir)
        }
    }

    fn tarball(name: &str, version: &str) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let json = format!(r#"{{"name":"{name}","version":"{version}","displayName":"Reg"}}"#);
        let mut h = tar::Header::new_gnu();
        h.set_size(json.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        builder
            .append_data(&mut h, "package/package.json", json.as_bytes())
            .unwrap();
        let tar = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn resolves_embedded_local_git_and_registry_packages() {
        let root = std::env::temp_dir().join(format!("oxide-packages-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("Game");
        // Embedded.
        write(
            &project.join("Packages/com.acme.embedded/package.json"),
            r#"{"name":"com.acme.embedded","version":"1.0.0","displayName":"Embedded Tools",
               "dependencies":{"com.acme.git":"https://github.com/acme/git.git#v0.3"},
               "samples":[{"displayName":"Demo","description":"A demo","path":"Samples~/Demo"}]}"#,
        );
        write(
            &project.join("Packages/com.acme.embedded/Samples~/Demo/readme.txt"),
            "hi",
        );
        // Local, with a Rust crate.
        write(
            &root.join("LocalPkg/package.json"),
            r#"{"name":"com.acme.local","version":"2.0.0","dependencies":{"com.acme.git":"https://github.com/acme/git.git#v0.3"}}"#,
        );
        write(
            &root.join("LocalPkg/Cargo.toml"),
            "[package]\nname = \"acme_local\"\nversion = \"0.1.0\"\n",
        );
        let mut m = Manifest {
            registry: Some("https://registry.example".into()),
            ..Default::default()
        };
        m.dependencies
            .insert("com.acme.local".into(), "file:../../LocalPkg".into());
        m.save(&project).unwrap();
        let back = Manifest::load(&project).unwrap();
        assert_eq!(back, m);
        let mut fetch = FakeFetch {
            calls: vec![],
            tarball: tarball("com.acme.reg", "1.1.0"),
        };
        let (pkgs, errors) = resolve(&project, &mut fetch);
        assert!(errors.is_empty(), "{errors:?}");
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "com.acme.embedded",
                "com.acme.git",
                "com.acme.local",
                "com.acme.reg"
            ]
        );
        let get = |n: &str| pkgs.iter().find(|p| p.name == n).unwrap();
        assert_eq!(get("com.acme.embedded").source, PackageSource::Embedded);
        assert!(get("com.acme.local").direct && !get("com.acme.git").direct);
        assert_eq!(get("com.acme.reg").display_name, "Reg");
        assert_eq!(
            get("com.acme.git").source,
            PackageSource::Git {
                url: "https://github.com/acme/git.git".into(),
                revision: Some("v0.3".into())
            }
        );
        assert_eq!(
            get("com.acme.local").rust_crate.as_ref().unwrap().0,
            "acme_local"
        );
        assert!(
            get("com.acme.git")
                .root
                .starts_with(project.join("Library/PackageCache"))
        );
        assert_eq!(fetch.calls.len(), 2);
        // Cached: resolving again fetches nothing.
        let (_, errors) = resolve(&project, &mut fetch);
        assert!(errors.is_empty());
        assert_eq!(fetch.calls.len(), 2);
        let lock = std::fs::read_to_string(project.join("Packages/packages-lock.json")).unwrap();
        assert!(lock.contains("\"com.acme.reg\"") && lock.contains("\"git\""));
        // Adding from disk and by git URL, then removing.
        let name = add(
            &project,
            &AddRequest::Disk(root.join("LocalPkg")),
            &mut fetch,
        )
        .unwrap();
        assert_eq!(name, "com.acme.local");
        assert_eq!(
            Manifest::load(&project).unwrap().dependencies["com.acme.local"],
            "file:../../LocalPkg"
        );
        let name = add(
            &project,
            &AddRequest::Git("https://github.com/acme/other.git#v1".into()),
            &mut fetch,
        )
        .unwrap();
        assert_eq!(name, "com.acme.git");
        assert!(
            cache_dir(&project, "com.acme.git", "v1")
                .join("package.json")
                .is_file()
        );
        assert!(remove(&project, "com.acme.git").unwrap());
        assert!(!remove(&project, "com.acme.git").unwrap());
        // Samples copy into Assets/Samples.
        let rel = import_sample(
            &project,
            get("com.acme.embedded"),
            &get("com.acme.embedded").samples[0],
        )
        .unwrap();
        assert_eq!(rel, "Assets/Samples/Embedded Tools/1.0.0/Demo");
        assert!(project.join(&rel).join("readme.txt").is_file());
        // Script crates go into the Scripts crate's dependencies.
        write(
            &project.join("Scripts/Cargo.toml"),
            "[package]\nname = \"scripts\"\n\n[dependencies]\noxide = \"0.1\"\n",
        );
        assert!(sync_script_dependencies(&project, &pkgs).unwrap());
        let cargo = std::fs::read_to_string(project.join("Scripts/Cargo.toml")).unwrap();
        assert!(
            cargo.contains("acme_local = { path = \"../../LocalPkg\" }"),
            "{cargo}"
        );
        assert!(cargo.contains("oxide = \"0.1\""));
        assert!(
            !sync_script_dependencies(&project, &pkgs).unwrap(),
            "unchanged"
        );
        // Removing the package removes its crate.
        assert!(sync_script_dependencies(&project, &[]).unwrap());
        assert!(
            !std::fs::read_to_string(project.join("Scripts/Cargo.toml"))
                .unwrap()
                .contains("acme_local")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn specs_and_versions() {
        assert_eq!(parse_spec("1.2.3"), DependencySpec::Version("1.2.3".into()));
        assert_eq!(parse_spec("file:../x"), DependencySpec::File("../x".into()));
        assert_eq!(
            parse_spec("https://github.com/a/b.git#main"),
            DependencySpec::Git {
                url: "https://github.com/a/b.git".into(),
                revision: Some("main".into())
            }
        );
        assert!(compare_versions("1.10.0", "1.9.3").is_gt());
        assert!(compare_versions("2.0.0", "2.0.0").is_eq());
        let m = Manifest {
            registry: Some("https://default".into()),
            scoped_registries: vec![ScopedRegistry {
                name: "Acme".into(),
                url: "https://acme".into(),
                scopes: vec!["com.acme".into()],
            }],
            ..Default::default()
        };
        assert_eq!(
            m.registry_for("com.acme.tools").as_deref(),
            Some("https://acme")
        );
        assert_eq!(
            m.registry_for("com.other").as_deref(),
            Some("https://default")
        );
    }
}
