//! A WGSL shader (`.wgsl`) used by Full Screen Pass renderer features (custom post
//! effects), or the compiled form of a Shader Graph (surface hooks for the Lit shader).
//! <https://docs.unity3d.com/Manual/urp/renderer-features/renderer-feature-full-screen-pass.html>

use crate::handle::Asset;
use crate::handle::Handle;
use crate::texture::Texture2D;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shader {
    pub name: String,
    /// WGSL source (for a Shader Graph: the `vertex_mod` / `surface` hooks).
    pub source: String,
    /// Set for a compiled Shader Graph: how materials using it draw and bind.
    pub surface: Option<SurfaceShader>,
}

impl Asset for Shader {
    const TYPE_NAME: &'static str = "Shader";
}

/// One exposed property of a Shader Graph: a uniform slot (vec4) or a texture slot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderProperty {
    /// Reference name (`_BaseColor`) materials store values under.
    pub name: String,
    pub display_name: String,
    /// `Float`, `Vector2`, `Vector3`, `Vector4`, `Color`, `Boolean`, `Texture2D`.
    pub kind: String,
    /// Uniform slot (vec4 index) for values; texture slot (0..8) for textures.
    pub slot: u32,
    pub default: [f32; 4],
    pub default_texture: Option<Handle<Texture2D>>,
    /// Slider range for floats (min, max), if any.
    pub range: Option<(f32, f32)>,
}

/// Render state and layout of a compiled Shader Graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceShader {
    pub unlit: bool,
    pub transparent: bool,
    pub alpha_clip: bool,
    pub double_sided: bool,
    pub cast_shadows: bool,
    pub properties: Vec<ShaderProperty>,
    /// Hooks for the shadow pass (vertex changes and alpha clipping), if they differ from
    /// the built-in material's.
    pub shadow_source: Option<String>,
    /// Bumped on every compile (pipelines rebuild).
    pub generation: u64,
    /// What the shader was compiled from (the graph's and its Sub Graphs' versions).
    pub stamp: u64,
    /// Sub Graphs the graph uses.
    pub dependencies: Vec<String>,
    /// Compile errors by node id (0: the graph).
    pub errors: Vec<(u32, String)>,
}

/// Marker for Shader Graph asset handles on materials (`.shadergraph` files; the graph
/// itself lives in `oxide_shadergraph`, its compiled shader in `AssetServer::shaders`
/// under the same path).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderGraphRef;

impl Asset for ShaderGraphRef {
    const TYPE_NAME: &'static str = "ShaderGraph";
}
