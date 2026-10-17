//! Native plugins (Unity's `Assets/Plugins` libraries called through `[DllImport]`): C ABI
//! libraries (`.dylib` / `.dll` / `.so`) anywhere under `Assets/`, found by name, loaded once
//! and kept loaded (like Unity, they can't be unloaded while the editor runs).
//!
//! ```ignore
//! oxide::native_plugin! {
//!     // `unsafe`: the signatures can't be checked against the library.
//!     unsafe "mathlib" {
//!         fn add(a: i32, b: i32) -> i32;
//!         fn lerp(a: f32, b: f32, t: f32) -> f32;
//!     }
//! }
//! let sum = add(2, 3); // loads libmathlib.dylib / mathlib.dll / libmathlib.so on first call
//! ```
//!
//! Each library's Plugin Inspector settings ([`PluginImporter`]) choose the platforms it is
//! included for; player builds leave out the others.
//! <https://docs.unity3d.com/Manual/plug-ins.html>

use oxide_assets::importers::PluginImporter;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

struct Registry {
    /// Folders searched for libraries (the project's / player's `Assets`).
    roots: Vec<PathBuf>,
    loaded: HashMap<String, Arc<libloading::Library>>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| {
        Mutex::new(Registry {
            roots: Vec::new(),
            loaded: HashMap::new(),
        })
    })
}

/// Unity's native plugin loader (static API).
pub struct NativePlugins;

impl NativePlugins {
    /// Where libraries are looked for (the engine sets the project's `Assets` folder).
    pub fn set_search_root(root: impl Into<PathBuf>) {
        let root = root.into();
        let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
        if !r.roots.contains(&root) {
            r.roots.push(root);
        }
    }

    /// File names a plugin called `name` may have on this platform.
    pub fn file_names(name: &str) -> Vec<String> {
        let (prefix, suffix) = (std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
        let mut names = vec![format!("{prefix}{name}{suffix}"), format!("{name}{suffix}")];
        names.dedup();
        names
    }

    /// Finds plugin `name` under the search roots (or `name` as a path).
    pub fn find(name: &str) -> Option<PathBuf> {
        let direct = Path::new(name);
        if direct.is_file() {
            return Some(direct.to_path_buf());
        }
        let wanted = Self::file_names(name);
        let roots = registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .roots
            .clone();
        fn walk(dir: &Path, wanted: &[String], depth: u32) -> Option<PathBuf> {
            let entries = std::fs::read_dir(dir).ok()?;
            let mut dirs = Vec::new();
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| wanted.iter().any(|w| w == n))
                {
                    return Some(p);
                }
            }
            if depth == 0 {
                return None;
            }
            dirs.into_iter().find_map(|d| walk(&d, wanted, depth - 1))
        }
        roots.iter().find_map(|r| walk(r, &wanted, 12))
    }

    /// Loads plugin `name` (once; later calls return the loaded library).
    pub fn load(name: &str) -> Result<Arc<libloading::Library>, String> {
        if let Some(l) = registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .loaded
            .get(name)
        {
            return Ok(l.clone());
        }
        let path = Self::find(name).ok_or_else(|| {
            format!(
                "native plugin `{name}` not found (looked for {} under Assets)",
                Self::file_names(name).join(" / ")
            )
        })?;
        // SAFETY: loading a library runs its initialisers; plugins are trusted project code,
        // like Unity's.
        let lib = unsafe { libloading::Library::new(&path) }
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let lib = Arc::new(lib);
        registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .loaded
            .insert(name.to_owned(), lib.clone());
        Ok(lib)
    }

    /// A function of plugin `name`.
    ///
    /// # Safety
    /// `T` must be the symbol's real type (an `extern "C" fn` with the right signature).
    pub unsafe fn symbol<T: Copy>(name: &str, symbol: &str) -> Result<T, String> {
        let lib = Self::load(name)?;
        // SAFETY: the caller vouches for `T`; the library stays loaded for the program's life
        // (it is never removed from the registry), so the copied pointer stays valid.
        let s: libloading::Symbol<'_, T> =
            unsafe { lib.get(symbol.as_bytes()) }.map_err(|e| format!("{name}: {symbol}: {e}"))?;
        Ok(*s)
    }

    /// Names of the loaded plugins.
    pub fn loaded() -> Vec<String> {
        let mut v: Vec<String> = registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .loaded
            .keys()
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// Loads every plugin marked "Load on startup" that is included here.
    pub fn load_startup_plugins(assets: &oxide_assets::AssetServer, editor: bool) {
        let root = assets.full_path("Assets");
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let Some(rel) = p
                    .strip_prefix(assets.full_path(""))
                    .ok()
                    .and_then(|r| r.to_str())
                    .map(|r| r.replace('\\', "/"))
                else {
                    continue;
                };
                if !oxide_assets::importers::is_native_plugin(&rel) {
                    continue;
                }
                let imp: PluginImporter = assets.importer(&rel);
                if imp.load_on_startup && imp.includes(assets.build_target, editor) {
                    let name = p
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_owned();
                    let name = name
                        .strip_prefix(std::env::consts::DLL_PREFIX)
                        .filter(|_| !std::env::consts::DLL_PREFIX.is_empty())
                        .unwrap_or(&name)
                        .to_owned();
                    if let Err(e) = Self::load(&name) {
                        oxide_core::debug_log_error!("{e}");
                    }
                }
            }
        }
    }
}

/// Unity's `[DllImport]`: Rust wrappers for a native plugin's C functions, bound on first
/// call. The `unsafe` marks that the signatures are trusted.
#[macro_export]
macro_rules! native_plugin {
    (unsafe $lib:literal { $(fn $f:ident($($a:ident : $t:ty),* $(,)?) $(-> $r:ty)?;)* }) => {
        $(
            #[allow(clippy::too_many_arguments)]
            pub fn $f($($a: $t),*) $(-> $r)? {
                type F = extern "C" fn($($t),*) $(-> $r)?;
                static PTR: ::std::sync::OnceLock<F> = ::std::sync::OnceLock::new();
                let f = *PTR.get_or_init(|| {
                    // SAFETY: the `unsafe` in the macro call vouches for the signature.
                    unsafe { $crate::native_plugins::NativePlugins::symbol::<F>($lib, stringify!($f)) }
                        .unwrap_or_else(|e| panic!("DllNotFoundException / EntryPointNotFoundException: {e}"))
                });
                f($($a),*)
            }
        )*
    };
}
