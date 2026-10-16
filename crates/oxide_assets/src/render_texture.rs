//! Unity `RenderTexture`: a texture a camera renders into (Camera `target_texture`), usable
//! anywhere a texture is (material maps, UI images). Saved as a `.renderTexture` file.
//! <https://docs.unity3d.com/Manual/class-RenderTexture.html>

use crate::handle::Asset;
use oxide_core::Reflect;

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct RenderTexture {
    #[reflect(skip)]
    pub name: String,
    pub width: u32,
    pub height: u32,
}

impl Default for RenderTexture {
    fn default() -> Self {
        Self {
            name: "RenderTexture".into(),
            width: 256,
            height: 256,
        }
    }
}

impl Asset for RenderTexture {
    const TYPE_NAME: &'static str = "RenderTexture";
}
