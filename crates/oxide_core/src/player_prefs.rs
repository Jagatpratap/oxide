//! Unity's `PlayerPrefs`: small key/value settings kept between sessions, in
//! `PlayerPrefs.json` under the persistent data folder the engine sets with
//! [`PlayerPrefs::set_path`]. Changes are written by [`PlayerPrefs::save`], which the engine
//! also calls when the game quits.
//! <https://docs.unity3d.com/ScriptReference/PlayerPrefs.html>

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum Pref {
    Int(i64),
    Float(f64),
    String(String),
}

#[derive(Default)]
struct Store {
    path: Option<PathBuf>,
    values: BTreeMap<String, Pref>,
    loaded: bool,
    dirty: bool,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let store = guard.get_or_insert_with(Store::default);
    if !store.loaded {
        store.loaded = true;
        if let Some(text) = store
            .path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
        {
            store.values = serde_json::from_str(&text).unwrap_or_default();
        }
    }
    f(store)
}

/// Unity `PlayerPrefs` (static API).
pub struct PlayerPrefs;

impl PlayerPrefs {
    /// Where the prefs live (`<persistent data path>/PlayerPrefs.json`); reloads them.
    pub fn set_path(path: impl Into<PathBuf>) {
        let mut guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(Store {
            path: Some(path.into()),
            ..Default::default()
        });
    }

    pub fn set_int(key: &str, value: i32) {
        with(|s| {
            s.values.insert(key.to_owned(), Pref::Int(value as i64));
            s.dirty = true;
        });
    }

    pub fn set_float(key: &str, value: f32) {
        with(|s| {
            s.values.insert(key.to_owned(), Pref::Float(value as f64));
            s.dirty = true;
        });
    }

    pub fn set_string(key: &str, value: &str) {
        with(|s| {
            s.values
                .insert(key.to_owned(), Pref::String(value.to_owned()));
            s.dirty = true;
        });
    }

    /// Unity `GetInt(key, defaultValue)`.
    pub fn get_int(key: &str, default: i32) -> i32 {
        with(|s| match s.values.get(key) {
            Some(Pref::Int(v)) => *v as i32,
            _ => default,
        })
    }

    pub fn get_float(key: &str, default: f32) -> f32 {
        with(|s| match s.values.get(key) {
            Some(Pref::Float(v)) => *v as f32,
            Some(Pref::Int(v)) => *v as f32,
            _ => default,
        })
    }

    pub fn get_string(key: &str, default: &str) -> String {
        with(|s| match s.values.get(key) {
            Some(Pref::String(v)) => v.clone(),
            _ => default.to_owned(),
        })
    }

    pub fn has_key(key: &str) -> bool {
        with(|s| s.values.contains_key(key))
    }

    pub fn delete_key(key: &str) {
        with(|s| {
            if s.values.remove(key).is_some() {
                s.dirty = true;
            }
        });
    }

    pub fn delete_all() {
        with(|s| {
            s.values.clear();
            s.dirty = true;
        });
    }

    /// Unity `PlayerPrefs.Save()`: writes changes to disk.
    pub fn save() -> Result<(), String> {
        with(|s| {
            let Some(path) = s.path.clone().filter(|_| s.dirty) else {
                return Ok(());
            };
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let text = serde_json::to_string_pretty(&s.values).map_err(|e| e.to_string())?;
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
            s.dirty = false;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_through_the_file() {
        let path = std::env::temp_dir().join(format!("oxide_prefs_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        PlayerPrefs::set_path(&path);
        assert_eq!(PlayerPrefs::get_int("score", 7), 7);
        PlayerPrefs::set_int("score", 42);
        PlayerPrefs::set_float("volume", 0.5);
        PlayerPrefs::set_string("name", "Ace");
        assert!(PlayerPrefs::has_key("score"));
        PlayerPrefs::save().unwrap();
        // A new session reads them back.
        PlayerPrefs::set_path(&path);
        assert_eq!(PlayerPrefs::get_int("score", 0), 42);
        assert_eq!(PlayerPrefs::get_float("volume", 0.0), 0.5);
        assert_eq!(PlayerPrefs::get_string("name", ""), "Ace");
        // Wrong type: the default.
        assert_eq!(PlayerPrefs::get_int("name", -1), -1);
        PlayerPrefs::delete_key("score");
        assert!(!PlayerPrefs::has_key("score"));
        PlayerPrefs::delete_all();
        PlayerPrefs::save().unwrap();
        PlayerPrefs::set_path(&path);
        assert!(!PlayerPrefs::has_key("name"));
        let _ = std::fs::remove_file(&path);
    }
}
