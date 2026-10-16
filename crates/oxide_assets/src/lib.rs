//! Oxide assets: what Unity keeps in the Project window.
//!
//! - [`handle`]: [`Handle`]s (path + id) and typed [`Assets`] stores.
//! - [`mesh`] / [`primitives`]: Unity `Mesh` and the built-in Cube/Sphere/Capsule/...
//! - [`texture`], [`material`]: `Texture2D` and URP-Lit-style `Material`.
//! - [`animation`]: transform `AnimationClip`s.
//! - [`model`] / [`gltf_import`]: imported model hierarchies (glTF 2.0).
//! - [`server`]: the [`AssetServer`] that owns every store and loads files.

// Submesh lists are `Vec<Range<u32>>`; a single-entry list is the normal case.
#![allow(clippy::single_range_in_vec_init)]

pub mod animation;
pub mod asset_types;
pub mod gltf_import;
pub mod handle;
pub mod ktx2_import;
pub mod lightmap_uv;
pub mod material;
pub mod mesh;
pub mod model;
pub mod preset;
pub mod primitives;
pub mod render_texture;
pub mod server;
pub mod shader;
pub mod terrain;
pub mod texture;
pub mod texture_compress;

pub use animation::{
    AnimationClip, AnimationEvent, AnimatorController, AnimatorControllerLayer,
    AnimatorLayerBlendingMode, AnimatorState, AnimatorStateMachine, AvatarMask, BlendTree2DType,
    Condition, ConditionMode, Interpolation, Motion, Parameter, ParameterType, Track,
    TransformCurves, Transition,
};
pub use asset_types::{AssetTypeInfo, AssetTypeRegistry, CreateKind, DataAsset, DataAssets};
pub use handle::{Asset, AssetId, Assets, Handle};
pub use material::{Material, ShaderKind, SurfaceType};
pub use mesh::Mesh;
pub use model::{Model, ModelNode, ModelSkin};
pub use preset::Preset;
pub use primitives::PrimitiveType;
pub use render_texture::RenderTexture;
pub use server::{AssetServer, AudioClip, DEFAULT_MATERIAL};
pub use shader::Shader;
pub use terrain::{TerrainData, TerrainLayer, TreeInstance, TreePrototype};
pub use texture::{
    CompressedFormat, CompressedMips, FilterMode, Texture2D, TextureCompression, TextureImporter,
    TextureImporterType, TextureWrapMode,
};
