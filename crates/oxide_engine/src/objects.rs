//! Unity's GameObject menu: primitives, cameras, lights, empty objects, model instances.
//! Reference: <https://docs.unity3d.com/Manual/PrimitiveObjects.html>

use oxide_assets::{AssetServer, Handle, Material, Model, PrimitiveType};
use oxide_math::{Color, Quaternion, QuaternionExt, Vector3};
use oxide_scene::{
    Camera, GameObjectId, Light, MeshFilter, MeshRenderer, SkinnedMeshRenderer, World,
};

/// Unity `GameObject.CreatePrimitive`: mesh, renderer and the matching collider
/// (Cylinder gets a CapsuleCollider and Plane/Quad a MeshCollider, as in Unity).
pub fn create_primitive(world: &mut World, p: PrimitiveType) -> GameObjectId {
    let id = world.create(p.name());
    world.add_component(
        id,
        MeshFilter {
            mesh: Some(Handle::from_path(p.mesh_path())),
        },
    );
    world.add_component(id, MeshRenderer::default());
    use oxide_physics::*;
    match p {
        PrimitiveType::Cube => {
            world.add_component(id, BoxCollider::default());
        }
        PrimitiveType::Sphere => {
            world.add_component(id, SphereCollider::default());
        }
        PrimitiveType::Capsule | PrimitiveType::Cylinder => {
            world.add_component(id, CapsuleCollider::default());
        }
        PrimitiveType::Plane | PrimitiveType::Quad => {
            world.add_component(id, MeshCollider::default());
        }
    }
    id
}

/// A material asset with a color (Unity: Create > Material, then set Base Map color).
/// A primitive mesh without a collider (visual-only parts of a larger object).
pub fn create_visual(
    world: &mut World,
    name: &str,
    p: PrimitiveType,
    material: Option<Handle<Material>>,
) -> GameObjectId {
    let id = world.create(name);
    world.add_component(
        id,
        MeshFilter {
            mesh: Some(Handle::from_path(p.mesh_path())),
        },
    );
    world.add_component(
        id,
        match material {
            Some(m) => MeshRenderer::with_material(m),
            None => MeshRenderer::default(),
        },
    );
    id
}

/// Sets local position/rotation (degrees)/scale in one call.
pub fn set_local(
    world: &mut World,
    id: GameObjectId,
    position: Vector3,
    euler: Vector3,
    scale: Vector3,
) {
    if let Some(t) = world.transform_mut(id) {
        t.local_position = position;
        t.local_rotation = Quaternion::euler_v(euler);
        t.local_scale = scale;
    }
}

pub fn color_material(world: &mut World, name: &str, color: Color) -> Handle<Material> {
    world
        .resource_mut::<AssetServer>()
        .materials
        .add(Material::lit(name, color))
}

/// Sets the first material of an object's renderer.
pub fn set_material(world: &mut World, id: GameObjectId, m: Handle<Material>) {
    if let Some(r) = world.get_component_mut::<MeshRenderer>(id) {
        r.materials = vec![m];
    }
}

pub fn create_camera(world: &mut World, name: &str) -> GameObjectId {
    let id = world.create(name);
    world.add_component(id, Camera::default());
    world.get_mut(id).expect("created").tag = "MainCamera".into();
    id
}

pub fn create_directional_light(world: &mut World) -> GameObjectId {
    let id = world.create_at(
        "Directional Light",
        Vector3::new(0.0, 3.0, 0.0),
        Quaternion::euler(50.0, -30.0, 0.0),
    );
    world.add_component(id, Light::directional());
    id
}

pub fn create_point_light(
    world: &mut World,
    position: Vector3,
    color: Color,
    range: f32,
    intensity: f32,
) -> GameObjectId {
    let id = world.create_at("Point Light", position, Quaternion::IDENTITY);
    world.add_component(
        id,
        Light {
            color,
            range,
            intensity,
            ..Default::default()
        },
    );
    id
}

/// Unity's new-scene defaults: "Main Camera" at (0, 1, -10) and a "Directional Light".
pub fn create_default_scene(world: &mut World) {
    let cam = create_camera(world, "Main Camera");
    world.transform_mut(cam).expect("camera").local_position = Vector3::new(0.0, 1.0, -10.0);
    create_directional_light(world);
}

/// Instantiates an imported model as GameObjects (Unity: drag a model into the scene).
/// Returns the root, named after the file. Skinned meshes get a SkinnedMeshRenderer bound
/// to the created bone objects.
pub fn instantiate_model(world: &mut World, model: &Handle<Model>) -> Option<GameObjectId> {
    let m = world.resource::<AssetServer>().models.get(model)?.clone();
    let root = world.create(&m.name);
    world.get_mut(root)?.prefab_source = Some(model.path().to_owned());
    let mut ids = vec![None; m.nodes.len()];
    let mut stack: Vec<(usize, GameObjectId)> = m.roots.iter().rev().map(|&r| (r, root)).collect();
    while let Some((i, parent)) = stack.pop() {
        let n = &m.nodes[i];
        let id = world.create(&n.name);
        {
            let t = &mut world.get_mut(id)?.transform;
            t.local_position = n.position;
            t.local_rotation = n.rotation;
            t.local_scale = n.scale;
        }
        let _ = world.set_parent(id, Some(parent), false);
        ids[i] = Some(id);
        for &c in n.children.iter().rev() {
            stack.push((c, id));
        }
    }
    // Animated models get an Animator with one state per clip (like Unity's import).
    if !m.animations.is_empty() {
        let assets = world.resource_mut::<AssetServer>();
        let clips: Vec<(String, Handle<oxide_assets::AnimationClip>)> = m
            .animations
            .iter()
            .map(|h| {
                (
                    assets
                        .animations
                        .get(h)
                        .map_or_else(|| h.path().to_owned(), |c| c.name.clone()),
                    h.clone(),
                )
            })
            .collect();
        let ctrl = oxide_assets::AnimatorController::from_clips(&m.name, &clips);
        let h = assets
            .controllers
            .insert(format!("{}#controller", model.path()), ctrl);
        world.add_component(root, oxide_animation::Animator::new(h));
    }
    for (i, n) in m.nodes.iter().enumerate() {
        let (Some(id), Some(mesh)) = (ids[i], &n.mesh) else {
            continue;
        };
        match n.skin.and_then(|s| m.skins.get(s)) {
            Some(skin) => {
                let bones = skin.joints.iter().filter_map(|&j| ids[j]).collect();
                world.add_component(
                    id,
                    SkinnedMeshRenderer {
                        mesh: Some(mesh.clone()),
                        materials: n.materials.clone(),
                        bones,
                        root_bone: skin.joints.first().and_then(|&j| ids[j]),
                        cast_shadows: oxide_scene::ShadowCastingMode::On,
                        receive_shadows: true,
                    },
                );
            }
            None => {
                world.add_component(
                    id,
                    MeshFilter {
                        mesh: Some(mesh.clone()),
                    },
                );
                world.add_component(
                    id,
                    MeshRenderer {
                        materials: n.materials.clone(),
                        ..Default::default()
                    },
                );
            }
        }
    }
    add_lod_groups(world, root);
    Some(root)
}

/// `Rock_LOD2` → (`Rock`, 2).
fn lod_suffix(name: &str) -> Option<(&str, usize)> {
    let (base, n) = name
        .rsplit_once("_LOD")
        .or_else(|| name.rsplit_once("_lod"))?;
    Some((base, n.parse().ok()?))
}

/// Unity's model import convention: children named `*_LOD0`, `*_LOD1`, ... become the levels
/// of a [`LODGroup`] on their parent (each level shows at half the screen height of the
/// previous one, the last down to 1%).
pub fn add_lod_groups(world: &mut World, root: GameObjectId) {
    for parent in world.descendants(root) {
        if world.has_component::<oxide_scene::LODGroup>(parent) {
            continue;
        }
        let mut levels: Vec<(usize, GameObjectId)> = world
            .children(parent)
            .iter()
            .filter_map(|&c| {
                let name = world.get(c)?.name.clone();
                lod_suffix(&name).map(|(_, n)| (n, c))
            })
            .collect();
        if levels.len() < 2 {
            continue;
        }
        levels.sort();
        let count = levels.len();
        let lods = levels
            .into_iter()
            .enumerate()
            .map(|(i, (_, obj))| oxide_scene::Lod {
                screen_relative_height: if i + 1 == count {
                    0.01
                } else {
                    0.6 / (1u32 << i) as f32
                },
                // The level's object and everything under it.
                renderers: world.descendants(obj),
            })
            .collect();
        world.add_component(parent, oxide_scene::LODGroup { lods, size: 0.0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_children_become_a_lod_group() {
        let mut w = World::new();
        crate::init_world(&mut w, std::env::temp_dir());
        let root = w.create("Rock");
        for n in ["Rock_LOD1", "Rock_LOD0", "Rock_LOD2"] {
            let c = create_primitive(&mut w, PrimitiveType::Sphere);
            w.get_mut(c).unwrap().name = n.into();
            w.set_parent(c, Some(root), false).unwrap();
        }
        add_lod_groups(&mut w, root);
        let g = w.get_component::<oxide_scene::LODGroup>(root).unwrap();
        let names: Vec<_> = g
            .lods
            .iter()
            .map(|l| w.get(l.renderers[0]).unwrap().name.as_str())
            .collect();
        assert_eq!(names, ["Rock_LOD0", "Rock_LOD1", "Rock_LOD2"]);
        assert_eq!(g.lods[2].screen_relative_height, 0.01);
        assert_eq!(lod_suffix("Tree_LOD12"), Some(("Tree", 12)));
        assert_eq!(lod_suffix("Tree"), None);
    }
}
