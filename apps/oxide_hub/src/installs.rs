//! Installed editor versions (Unity Hub's Installs): downloaded releases unpacked into the
//! user's Editors folder, plus folders added with Locate (another unpacked release, or an
//! engine source tree that has been built).
//!
//! ```text
//! <data>/Editors/0.2.0/          an unpacked release download
//!     bin/oxide                  that version's launcher (and editor sandbox)
//!     bin/oxide-editor, lib/     its prebuilt project editor and engine library
//!     crates/ apps/ ...          its engine source (player builds compile it)
//! <config>/Installs.json         folders added with Locate
//! ```

use crate::manifest::{Channel, Download};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const EXE: &str = if cfg!(windows) { "oxide.exe" } else { "oxide" };

/// Per-user settings folder, shared with the editor (`Hub.yaml`, `EditorPrefs.yaml`).
pub fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/Oxide"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Oxide"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(|x| PathBuf::from(x).join("oxide"))
            .or_else(|| home.map(|h| h.join(".config/oxide")))
    };
    base.unwrap_or_else(std::env::temp_dir)
}

/// Where downloaded editors are unpacked. `$OXIDE_EDITORS_DIR` overrides it.
pub fn editors_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("OXIDE_EDITORS_DIR") {
        return d.into();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/Oxide/Editors"))
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join("Oxide/Editors"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(|x| PathBuf::from(x).join("oxide/editors"))
            .or_else(|| home.map(|h| h.join(".local/share/oxide/editors")))
    };
    base.unwrap_or_else(|| std::env::temp_dir().join("oxide-editors"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Install {
    pub version: String,
    /// The engine source folder (`OXIDE_ENGINE_ROOT` for its launcher).
    pub root: PathBuf,
    /// Its `oxide` launcher.
    pub exe: PathBuf,
    /// Downloaded by the Hub (Uninstall deletes it); otherwise added with Locate.
    pub managed: bool,
}

impl Install {
    pub fn channel(&self) -> Channel {
        Channel::of_version(&self.version)
    }

    /// A `Command` running this version's launcher on its own engine source.
    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.exe);
        c.env("OXIDE_ENGINE_ROOT", &self.root)
            .env_remove("CARGO_TARGET_DIR");
        c
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Located {
    root: PathBuf,
    exe: PathBuf,
    version: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct InstallsFile {
    #[serde(default)]
    located: Vec<Located>,
}

fn installs_file() -> PathBuf {
    config_dir().join("Installs.json")
}

fn load_located(path: &Path) -> InstallsFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_located(path: &Path, f: &InstallsFile) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn is_engine(root: &Path) -> bool {
    root.join("crates/oxide_dylib/Cargo.toml").is_file()
}

/// Every install: downloaded ones in `editors` and located ones, newest version first.
pub fn scan(editors: &Path) -> Vec<Install> {
    scan_with(editors, &installs_file())
}

fn scan_with(editors: &Path, located: &Path) -> Vec<Install> {
    let mut out = Vec::new();
    if let Ok(dir) = std::fs::read_dir(editors) {
        for e in dir.flatten() {
            let root = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let exe = root.join("bin").join(EXE);
            if !name.starts_with('.') && exe.is_file() && is_engine(&root) {
                out.push(Install {
                    version: name,
                    root,
                    exe,
                    managed: true,
                });
            }
        }
    }
    for l in load_located(located).located {
        if l.exe.is_file() && is_engine(&l.root) && !out.iter().any(|i| i.root == l.root) {
            out.push(Install {
                version: l.version,
                root: l.root,
                exe: l.exe,
                managed: false,
            });
        }
    }
    out.sort_by(|a, b| crate::manifest::compare_versions(&b.version, &a.version));
    out
}

/// The launcher in a folder picked with Locate: an unpacked release (`bin/oxide`) or an
/// engine source tree built with `cargo build -p oxide` (release build preferred).
pub fn launcher_in(root: &Path) -> Option<PathBuf> {
    if !is_engine(root) {
        return None;
    }
    ["bin", "target/release", "target/debug"]
        .iter()
        .map(|d| root.join(d).join(EXE))
        .find(|p| p.is_file())
}

/// `oxide --version` of a launcher (its first line is `oxide 0.2.0`).
pub fn launcher_version(exe: &Path, root: &Path) -> Result<String, String> {
    let out = Command::new(exe)
        .arg("--version")
        .env("OXIDE_ENGINE_ROOT", root)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", exe.display()))?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("oxide "))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{} didn't report a version", exe.display()))
}

/// Locate: adds an existing install folder.
pub fn locate(root: &Path) -> Result<Install, String> {
    locate_with(root, &installs_file())
}

fn locate_with(root: &Path, file: &Path) -> Result<Install, String> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let exe = launcher_in(&root).ok_or_else(|| {
        format!(
            "{} isn't an Oxide install: it needs bin/{EXE} (a release download) or the engine \
             source built with `cargo build -p oxide`.",
            root.display()
        )
    })?;
    let version = launcher_version(&exe, &root)?;
    let mut f = load_located(file);
    f.located.retain(|l| l.root != root);
    f.located.push(Located {
        root: root.clone(),
        exe: exe.clone(),
        version: version.clone(),
    });
    save_located(file, &f)?;
    Ok(Install {
        version,
        root,
        exe,
        managed: false,
    })
}

/// Uninstall: deletes a downloaded install, or forgets a located one (its files are kept).
pub fn remove(install: &Install) -> Result<(), String> {
    remove_with(install, &installs_file())
}

fn remove_with(install: &Install, file: &Path) -> Result<(), String> {
    if install.managed {
        std::fs::remove_dir_all(&install.root)
            .map_err(|e| format!("cannot delete {}: {e}", install.root.display()))
    } else {
        let mut f = load_located(file);
        f.located.retain(|l| l.root != install.root);
        save_located(file, &f)
    }
}

// ---------------------------------------------------------------------------------------
// Downloading and unpacking.

#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    Downloading { done: u64, total: u64 },
    Verifying,
    Unpacking { files: usize },
    Done(Install),
    Failed(String),
}

/// An install running on its own thread.
pub struct InstallJob {
    pub version: String,
    pub stage: Arc<Mutex<Stage>>,
    cancel: Arc<AtomicBool>,
}

impl InstallJob {
    pub fn start(version: String, download: Download, editors: PathBuf) -> Self {
        let stage = Arc::new(Mutex::new(Stage::Downloading {
            done: 0,
            total: download.size,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (s, c, v) = (stage.clone(), cancel.clone(), version.clone());
        std::thread::spawn(move || {
            let r = install(&v, &download, &editors, &s, &c);
            *s.lock().unwrap() = match r {
                Ok(i) => Stage::Done(i),
                Err(e) => Stage::Failed(e),
            };
        });
        Self {
            version,
            stage,
            cancel,
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn stage(&self) -> Stage {
        self.stage.lock().unwrap().clone()
    }
}

fn install(
    version: &str,
    download: &Download,
    editors: &Path,
    stage: &Mutex<Stage>,
    cancel: &AtomicBool,
) -> Result<Install, String> {
    if version.is_empty() || version.contains(['/', '\\']) || version.starts_with('.') {
        return Err(format!("bad version name `{version}`"));
    }
    std::fs::create_dir_all(editors).map_err(|e| format!("{}: {e}", editors.display()))?;
    let zip = download.url.ends_with(".zip");
    let archive = editors.join(format!(
        ".download-{version}{}",
        if zip { ".zip" } else { ".tar.gz" }
    ));
    let partial = editors.join(format!(".unpack-{version}"));
    let result = (|| {
        fetch_to(&download.url, &archive, download.size, stage, cancel)?;
        *stage.lock().unwrap() = Stage::Verifying;
        let sha = sha256_file(&archive)?;
        if !sha.eq_ignore_ascii_case(download.sha256.trim()) {
            return Err(format!(
                "the download is damaged (SHA-256 {sha}, expected {}); try again",
                download.sha256
            ));
        }
        let _ = std::fs::remove_dir_all(&partial);
        let mut files = 0;
        let mut on_file = || {
            files += 1;
            if files % 64 == 0 {
                *stage.lock().unwrap() = Stage::Unpacking { files };
            }
            !cancel.load(Ordering::Relaxed)
        };
        if zip {
            unpack_zip(&archive, &partial, &mut on_file)?;
        } else {
            unpack_tar_gz(&archive, &partial, &mut on_file)?;
        }
        let root = editors.join(version);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::rename(&partial, &root).map_err(|e| format!("{}: {e}", root.display()))?;
        let exe = root.join("bin").join(EXE);
        if !exe.is_file() || !is_engine(&root) {
            let _ = std::fs::remove_dir_all(&root);
            return Err("the download doesn't contain an Oxide editor".into());
        }
        Ok(Install {
            version: version.to_owned(),
            root,
            exe,
            managed: true,
        })
    })();
    let _ = std::fs::remove_file(&archive);
    let _ = std::fs::remove_dir_all(&partial);
    result
}

fn fetch_to(
    url: &str,
    dest: &Path,
    size: u64,
    stage: &Mutex<Stage>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut reader: Box<dyn Read> = if let Some(path) = url.strip_prefix("file://") {
        Box::new(std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?)
    } else {
        let resp = crate::net::agent()
            .get(url)
            .call()
            .map_err(|e| format!("download failed: {e}"))?;
        Box::new(resp.into_body().into_reader())
    };
    let mut out = std::fs::File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".into());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("{}: {e}", dest.display()))?;
        done += n as u64;
        *stage.lock().unwrap() = Stage::Downloading {
            done,
            total: size.max(done),
        };
    }
    out.flush().map_err(|e| e.to_string())
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `oxide-v0.2.0-<target>/crates/x.rs` → `crates/x.rs`: drops the archive's top folder and
/// refuses paths that would leave the destination.
fn inner_path(path: &Path) -> Option<PathBuf> {
    let mut parts = path.components();
    match parts.next()? {
        Component::Normal(_) => {}
        _ => return None,
    }
    let rest: PathBuf = parts.collect();
    if rest.as_os_str().is_empty() {
        return Some(rest);
    }
    rest.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then_some(rest)
}

fn unpack_tar_gz(
    archive: &Path,
    dest: &Path,
    on_file: &mut dyn FnMut() -> bool,
) -> Result<(), String> {
    let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    tar.set_preserve_permissions(true);
    let bad = |e: std::io::Error| format!("cannot unpack the download: {e}");
    for entry in tar.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        let path = entry.path().map_err(bad)?.into_owned();
        let Some(rel) = inner_path(&path) else {
            return Err(format!("unsafe path in the download: {}", path.display()));
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue; // no links or devices
        }
        let out = dest.join(&rel);
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d).map_err(bad)?;
        }
        entry.unpack(&out).map_err(bad)?;
        if !on_file() {
            return Err("Cancelled.".into());
        }
    }
    Ok(())
}

fn unpack_zip(
    archive: &Path,
    dest: &Path,
    on_file: &mut dyn FnMut() -> bool,
) -> Result<(), String> {
    let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("cannot unpack the download: {e}"))?;
    let bad = |e: std::io::Error| format!("cannot unpack the download: {e}");
    for i in 0..z.len() {
        let mut file = z
            .by_index(i)
            .map_err(|e| format!("cannot unpack the download: {e}"))?;
        let Some(name) = file.enclosed_name() else {
            return Err(format!("unsafe path in the download: {}", file.name()));
        };
        let Some(rel) = inner_path(&name) else {
            return Err(format!("unsafe path in the download: {}", name.display()));
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&rel);
        if file.is_dir() {
            std::fs::create_dir_all(&out).map_err(bad)?;
        } else {
            if let Some(d) = out.parent() {
                std::fs::create_dir_all(d).map_err(bad)?;
            }
            let mut w = std::fs::File::create(&out).map_err(bad)?;
            std::io::copy(&mut file, &mut w).map_err(bad)?;
            #[cfg(unix)]
            if let Some(mode) = file.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
            }
        }
        if !on_file() {
            return Err("Cancelled.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxide-hub-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A fake release archive like scripts/package_release.py writes, with a launcher that
    /// prints `oxide <version>`.
    fn fake_release(dir: &Path, version: &str, extra: Option<(&str, &[u8])>) -> PathBuf {
        let path = dir.join(format!(
            "oxide-v{version}-{}.tar.gz",
            crate::manifest::TARGET
        ));
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut t = tar::Builder::new(gz);
        let top = format!("oxide-v{version}-x");
        let mut add = |name: &str, data: &[u8], mode: u32| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            t.append_data(&mut h, name, data).unwrap();
        };
        let script = format!("#!/bin/sh\necho oxide {version}\n");
        add(&format!("{top}/bin/{EXE}"), script.as_bytes(), 0o755);
        add(
            &format!("{top}/crates/oxide_dylib/Cargo.toml"),
            b"[package]\n",
            0o644,
        );
        if let Some((name, data)) = extra {
            // Written raw, so a `..` path reaches the archive as is.
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            let raw = &mut h.as_old_mut().name;
            raw[..name.len()].copy_from_slice(name.as_bytes());
            h.set_cksum();
            t.append(&h, data).unwrap();
        }
        t.into_inner().unwrap().finish().unwrap();
        path
    }

    fn run(version: &str, archive: &Path, sha: String, editors: &Path) -> Stage {
        let job = InstallJob::start(
            version.into(),
            Download {
                url: format!("file://{}", archive.display()),
                sha256: sha,
                size: std::fs::metadata(archive).unwrap().len(),
            },
            editors.to_owned(),
        );
        loop {
            match job.stage() {
                s @ (Stage::Done(_) | Stage::Failed(_)) => return s,
                _ => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        }
    }

    #[test]
    fn installs_verifies_and_scans() {
        let d = temp("install");
        let editors = d.join("Editors");
        let archive = fake_release(&d, "0.3.0-beta.1", None);
        let sha = sha256_file(&archive).unwrap();

        let bad = run("0.3.0-beta.1", &archive, "00".repeat(32), &editors);
        assert!(
            matches!(&bad, Stage::Failed(e) if e.contains("damaged")),
            "{bad:?}"
        );
        assert!(scan_with(&editors, &d.join("none.json")).is_empty());

        let Stage::Done(i) = run("0.3.0-beta.1", &archive, sha, &editors) else {
            panic!("install failed");
        };
        assert_eq!(i.root, editors.join("0.3.0-beta.1"));
        assert!(i.exe.is_file());
        assert_eq!(i.channel(), Channel::Beta);
        // Nothing left over from the download.
        let names: Vec<_> = std::fs::read_dir(&editors)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["0.3.0-beta.1"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&i.exe).unwrap().permissions().mode();
            assert!(mode & 0o100 != 0, "the launcher stays executable");
            assert_eq!(launcher_version(&i.exe, &i.root).unwrap(), "0.3.0-beta.1");
        }
        let all = scan_with(&editors, &d.join("none.json"));
        assert_eq!(all, std::slice::from_ref(&i));
        remove_with(&i, &d.join("none.json")).unwrap();
        assert!(!i.root.exists());
    }

    #[test]
    fn refuses_paths_outside_the_install() {
        let d = temp("unsafe");
        let archive = fake_release(&d, "0.1.0", Some(("top/../../evil.txt", b"x")));
        let sha = sha256_file(&archive).unwrap();
        let s = run("0.1.0", &archive, sha, &d.join("Editors"));
        assert!(
            matches!(&s, Stage::Failed(e) if e.contains("unsafe")),
            "{s:?}"
        );
        assert!(!d.join("evil.txt").exists());
        assert!(!d.join("Editors/0.1.0").exists());
        assert_eq!(inner_path(Path::new("top/a/b")), Some(PathBuf::from("a/b")));
        assert_eq!(inner_path(Path::new("/abs/a")), None);
    }

    #[cfg(unix)]
    #[test]
    fn locate_adds_and_forgets_a_folder() {
        let d = temp("locate");
        let root = d.join("MyEngine");
        std::fs::create_dir_all(root.join("crates/oxide_dylib")).unwrap();
        std::fs::write(root.join("crates/oxide_dylib/Cargo.toml"), "").unwrap();
        let file = d.join("Installs.json");
        assert!(locate_with(&root, &file).is_err(), "no launcher yet");
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        let exe = root.join("target/debug").join(EXE);
        std::fs::write(&exe, "#!/bin/sh\necho oxide 0.0.7-dev\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let i = locate_with(&root, &file).unwrap();
        assert_eq!(i.version, "0.0.7-dev");
        assert!(!i.managed);
        let all = scan_with(&d.join("Editors"), &file);
        assert_eq!(all.len(), 1);
        remove_with(&i, &file).unwrap();
        assert!(root.is_dir(), "a located folder is kept");
        assert!(scan_with(&d.join("Editors"), &file).is_empty());
    }
}
