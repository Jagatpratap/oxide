//! Unity `Application` and `SceneManager` requests from game code. Scripts set them through
//! [`Ctx`](crate::Ctx); the engine applies them at the end of the frame.

/// Requests to the application, and the scenes in the build.
#[derive(Default)]
pub struct Application {
    /// `Application.Quit()`.
    pub quit: bool,
    /// `SceneManager.LoadScene(...)`: a scene path, name or build index (as text).
    pub pending_scene: Option<String>,
    /// Scenes in the build, in Build Settings order.
    pub build_scenes: Vec<String>,
    /// Player settings: Unity `Application.companyName`, `productName` and `version`.
    pub company_name: String,
    pub product_name: String,
    pub version: String,
    /// Unity `Application.isEditor`.
    pub is_editor: bool,
    /// Unity `Application.targetFrameRate`: frames per second the player aims for (0 or
    /// less: as fast as possible / vsync).
    pub target_frame_rate: i32,
    /// Unity `Application.dataPath`: the project's `Assets` folder (editor) or the player's
    /// data folder.
    pub data_path: std::path::PathBuf,
}

/// Unity `RuntimePlatform`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimePlatform {
    OSXEditor,
    WindowsEditor,
    LinuxEditor,
    OSXPlayer,
    WindowsPlayer,
    LinuxPlayer,
    Android,
    IPhonePlayer,
    WebGLPlayer,
}

/// Unity `Screen`: the game's resolution (the Game view in the editor), and resolution
/// requests the player applies at the end of the frame.
#[derive(Clone, Debug, Default)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    /// Window scale × 96 (Unity `Screen.dpi`).
    pub dpi: f32,
    pub full_screen: bool,
    /// `Screen.SetResolution` waiting to be applied: (width, height, full screen).
    pub request: Option<(u32, u32, bool)>,
}

impl Screen {
    /// Unity `Screen.SetResolution(width, height, fullscreen)` (players only).
    pub fn set_resolution(&mut self, width: u32, height: u32, full_screen: bool) {
        self.request = Some((width.max(1), height.max(1), full_screen));
    }
}

impl Application {
    /// Unity `Application.platform`.
    pub fn platform(&self) -> RuntimePlatform {
        use RuntimePlatform as P;
        match (self.is_editor, std::env::consts::OS) {
            (_, "android") => P::Android,
            (_, "ios") => P::IPhonePlayer,
            _ if cfg!(target_arch = "wasm32") => P::WebGLPlayer,
            (true, "macos") => P::OSXEditor,
            (true, "windows") => P::WindowsEditor,
            (true, _) => P::LinuxEditor,
            (false, "macos") => P::OSXPlayer,
            (false, "windows") => P::WindowsPlayer,
            (false, _) => P::LinuxPlayer,
        }
    }

    /// Unity `Application.persistentDataPath`: a per-user folder for saves and settings,
    /// named after the company and product (macOS `~/Library/Application Support/...`,
    /// Windows `%USERPROFILE%/AppData/LocalLow/...`, Linux `~/.config/unity3d/...`).
    pub fn persistent_data_path(&self) -> std::path::PathBuf {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let base = match std::env::consts::OS {
            "macos" => home.join("Library/Application Support"),
            "windows" => home.join("AppData/LocalLow"),
            _ => home.join(".config/unity3d"),
        };
        let clean = |s: &str, fallback: &str| {
            let s: String = s
                .chars()
                .map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c })
                .collect();
            if s.trim().is_empty() {
                fallback.to_owned()
            } else {
                s
            }
        };
        base.join(clean(&self.company_name, "DefaultCompany"))
            .join(clean(&self.product_name, "Oxide Game"))
    }

    /// Finds a scene by build index, path or file name (without `.scene`).
    pub fn resolve_scene(&self, name: &str, all_scenes: &[String]) -> Option<String> {
        if let Ok(i) = name.parse::<usize>() {
            return self.build_scenes.get(i).cloned();
        }
        if name.ends_with(".scene") {
            return Some(name.to_owned());
        }
        let stem = |p: &String| {
            std::path::Path::new(p)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        };
        self.build_scenes
            .iter()
            .chain(all_scenes)
            .find(|p| stem(p).as_deref() == Some(name))
            .cloned()
    }
}
