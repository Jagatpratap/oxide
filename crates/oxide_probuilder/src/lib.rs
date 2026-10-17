//! Oxide ProBuilder: Unity's in-editor modeling. A [`ProBuilderMesh`] holds an editable
//! polygon mesh ([`PbMesh`]); the system rebuilds its renderable mesh whenever it changes and
//! points the object's MeshFilter (and MeshCollider) at it. Shapes come from [`shapes`].
//! Reference: <https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/index.html>

pub mod csg;
pub mod edit;
pub mod pbmesh;
pub mod shapes;

pub use csg::{BooleanOp, boolean, boolean_transformed};
pub use pbmesh::{AutoUv, Face, PbMesh, edge_key, triangulate};
pub use shapes::{ShapeParams, ShapeType};

use oxide_assets::{AssetServer, DEFAULT_MATERIAL, Handle, Material, Mesh};
use oxide_core::Reflect;
use oxide_scene::{Component, GameObjectId, MeshFilter, MeshRenderer, System, World};
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct BuildState {
    stamp: u64,
    shape_stamp: u64,
}

/// Unity `ProBuilderMesh` (with `ProBuilderShape`'s settings while the shape is unedited).
#[derive(Clone, Debug, Reflect)]
pub struct ProBuilderMesh {
    pub mesh: PbMesh,
    /// One per material slot used by faces (empty slots use the default material).
    pub materials: Vec<Handle<Material>>,
    /// The shape it was made from; changing it rebuilds the geometry. Editing faces,
    /// edges or vertices drops it (Unity keeps edits and the shape apart the same way).
    pub shape: Option<ShapeParams>,
    /// Unity's Generate Lightmap UVs: a non-overlapping second UV set so the object can be
    /// lightmapped (Contribute GI).
    pub generate_lightmap_uvs: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub state: BuildState,
}

impl Default for ProBuilderMesh {
    fn default() -> Self {
        Self::from_shape(ShapeParams::default())
    }
}

impl Component for ProBuilderMesh {}

fn hash_shape(s: &ShapeParams) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{s:?}").hash(&mut h);
    h.finish()
}

impl ProBuilderMesh {
    pub fn from_shape(shape: ShapeParams) -> Self {
        Self {
            mesh: shape.build(),
            materials: Vec::new(),
            state: BuildState {
                stamp: 0,
                shape_stamp: hash_shape(&shape),
            },
            shape: Some(shape),
            generate_lightmap_uvs: false,
        }
    }

    pub fn from_mesh(mesh: PbMesh) -> Self {
        Self {
            mesh,
            materials: Vec::new(),
            shape: None,
            generate_lightmap_uvs: false,
            state: BuildState::default(),
        }
    }

    /// Material slots the faces use.
    pub fn slot_count(&self) -> usize {
        self.mesh
            .faces
            .iter()
            .map(|f| f.material as usize + 1)
            .max()
            .unwrap_or(1)
            .max(self.materials.len())
    }

    /// Unity's Apply Material to faces: the material's slot (added if new).
    pub fn set_face_material(&mut self, faces: &[usize], material: Handle<Material>) {
        let slot = match self
            .materials
            .iter()
            .position(|m| m.path() == material.path())
        {
            Some(i) => i,
            None => {
                // Slot 0 keeps the default until something is assigned.
                if self.materials.is_empty() {
                    self.materials
                        .push(Handle::from_path(DEFAULT_MATERIAL.to_owned()));
                }
                self.materials.push(material);
                self.materials.len() - 1
            }
        };
        for f in faces {
            if let Some(face) = self.mesh.faces.get_mut(*f) {
                face.material = slot as u32;
            }
        }
        self.edited();
    }

    /// Marks the geometry as edited by hand (the shape no longer drives it).
    pub fn edited(&mut self) {
        self.shape = None;
    }

    fn stamp(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for p in &self.mesh.positions {
            p.x.to_bits().hash(&mut h);
            p.y.to_bits().hash(&mut h);
            p.z.to_bits().hash(&mut h);
        }
        for f in &self.mesh.faces {
            f.indices.hash(&mut h);
            f.material.hash(&mut h);
            f.smoothing_group.hash(&mut h);
            format!("{:?}{:?}{:?}", f.uv, f.color, f.manual_uv).hash(&mut h);
        }
        for m in &self.materials {
            m.path().hash(&mut h);
        }
        self.generate_lightmap_uvs.hash(&mut h);
        h.finish()
    }
}

/// Where an object's built mesh lives.
pub fn mesh_path(id: GameObjectId) -> String {
    format!("runtime:probuilder/{}", id.to_bits())
}

/// Rebuilds every ProBuilder mesh that changed (Edit mode too).
pub fn update(world: &mut World) {
    let ids: Vec<GameObjectId> = world.query::<ProBuilderMesh>().map(|(id, _)| id).collect();
    for id in ids {
        let Some(pb) = world.get_component_mut::<ProBuilderMesh>(id) else {
            continue;
        };
        // A changed shape rebuilds the geometry.
        if let Some(s) = &pb.shape {
            let st = hash_shape(s);
            if st != pb.state.shape_stamp {
                pb.mesh = s.build();
                pb.state.shape_stamp = st;
            }
        }
        let stamp = pb.stamp();
        let path = mesh_path(id);
        let filter_ok = world
            .get_component::<MeshFilter>(id)
            .is_some_and(|f| f.mesh.as_ref().is_some_and(|m| m.path() == path));
        let pb = world.get_component::<ProBuilderMesh>(id).expect("checked");
        if stamp == pb.state.stamp && filter_ok {
            continue;
        }
        let name = world
            .get(id)
            .map_or("ProBuilder", |o| o.name.as_str())
            .to_owned();
        let slots = pb.slot_count();
        let mut mesh: Mesh = pb.mesh.build(&name, slots);
        if pb.generate_lightmap_uvs {
            mesh.generate_lightmap_uvs();
        }
        let mats: Vec<Handle<Material>> = (0..slots)
            .map(|k| {
                pb.materials
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| Handle::from_path(DEFAULT_MATERIAL.to_owned()))
            })
            .collect();
        let handle = world
            .resource_mut::<AssetServer>()
            .meshes
            .insert(path, mesh);
        match world.get_component_mut::<MeshFilter>(id) {
            Some(f) => f.mesh = Some(handle.clone()),
            None => {
                world.add_component(
                    id,
                    MeshFilter {
                        mesh: Some(handle.clone()),
                    },
                );
            }
        }
        match world.get_component_mut::<MeshRenderer>(id) {
            Some(r) => r.materials = mats,
            None => {
                world.add_component(
                    id,
                    MeshRenderer {
                        materials: mats,
                        ..Default::default()
                    },
                );
            }
        }
        if let Some(c) = world.get_component_mut::<oxide_physics::MeshCollider>(id) {
            c.mesh = Some(handle);
        }
        if let Some(pb) = world.get_component_mut::<ProBuilderMesh>(id) {
            pb.state.stamp = stamp;
        }
    }
}

/// Keeps ProBuilder meshes built.
#[derive(Default)]
pub struct ProBuilderSystem;

impl System for ProBuilderSystem {
    fn name(&self) -> &'static str {
        "ProBuilder"
    }

    fn always(&mut self, world: &mut World) {
        update(world);
    }
}

/// Unity's Shape tool: a new object with a shape (and a Mesh Collider, as ProBuilder adds).
pub fn create_shape(world: &mut World, shape: ShapeParams) -> GameObjectId {
    let id = world.create(shape.shape.name());
    world.add_component(id, ProBuilderMesh::from_shape(shape));
    world.add_component(id, oxide_physics::MeshCollider::default());
    update(world);
    id
}

/// Unity's Boolean tool: a new object "<a> <op> <b>" with `a` op `b` (in `a`'s space, at
/// `a`'s transform), using both objects' materials. The originals are kept.
pub fn boolean_objects(
    world: &mut World,
    a: GameObjectId,
    b: GameObjectId,
    op: BooleanOp,
) -> Result<GameObjectId, String> {
    let get = |id| {
        world
            .get_component::<ProBuilderMesh>(id)
            .cloned()
            .ok_or("both objects need a ProBuilder Mesh")
    };
    let (pa, pb) = (get(a)?, get(b)?);
    let a_model = world.local_to_world(a);
    let to_a = a_model.inverse() * world.local_to_world(b);
    let slots_a = pa.slot_count();
    let mesh = boolean_transformed(
        &pa.mesh,
        &|p| p,
        &pb.mesh,
        &|p| to_a.transform_point3(p),
        op,
        slots_a as u32,
    );
    if mesh.faces.is_empty() {
        return Err(format!("{op:?} left nothing (do the objects overlap?)"));
    }
    let default = || Handle::from_path(DEFAULT_MATERIAL.to_owned());
    let mut materials: Vec<Handle<Material>> = (0..slots_a)
        .map(|k| pa.materials.get(k).cloned().unwrap_or_else(default))
        .collect();
    materials
        .extend((0..pb.slot_count()).map(|k| pb.materials.get(k).cloned().unwrap_or_else(default)));
    let name = |id| {
        world
            .get(id)
            .map_or("Object".to_owned(), |o| o.name.clone())
    };
    let label = match op {
        BooleanOp::Union => "Union",
        BooleanOp::Subtract => "Subtract",
        BooleanOp::Intersect => "Intersect",
    };
    let title = format!("{} {label} {}", name(a), name(b));
    let (pos, rot) = (world.position(a), world.rotation(a));
    let scale = world
        .get(a)
        .map_or(oxide_math::Vector3::ONE, |o| o.transform.local_scale);
    let id = world.create(&title);
    world.set_position(id, pos);
    world.set_rotation(id, rot);
    if let Some(o) = world.get_mut(id) {
        o.transform.local_scale = scale;
    }
    let mut c = ProBuilderMesh::from_mesh(mesh);
    c.materials = materials;
    world.add_component(id, c);
    world.add_component(id, oxide_physics::MeshCollider::default());
    update(world);
    Ok(id)
}

/// Unity's ProBuilderize: turns an object's mesh into an editable ProBuilder mesh.
pub fn probuilderize(world: &mut World, id: GameObjectId) -> Result<(), String> {
    let path = world
        .get_component::<MeshFilter>(id)
        .and_then(|f| f.mesh.clone())
        .ok_or("the object has no Mesh Filter mesh")?;
    let mats = world
        .get_component::<MeshRenderer>(id)
        .map(|r| r.materials.clone())
        .unwrap_or_default();
    let assets = world.resource::<AssetServer>();
    let mesh = assets
        .meshes
        .get(&path)
        .ok_or_else(|| format!("{} is not loaded", path.path()))?;
    if mesh.is_skinned() {
        return Err("skinned meshes can't be ProBuilderized".into());
    }
    let mut pb = PbMesh::from_mesh(mesh);
    pb.merge_coplanar_triangles();
    let mut c = ProBuilderMesh::from_mesh(pb);
    c.materials = mats;
    world.add_component(id, c);
    update(world);
    Ok(())
}

impl PbMesh {
    /// Joins triangle pairs that form flat quads (ProBuilderize keeps quads as quads).
    pub fn merge_coplanar_triangles(&mut self) {
        let mut merged = true;
        while merged {
            merged = false;
            let ef = self.edge_faces();
            let mut pairs: Vec<(usize, usize)> = ef
                .values()
                .filter(|f| f.len() == 2)
                .map(|f| (f[0].min(f[1]), f[0].max(f[1])))
                .filter(|(a, b)| {
                    self.faces[*a].indices.len() == 3
                        && self.faces[*b].indices.len() == 3
                        && self.faces[*a].material == self.faces[*b].material
                        && self.face_normal(*a).dot(self.face_normal(*b)) > 0.9999
                })
                .collect();
            pairs.sort();
            if let Some((a, b)) = pairs.first().copied() {
                merged = self.merge_faces(&[a, b]);
            }
        }
    }
}

/// Installs the component; returns the system.
pub fn install(world: &mut World) -> Box<dyn System> {
    world
        .registry
        .register::<ProBuilderMesh>("ProBuilder/ProBuilder Mesh");
    Box::new(ProBuilderSystem)
}
