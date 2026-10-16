//! Game scripts: the project's `Scripts` crate, compiled in the background and hot-reloaded
//! into the running editor (Unity's script compilation + domain reload).
//!
//! The project is a Cargo workspace whose members are `Scripts` (a Rust `dylib`) and the
//! project's editor executable; both link the engine library `oxide_dylib`, so a freshly
//! built Scripts library can be loaded into the editor process and share its types. On a
//! reload, script components are taken out with their field values, the old types are
//! unregistered, the new library is loaded and registered, and the components are put back
//! in the same slots (objects, physics and everything else stay as they are). Old libraries
//! stay loaded: unloading code that may still be referenced is not worth the risk.
//!
//! References: <https://docs.unity3d.com/Manual/script-compilation.html>,
//! <https://docs.unity3d.com/Manual/domain-reloading.html>

use oxide_assets::AssetServer;
use oxide_core::{debug_log, debug_log_error, debug_log_warning};
use oxide_engine::Engine;
use oxide_scene::ComponentRegistry;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Instant, SystemTime};

/// Rust crate (and library) name of a project's scripts.
pub const SCRIPTS_CRATE: &str = "scripts";

#[derive(Clone, Debug, PartialEq)]
pub enum ScriptState {
    /// No `Scripts/Cargo.toml` in the project.
    None,
    Compiling,
    /// Last build failed; the previous scripts (if any) are still running.
    Failed,
    Ready,
}

struct BuildOutcome {
    ok: bool,
    artifact: Option<PathBuf>,
    /// (is_error, rendered message)
    messages: Vec<(bool, String)>,
    seconds: f32,
}

pub struct ScriptHost {
    root: PathBuf,
    /// Engine library build the editor runs on (scripts must match it).
    build_id: &'static str,
    signature: u64,
    last_check: Instant,
    job: Option<Receiver<BuildOutcome>>,
    pub state: ScriptState,
    /// Compile errors of the last build.
    pub errors: Vec<String>,
    /// Component types the loaded library registered.
    pub script_types: Vec<String>,
    /// Data asset types the loaded library registered (`scripts! { ...; assets: ... }`).
    pub script_asset_types: Vec<String>,
    libraries: Vec<libloading::Library>,
    generation: u32,
    pub last_build_seconds: f32,
}

/// Changes when any script source (or the Scripts manifest) changes.
fn source_signature(scripts: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut stack = vec![scripts.join("src")];
    let mut files = vec![scripts.join("Cargo.toml")];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                files.push(p);
            }
        }
    }
    files.sort();
    for f in files {
        f.hash(&mut h);
        if let Ok(m) = std::fs::metadata(&f) {
            m.len().hash(&mut h);
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .hash(&mut h);
        }
    }
    h.finish()
}

fn run_cargo(root: PathBuf) -> BuildOutcome {
    let started = Instant::now();
    let out = std::process::Command::new(crate::project_files::cargo_command())
        .args(["build", "-p", SCRIPTS_CRATE, "--message-format=json"])
        .current_dir(&root)
        .env_remove("CARGO_TARGET_DIR")
        .output();
    let mut result = BuildOutcome {
        ok: false,
        artifact: None,
        messages: Vec::new(),
        seconds: 0.0,
    };
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            result
                .messages
                .push((true, format!("could not run cargo: {e}")));
            return result;
        }
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v["reason"].as_str() {
            Some("compiler-message") => {
                let level = v["message"]["level"].as_str().unwrap_or("");
                if let Some(text) = v["message"]["rendered"].as_str()
                    && matches!(level, "error" | "warning")
                {
                    result
                        .messages
                        .push((level == "error", text.trim_end().to_owned()));
                }
            }
            Some("compiler-artifact") if v["target"]["name"] == SCRIPTS_CRATE => {
                result.artifact = v["filenames"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|f| f.as_str())
                    .find(|f| f.ends_with(std::env::consts::DLL_SUFFIX))
                    .map(PathBuf::from);
            }
            _ => {}
        }
    }
    result.ok = out.status.success();
    if !result.ok && !result.messages.iter().any(|m| m.0) {
        let err = String::from_utf8_lossy(&out.stderr);
        result.messages.push((true, err.trim().to_owned()));
    }
    result.seconds = started.elapsed().as_secs_f32();
    result
}

impl ScriptHost {
    /// `root` is the project folder (with `Scripts/`); `build_id` identifies the engine
    /// library this editor runs on.
    pub fn new(root: PathBuf, build_id: &'static str) -> Self {
        let state = if root.join("Scripts/Cargo.toml").is_file() {
            ScriptState::Ready
        } else {
            ScriptState::None
        };
        Self {
            root,
            build_id,
            signature: 0,
            last_check: Instant::now() - std::time::Duration::from_secs(10),
            job: None,
            state,
            errors: Vec::new(),
            script_types: Vec::new(),
            script_asset_types: Vec::new(),
            libraries: Vec::new(),
            generation: 0,
            last_build_seconds: 0.0,
        }
    }

    pub fn scripts_dir(&self) -> PathBuf {
        self.root.join("Scripts")
    }

    pub fn is_compiling(&self) -> bool {
        self.job.is_some()
    }

    /// Starts a build now (even if nothing changed).
    pub fn recompile(&mut self) {
        if self.job.is_some() || self.state == ScriptState::None {
            return;
        }
        self.signature = source_signature(&self.scripts_dir());
        let (tx, rx) = channel();
        let root = self.root.clone();
        std::thread::spawn(move || {
            let _ = tx.send(run_cargo(root));
        });
        self.job = Some(rx);
        self.state = ScriptState::Compiling;
    }

    /// Called every editor frame: starts a build when sources changed and swaps in the
    /// result when a build finishes.
    pub fn poll(&mut self, engine: &mut Engine) {
        if self.state == ScriptState::None {
            return;
        }
        if let Some(rx) = &self.job {
            let Ok(outcome) = rx.try_recv() else { return };
            self.job = None;
            self.finish(engine, outcome);
            return;
        }
        if self.last_check.elapsed().as_secs_f32() < 0.5 {
            return;
        }
        self.last_check = Instant::now();
        if source_signature(&self.scripts_dir()) != self.signature {
            debug_log!("Compiling scripts...");
            self.recompile();
        }
    }

    fn finish(&mut self, engine: &mut Engine, outcome: BuildOutcome) {
        self.last_build_seconds = outcome.seconds;
        self.errors.clear();
        for (is_error, text) in &outcome.messages {
            if *is_error {
                debug_log_error!("{text}");
                self.errors.push(text.clone());
            } else {
                debug_log_warning!("{text}");
            }
        }
        let artifact = outcome.artifact.filter(|_| outcome.ok);
        let Some(artifact) = artifact else {
            debug_log_error!(
                "Scripts have compile errors ({}); the previous scripts keep running",
                self.errors.len().max(1)
            );
            self.state = ScriptState::Failed;
            return;
        };
        match self.load(engine, &artifact) {
            Ok(n) => {
                self.state = ScriptState::Ready;
                debug_log!(
                    "Scripts reloaded: {n} component type(s), compiled in {:.1}s",
                    outcome.seconds
                );
            }
            Err(e) => {
                self.state = ScriptState::Failed;
                self.errors.push(e.clone());
                debug_log_error!("{e}");
            }
        }
    }

    /// Loads a built Scripts library and swaps it in.
    fn load(&mut self, engine: &mut Engine, built: &Path) -> Result<usize, String> {
        // Load a uniquely named copy: platforms cache libraries by path.
        let dir = self.root.join("Library/ScriptAssemblies");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        self.generation += 1;
        let copy = dir.join(format!(
            "{}{SCRIPTS_CRATE}-{}-{}{}",
            std::env::consts::DLL_PREFIX,
            std::process::id(),
            self.generation,
            std::env::consts::DLL_SUFFIX
        ));
        std::fs::copy(built, &copy).map_err(|e| format!("cannot copy {}: {e}", built.display()))?;
        // SAFETY: the library was built from this project's Scripts against the same engine
        // library (checked below) by the same compiler; its entry points are Rust functions
        // with the signatures `scripts!` generates.
        let lib = unsafe { libloading::Library::new(&copy) }
            .map_err(|e| format!("cannot load scripts: {e}"))?;
        let build_id: libloading::Symbol<fn() -> &'static str> = unsafe {
            lib.get(b"oxide_scripts_build_id")
        }
        .map_err(|_| "Scripts/src/lib.rs has no `oxide::scripts! { ... }` block".to_string())?;
        if build_id() != self.build_id {
            return Err(format!(
                "scripts were built against another engine build ({} vs {}); restart the editor",
                build_id(),
                self.build_id
            ));
        }
        let register: libloading::Symbol<fn(&mut ComponentRegistry)> =
            unsafe { lib.get(b"oxide_register_scripts") }.map_err(|e| e.to_string())?;

        // Older Scripts libraries have no custom Inspectors or data asset types.
        let register_editors: Option<libloading::Symbol<fn(&mut oxide_engine::CustomEditors)>> =
            unsafe { lib.get(b"oxide_register_editors") }.ok();
        let register_assets: Option<libloading::Symbol<fn(&mut AssetServer)>> =
            unsafe { lib.get(b"oxide_register_assets") }.ok();

        let w = &mut engine.world;
        let old = std::mem::take(&mut self.script_types);
        if let Some(e) = w.resources.get_mut::<oxide_engine::CustomEditors>() {
            e.remove(&old);
        }
        let taken = w.take_components_for_reload(|t| old.iter().any(|o| o == t));
        w.registry
            .unregister_where(|r| old.iter().any(|o| o == r.info.name));
        w.resources.remove_crate(SCRIPTS_CRATE);
        let before: std::collections::HashSet<String> =
            w.registry.iter().map(|r| r.info.name.to_owned()).collect();
        register(&mut w.registry);
        self.script_types = w
            .registry
            .iter()
            .map(|r| r.info.name.to_owned())
            .filter(|n| !before.contains(n))
            .collect();
        if let Some(f) = register_editors {
            if !w.resources.contains::<oxide_engine::CustomEditors>() {
                w.resources.insert(oxide_engine::CustomEditors::default());
            }
            if let Some(e) = w.resources.get_mut::<oxide_engine::CustomEditors>() {
                f(e);
            }
        }
        // Data asset types: drop the old library's (and their loaded values), register the
        // new ones, then reload those assets from their files.
        let assets = w.resource_mut::<AssetServer>();
        let mut reload = Vec::new();
        for t in std::mem::take(&mut self.script_asset_types) {
            assets.types.unregister(&t);
            reload.extend(assets.data.remove_type(&t));
        }
        if let Some(f) = register_assets {
            let before: std::collections::HashSet<String> =
                assets.types.iter().map(|t| t.type_name.to_owned()).collect();
            f(assets);
            self.script_asset_types = assets
                .types
                .iter()
                .map(|t| t.type_name.to_owned())
                .filter(|n| !before.contains(n))
                .collect();
        }
        for p in reload {
            if let Err(e) = assets.load_data(&p) {
                debug_log_warning!("{e}");
            }
        }
        let missing = w.restore_components_after_reload(taken);
        for m in missing {
            debug_log_warning!("Script `{m}` no longer exists; its components keep their data");
        }
        self.libraries.push(lib);
        Ok(self.script_types.len())
    }

    /// Creates `Scripts/src/<snake>.rs` from the template and declares it in `lib.rs`
    /// (Unity's Create > C# Script). Returns the file path.
    pub fn create_script(&self, type_name: &str) -> Result<PathBuf, String> {
        crate::project_files::add_script(&self.scripts_dir(), type_name)
    }
}
