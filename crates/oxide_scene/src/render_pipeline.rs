//! Unity's Scriptable Render Pipeline settings: the Universal Render Pipeline Asset
//! (`.renderpipeline`, URP Asset with its Universal Renderer), chosen in Graphics Settings
//! and overridable per Quality level, and custom render pipelines from scripts.
//! References: <https://docs.unity3d.com/Manual/urp/universalrp-asset.html>,
//! <https://docs.unity3d.com/Manual/urp/urp-universal-renderer.html>,
//! <https://docs.unity3d.com/Manual/srp-creating-render-pipeline-asset-and-render-pipelline-instance.html>

use oxide_assets::Handle;
use oxide_core::Reflect;

/// URP's Universal Renderer "Rendering Path".
#[derive(
    Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub enum RenderingPath {
    Forward,
    #[default]
    ForwardPlus,
    /// G-buffer and a lighting pass for opaque objects (one camera, no MSAA).
    Deferred,
}

/// Unity's MSAA setting on the URP Asset: off, or the Quality level's.
#[derive(
    Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub enum PipelineMsaa {
    /// The active Quality level's MSAA.
    #[default]
    QualityLevel,
    Disabled,
    Msaa2x,
    Msaa4x,
    Msaa8x,
}

/// The Universal Render Pipeline Asset (Unity `UniversalRenderPipelineAsset`) with its
/// Universal Renderer data.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(default)]
pub struct UniversalRenderPipelineAsset {
    pub rendering_path: RenderingPath,
    pub msaa: PipelineMsaa,
    /// Universal Renderer "Opaque Layer Mask": layers whose opaque objects are drawn.
    pub opaque_layer_mask: u32,
    /// Universal Renderer "Transparent Layer Mask".
    pub transparent_layer_mask: u32,
    /// Post-processing on cameras that ask for it.
    pub post_processing: bool,
    /// A custom render pipeline registered by scripts (`scripts! { pipelines: ... }`);
    /// empty = the Universal Render Pipeline.
    pub custom_pipeline: String,
}

impl Default for UniversalRenderPipelineAsset {
    fn default() -> Self {
        Self {
            rendering_path: RenderingPath::ForwardPlus,
            msaa: PipelineMsaa::QualityLevel,
            opaque_layer_mask: u32::MAX,
            transparent_layer_mask: u32::MAX,
            post_processing: true,
            custom_pipeline: String::new(),
        }
    }
}

impl oxide_assets::Asset for UniversalRenderPipelineAsset {
    const TYPE_NAME: &'static str = "UniversalRenderPipelineAsset";
}

/// Unity's Graphics Settings (Project Settings): the default render pipeline.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GraphicsSettings {
    /// None: the built-in Universal settings (Forward+, Quality level MSAA).
    pub default_render_pipeline: Option<Handle<UniversalRenderPipelineAsset>>,
}

impl GraphicsSettings {
    /// Unity `GraphicsSettings.currentRenderPipeline`: the active Quality level's override,
    /// else the default.
    pub fn current(world: &crate::World) -> Option<Handle<UniversalRenderPipelineAsset>> {
        crate::QualitySettings::of(world)
            .render_pipeline
            .clone()
            .or_else(|| {
                world
                    .try_resource::<GraphicsSettings>()
                    .and_then(|g| g.default_render_pipeline.clone())
            })
    }
}

/// Unity's global shader keywords (`Shader.EnableKeyword`): a Shader Graph's Global-scope
/// keywords take their state from here for every material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shader {
    pub enabled_global_keywords: Vec<String>,
}

impl Shader {
    /// Unity `Shader.EnableKeyword`.
    pub fn enable_keyword(world: &mut crate::World, keyword: &str) {
        if world.try_resource::<Shader>().is_none() {
            world.resources.insert(Shader::default());
        }
        let s = world.resource_mut::<Shader>();
        if !s.enabled_global_keywords.iter().any(|k| k == keyword) {
            s.enabled_global_keywords.push(keyword.to_owned());
        }
    }

    /// Unity `Shader.DisableKeyword`.
    pub fn disable_keyword(world: &mut crate::World, keyword: &str) {
        if let Some(s) = world.try_resource_mut::<Shader>() {
            s.enabled_global_keywords.retain(|k| k != keyword);
        }
    }

    /// Unity `Shader.IsKeywordEnabled`.
    pub fn is_keyword_enabled(world: &crate::World, keyword: &str) -> bool {
        world
            .try_resource::<Shader>()
            .is_some_and(|s| s.enabled_global_keywords.iter().any(|k| k == keyword))
    }
}
