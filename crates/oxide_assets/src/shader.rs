//! A WGSL shader file (`.wgsl`) used by Full Screen Pass renderer features (custom post
//! effects). <https://docs.unity3d.com/Manual/urp/renderer-features/renderer-feature-full-screen-pass.html>

use crate::handle::Asset;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shader {
    pub name: String,
    /// WGSL source.
    pub source: String,
}

impl Asset for Shader {
    const TYPE_NAME: &'static str = "Shader";
}
