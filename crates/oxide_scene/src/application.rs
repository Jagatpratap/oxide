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
}

impl Application {
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
