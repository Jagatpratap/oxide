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

pub mod anim_files;
pub mod animation;
pub mod animator_controller;
pub mod asset_types;
pub mod database;
pub mod fbx_import;
pub mod gltf_import;
pub mod handle;
pub mod importers;
pub mod ktx2_import;
pub mod lightmap_uv;
pub mod material;
pub mod mesh;
pub mod model;
pub mod obj_import;
pub mod preset;
pub mod primitives;
pub mod psd;
pub mod render_texture;
pub mod server;
pub mod shader;
pub mod sprite;
pub mod sprite_atlas;
pub mod terrain;
pub mod terrain_file;
pub mod texture;
pub mod texture_compress;

pub use animation::{
    AnimationClip, AnimationEvent, Interpolation, ObjectCurve, ObjectKey, PropertyCurve, Track,
    TransformCurves,
};
pub use animator_controller::{
    AnimatorController, AnimatorControllerLayer, AnimatorLayerBlendingMode,
    AnimatorOverrideController, AnimatorState, AnimatorStateMachine, AvatarMask,
    AvatarMaskBodyPart, BlendTree, BlendTree2DType, BlendTreeType, ChildMotion, ClipOverride,
    Condition, ConditionMode, InterruptionSource, Motion, Parameter, ParameterType, StateBehaviour,
    StatePath, SyncedMotion, Transition, TransitionSource, TransitionTarget,
};
pub use asset_types::{AssetTypeInfo, AssetTypeRegistry, CreateKind, DataAsset, DataAssets};
pub use handle::{Asset, AssetId, Assets, Handle};
pub use material::{Material, MaterialProperty, ShaderKind, SurfaceType};
pub use mesh::Mesh;
pub use model::{Model, ModelNode, ModelSkin};
pub use preset::Preset;
pub use primitives::PrimitiveType;
pub use render_texture::RenderTexture;
pub use server::{AssetServer, AudioClip, DEFAULT_MATERIAL};
pub use shader::{Shader, ShaderGraphRef, ShaderProperty, SurfaceShader};
pub use sprite::{Sprite, SpriteImportMode, SpriteRect};
pub use sprite_atlas::{AtlasReport, SpriteAtlas};
pub use terrain::{
    Brush, BrushShape, DetailPrototype, DetailRenderMode, HeightOp, TerrainData, TerrainLayer,
    TerrainVersions, TreeInstance, TreePrototype, WindSettings,
};
pub use texture::{
    CompressedFormat, CompressedMips, FilterMode, Texture2D, TextureCompression, TextureImporter,
    TextureImporterType, TextureWrapMode,
};

/// Whether `path` is a model file the importer reads (glTF, FBX, OBJ).
pub fn is_model_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [".glb", ".gltf", ".fbx", ".obj"]
        .iter()
        .any(|e| p.ends_with(e))
}
