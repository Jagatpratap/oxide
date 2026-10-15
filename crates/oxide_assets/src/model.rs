//! Imported 3D model (Unity's "model prefab"): a node hierarchy referencing meshes,
//! materials, skins and animation clips stored in the asset stores.

use crate::animation::AnimationClip;
use crate::handle::{Asset, Handle};
use crate::material::Material;
use crate::mesh::Mesh;
use oxide_math::{Quaternion, Vector3};

#[derive(Clone, Debug)]
pub struct ModelNode {
    pub name: String,
    pub position: Vector3,
    pub rotation: Quaternion,
    pub scale: Vector3,
    pub children: Vec<usize>,
    pub mesh: Option<Handle<Mesh>>,
    /// One per sub-mesh.
    pub materials: Vec<Handle<Material>>,
    /// Index into [`Model::skins`] if the mesh is skinned.
    pub skin: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct ModelSkin {
    /// Node index of each bone, in the order used by the mesh's bone indices.
    pub joints: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub name: String,
    pub nodes: Vec<ModelNode>,
    pub roots: Vec<usize>,
    pub skins: Vec<ModelSkin>,
    pub animations: Vec<Handle<AnimationClip>>,
}

impl Asset for Model {
    const TYPE_NAME: &'static str = "Model";
}
