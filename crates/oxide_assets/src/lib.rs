//! Oxide assets: what Unity keeps in the Project window.
//!
//! - [`handle`]: [`Handle`]s (path + id) and typed [`Assets`] stores.
//! - [`mesh`] / [`primitives`]: Unity `Mesh` and the built-in Cube/Sphere/Capsule/...
//! - [`texture`], [`material`]: `Texture2D` and URP-Lit-style `Material`.
//! - [`animation`]: transform `AnimationClip`s.
//! - [`model`] / [`gltf_import`]: imported model hierarchies (glTF 2.0).
//! - [`server`]: the [`AssetServer`] that owns every store and loads files.

pub mod animation;
pub mod gltf_import;
pub mod handle;
pub mod material;
pub mod mesh;
pub mod model;
pub mod primitives;
pub mod server;
pub mod texture;

pub use animation::{AnimationClip, Interpolation, Track, TransformCurves};
pub use handle::{Asset, AssetId, Assets, Handle};
pub use material::{Material, ShaderKind, SurfaceType};
pub use mesh::Mesh;
pub use model::{Model, ModelNode, ModelSkin};
pub use primitives::PrimitiveType;
pub use server::{AssetServer, AudioClip, DEFAULT_MATERIAL};
pub use texture::{FilterMode, Texture2D, TextureWrapMode};
