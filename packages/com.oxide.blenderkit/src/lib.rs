//! BlenderKit for Oxide: search <https://www.blenderkit.com> from the editor and import its
//! models (glTF), materials (textures read out of the `.blend`) and HDRIs (skyboxes).
//! Installed through the Package Manager (Built-in → BlenderKit); the project's Scripts
//! library calls [`register`] because `package.json` names it under
//! `"oxide": { "editorExtensions" }`.

pub mod api;
pub mod blend;
pub mod import;
pub mod settings;
mod window;

pub use window::BlenderKitWindow;

/// Registers the BlenderKit window: Window → BlenderKit, docked next to the Inspector and
/// opened the first time a project has the package.
pub fn register(ext: &mut oxide::EditorExtensions) {
    ext.window::<BlenderKitWindow>("BlenderKit", "Window/BlenderKit")
        .dock(oxide::WindowDock::Inspector)
        .open_by_default();
}
