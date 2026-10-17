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

/// A shader keyword compiled as variants (Unity `LocalKeyword` of a Shader Graph): a WGSL
/// `override` constant chosen when the material's pipeline is made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderKeyword {
    pub reference: String,
    /// Empty for Boolean keywords; the entry names for Enum keywords.
    pub entries: Vec<String>,
    pub default: u32,
    /// Set with `Shader.EnableKeyword` (all materials) instead of per material.
    pub global: bool,
}

impl ShaderKeyword {
    /// The keyword's state given the enabled keyword names: Boolean `REF`, Enum `REF_ENTRY`
    /// (Unity's naming). Materials start with the default states enabled.
    pub fn value(&self, enabled: &dyn Fn(&str) -> bool) -> u32 {
        if self.entries.is_empty() {
            return enabled(&self.reference) as u32;
        }
        self.entries
            .iter()
            .position(|e| enabled(&format!("{}_{}", self.reference, e.to_uppercase())))
            .map_or(self.default, |i| i as u32)
    }
}

impl ShaderProperty {
    /// A Dynamic Branch keyword stored as a value: its keyword (`kind` `Keyword[:entries]`).
    pub fn as_keyword(&self) -> Option<ShaderKeyword> {
        let rest = self.kind.strip_prefix("Keyword")?;
        Some(ShaderKeyword {
            reference: self.name.clone(),
            entries: rest
                .strip_prefix(':')
                .map(|e| e.split('|').map(str::to_owned).collect())
                .unwrap_or_default(),
            default: self.default[0] as u32,
            global: false,
        })
    }
}

/// The keyword names a keyword's state enables: Boolean `REF` (on) or nothing (off), Enum
/// `REF_ENTRY` (Unity's naming).
pub fn keyword_names(reference: &str, entries: &[String], state: u32) -> Option<String> {
    if entries.is_empty() {
        (state != 0).then(|| reference.to_owned())
    } else {
        entries
            .get(state as usize)
            .map(|e| format!("{reference}_{}", e.to_uppercase()))
    }
}

/// The WGSL identifier of a keyword's override constant.
pub fn keyword_constant(reference: &str) -> String {
    let id: String = reference
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("kw_{id}")
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
    /// Keywords compiled as variants (Shader Feature / Multi Compile).
    pub keywords: Vec<ShaderKeyword>,
}

/// Marker for Shader Graph asset handles on materials (`.shadergraph` files; the graph
/// itself lives in `oxide_shadergraph`, its compiled shader in `AssetServer::shaders`
/// under the same path).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderGraphRef;

impl Asset for ShaderGraphRef {
    const TYPE_NAME: &'static str = "ShaderGraph";
}
