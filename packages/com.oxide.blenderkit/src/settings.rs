//! Where the package keeps its settings. The API key is per user, outside every project (so
//! it is never committed or shipped): `BlenderKit.json` in Oxide's settings folder, readable
//! only by you. `BLENDERKIT_API_KEY` overrides it. The project gets a stable id for
//! BlenderKit's download statistics (`UserSettings/BlenderKit.json`, like the Blender add-on's
//! per-scene id).

use oxide::assets::deps::serde_json::{self, Value, json};
use std::path::{Path, PathBuf};

/// The user's BlenderKit settings file.
pub fn user_file() -> PathBuf {
    oxide::editor::prefs::config_dir().join("BlenderKit.json")
}

fn read_json(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

/// The saved API key (`BLENDERKIT_API_KEY` wins), or "".
pub fn load_api_key(file: &Path) -> String {
    if let Ok(k) = std::env::var("BLENDERKIT_API_KEY")
        && !k.trim().is_empty()
    {
        return k.trim().to_owned();
    }
    read_json(file)
        .get("api_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// Saves (or, when empty, removes) the API key, with owner-only permissions.
pub fn save_api_key(file: &Path, key: &str) -> Result<(), String> {
    let mut v = read_json(file);
    if key.trim().is_empty() {
        if let Some(o) = v.as_object_mut() {
            o.remove("api_key");
        }
    } else {
        v["api_key"] = Value::String(key.trim().to_owned());
    }
    write_json(file, &v)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// This project's id for BlenderKit (made on first use).
pub fn project_uuid(project: &Path) -> String {
    let file = project.join("UserSettings/BlenderKit.json");
    let mut v = read_json(&file);
    if let Some(id) = v.get("scene_uuid").and_then(Value::as_str) {
        return id.to_owned();
    }
    let id = new_uuid(project);
    v["scene_uuid"] = Value::String(id.clone());
    let _ = write_json(&file, &v);
    id
}

/// A random-enough version 4 UUID (time, process and path hashed).
fn new_uuid(seed: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut words = [0u64; 2];
    for (i, w) in words.iter_mut().enumerate() {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (i, seed, std::process::id()).hash(&mut h);
        std::time::SystemTime::now().hash(&mut h);
        *w = h.finish();
    }
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&words[0].to_le_bytes());
    b[8..].copy_from_slice(&words[1].to_le_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
