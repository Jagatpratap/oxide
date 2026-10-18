//! Unity-style drag and drop, shared by the editor windows.
//!
//! A drag carries either a project asset ([`AssetPayload`]) or Hierarchy objects
//! ([`HierarchyDrag`]). While it moves, the target under the mouse decides whether it takes
//! it: [`mark`] outlines an accepting target and sets the copy cursor; anywhere else shows
//! the "rejected" cursor. A label naming what is dragged follows the cursor ([`overlay`]).
//! Esc cancels (egui drops the payload).
//!
//! The Scene view works as in Unity: a prefab or model becomes a preview instance that follows
//! the surface under the mouse and stays on release; a material previews on the object under
//! the mouse and is assigned to the sub-mesh hit on release; a script adds its component.

use crate::EditorState;
use crate::hierarchy::HierarchyDrag;
use crate::project::AssetPayload;
use egui::{Color32, CursorIcon, DragAndDrop, Stroke};
use oxide_assets::{AssetServer, Handle, Material};
use oxide_engine::Engine;
use oxide_math::{Ray, Vector3};
use oxide_scene::builtin::{MeshRenderer, SkinnedMeshRenderer};
use oxide_scene::{GameObjectId, World};

const ACCEPT: Color32 = Color32::from_rgb(90, 160, 255);

/// Unity type of an asset path: data assets by their file's `type`, files by extension, and
/// sub-assets (`car.glb#Body`) by the store that holds them.
pub fn asset_type(assets: &AssetServer, path: &str) -> Option<&'static str> {
    if path.contains('#') {
        if assets.materials.handle_of(path).is_some() {
            return Some("Material");
        }
        if assets.meshes.handle_of(path).is_some() {
            return Some("Mesh");
        }
        if assets.animations.handle_of(path).is_some() {
            return Some("AnimationClip");
        }
        if assets.sprites.handle_of(path).is_some() {
            return Some("Sprite");
        }
    }
    assets
        .data
        .type_of(path)
        .or_else(|| assets.types.for_path(path).map(|t| t.type_name))
}

/// What an asset does when dropped into the scene (Scene view or Hierarchy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneDrop {
    /// Prefab or model: an instance is created.
    Instantiate,
    /// Assigned to the renderer of the object it lands on.
    Material,
    /// The script's components are added to the object it lands on.
    Script,
    /// A new object with an Audio Source playing the clip (Unity does the same).
    AudioClip,
    /// Opened additively (Hierarchy only, as in Unity).
    Scene,
}

pub fn scene_drop_kind(assets: &AssetServer, path: &str) -> Option<SceneDrop> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".prefab") || oxide_assets::is_model_path(&lower) && !path.contains('#') {
        return Some(SceneDrop::Instantiate);
    }
    match asset_type(assets, path)? {
        "Material" => Some(SceneDrop::Material),
        "Script" => Some(SceneDrop::Script),
        "AudioClip" => Some(SceneDrop::AudioClip),
        "Scene" => Some(SceneDrop::Scene),
        _ => None,
    }
}

/// Component types a script file defines (Unity: the MonoScript's class). Every
/// `struct Name` in the file that is a registered component counts.
pub fn script_components(world: &World, path: &str) -> Vec<&'static str> {
    let full = world.resource::<AssetServer>().full_path(path);
    let Ok(src) = std::fs::read_to_string(full) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut words = src
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty());
    while let Some(w) = words.next() {
        if w == "struct"
            && let Some(name) = words.next()
            && let Some(info) = world.registry.get(name)
            && !out.contains(&info.info.name)
        {
            out.push(info.info.name);
        }
    }
    out
}

/// The asset path to store when `path` is dropped on a field of `type_name`, or `None` if
/// the field can't hold it. A model dropped on a Mesh field gives its first mesh, a texture
/// on a Sprite field its sprite, as in Unity.
pub fn resolve_for_field(
    assets: &AssetServer,
    type_name: Option<&str>,
    path: &str,
    candidates: &[String],
) -> Option<String> {
    let Some(want) = type_name else {
        return Some(path.to_owned());
    };
    if candidates.iter().any(|c| c == path) {
        return Some(path.to_owned());
    }
    let prefix = format!("{path}#");
    if let Some(sub) = candidates.iter().find(|c| c.starts_with(&prefix)) {
        return Some(sub.clone());
    }
    let have = asset_type(assets, path)?;
    let same = have == want
        || matches!(
            (want, have),
            ("Texture2D" | "Texture", "Texture" | "RenderTexture")
                | ("RuntimeAnimatorController", "AnimatorController")
        );
    same.then(|| path.to_owned())
}

/// The asset being dragged, if any.
pub fn dragged_asset(ctx: &egui::Context) -> Option<String> {
    DragAndDrop::payload::<AssetPayload>(ctx).map(|p| p.0.clone())
}

/// The Hierarchy objects being dragged, if any.
pub fn dragged_objects(ctx: &egui::Context) -> Option<Vec<GameObjectId>> {
    DragAndDrop::payload::<HierarchyDrag>(ctx).map(|p| p.0.clone())
}

pub fn is_dragging(ctx: &egui::Context) -> bool {
    DragAndDrop::has_any_payload(ctx)
}

/// Released this frame over `r`.
pub fn released_over(r: &egui::Response) -> bool {
    r.contains_pointer() && r.ctx.input(|i| i.pointer.any_released())
}

fn accepted_id() -> egui::Id {
    egui::Id::new("oxide-dnd-accepted")
}

/// A drop target under the dragged item: outlines `rect` and sets the cursor when it accepts;
/// shows the rejected cursor when it doesn't.
pub fn mark(ctx: &egui::Context, rect: egui::Rect, accepts: bool) {
    if accepts {
        ctx.debug_painter().rect_stroke(
            rect.shrink(0.5),
            2.0,
            Stroke::new(1.5, ACCEPT),
            egui::StrokeKind::Inside,
        );
        ctx.data_mut(|d| d.insert_temp(accepted_id(), true));
        ctx.set_cursor_icon(CursorIcon::Copy);
    } else {
        ctx.set_cursor_icon(CursorIcon::NoDrop);
    }
}

/// [`mark`] for a target that accepts by itself (draws nothing; Scene view placement).
pub fn accept_cursor(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(accepted_id(), true));
    ctx.set_cursor_icon(CursorIcon::Copy);
}

/// Once a frame, after every window: the label at the cursor, and the rejected cursor when no
/// target took the drag.
pub fn overlay(ctx: &egui::Context, world: &World) {
    let accepted = ctx.data_mut(|d| d.remove_temp::<bool>(accepted_id()).unwrap_or(false));
    let label = if let Some(path) = dragged_asset(ctx) {
        let name = path.rsplit(['/', '#']).next().unwrap_or(&path).to_owned();
        match asset_type(world.resource::<AssetServer>(), &path) {
            Some(t) => format!("{name}  ({t})"),
            None => name,
        }
    } else if let Some(ids) = dragged_objects(ctx) {
        match ids.as_slice() {
            [one] => world
                .get(*one)
                .map_or("GameObject".into(), |o| o.name.clone()),
            many => format!("{} GameObjects", many.len()),
        }
    } else {
        return;
    };
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    if !accepted {
        ctx.output_mut(|o| {
            if matches!(o.cursor_icon, CursorIcon::Default | CursorIcon::Grab) {
                o.cursor_icon = CursorIcon::NoDrop;
            }
        });
    }
    egui::Area::new(egui::Id::new("oxide-dnd-label"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + egui::vec2(14.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(egui::Margin::symmetric(6, 3))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(label).size(12.0));
                });
        });
}

// ---- edits shared by the Scene view, Hierarchy and Inspector ------------------------------

/// Renderer materials of `id` (Mesh or Skinned Mesh Renderer).
pub fn materials_of(world: &World, id: GameObjectId) -> Option<Vec<Handle<Material>>> {
    world
        .get_component::<MeshRenderer>(id)
        .map(|r| r.materials.clone())
        .or_else(|| {
            world
                .get_component::<SkinnedMeshRenderer>(id)
                .map(|r| r.materials.clone())
        })
}

pub fn set_materials(world: &mut World, id: GameObjectId, mats: Vec<Handle<Material>>) {
    if let Some(r) = world.get_component_mut::<MeshRenderer>(id) {
        r.materials = mats;
    } else if let Some(r) = world.get_component_mut::<SkinnedMeshRenderer>(id) {
        r.materials = mats;
    }
}

/// `materials` with slot `slot` set to `path` (slots up to it are filled with the first).
fn with_slot(mut mats: Vec<Handle<Material>>, slot: usize, path: &str) -> Vec<Handle<Material>> {
    let h = Handle::from_path(path);
    while mats.len() <= slot {
        mats.push(mats.first().cloned().unwrap_or_else(|| h.clone()));
    }
    mats[slot] = h;
    mats
}

/// Unity: a material dropped on an object replaces the material of the sub-mesh it lands on.
pub fn assign_material(
    st: &mut EditorState,
    world: &mut World,
    id: GameObjectId,
    slot: usize,
    path: &str,
) -> Result<String, String> {
    let mats = materials_of(world, id).ok_or("the object has no renderer")?;
    if world
        .resource_mut::<AssetServer>()
        .load_material(path)
        .is_err()
    {
        return Err(format!("cannot load material {path}"));
    }
    st.record(world, "Assign Material", None, now());
    set_materials(world, id, with_slot(mats, slot, path));
    let name = world.get(id).map_or("", |o| o.name.as_str()).to_owned();
    Ok(format!("Assigned {path} to {name}"))
}

/// Unity: a script dropped on an object adds its component.
pub fn add_script(
    st: &mut EditorState,
    world: &mut World,
    id: GameObjectId,
    path: &str,
) -> Result<String, String> {
    let comps = script_components(world, path);
    if comps.is_empty() {
        return Err(format!(
            "{path} has no component the editor knows (is it registered in `scripts!` and compiled?)"
        ));
    }
    st.record(world, "Add Component", None, now());
    for c in &comps {
        world.add_component_by_name(id, c)?;
    }
    st.select(id);
    Ok(format!("Added {}", comps.join(", ")))
}

/// Creates what a dropped asset makes in the scene: a prefab or model instance, or an Audio
/// Source object. Records Undo, selects it. `parent`/`sibling` place it in the Hierarchy;
/// `position` (world) is where it lands in the Scene view.
pub fn instantiate_asset(
    st: &mut EditorState,
    world: &mut World,
    path: &str,
    parent: Option<GameObjectId>,
    sibling: Option<usize>,
    position: Option<Vector3>,
) -> Result<GameObjectId, String> {
    let kind = scene_drop_kind(world.resource::<AssetServer>(), path);
    let label = match kind {
        Some(SceneDrop::AudioClip) => "Create Audio Source",
        _ => "Instantiate Prefab",
    };
    st.record(world, label, None, now());
    let id = create_instance(world, path, kind)?;
    if let Some(p) = parent {
        world.set_parent(id, Some(p), false)?;
    }
    if let Some(i) = sibling {
        world.set_sibling_index(id, i);
    }
    if let Some(p) = position {
        world.set_position(id, p);
    }
    st.select(id);
    st.hierarchy.reveal(world, id);
    Ok(id)
}

fn create_instance(
    world: &mut World,
    path: &str,
    kind: Option<SceneDrop>,
) -> Result<GameObjectId, String> {
    match kind {
        Some(SceneDrop::AudioClip) => {
            let name = path.rsplit('/').next().unwrap_or(path);
            let name = name.rsplit_once('.').map_or(name, |(n, _)| n);
            let id = world.create(name);
            let cid = world.add_component_by_name(id, "AudioSource")?;
            if let Some(c) = world.component_dyn_mut(id, cid) {
                let _ = c.set_field("clip", oxide_core::reflect::Value::Asset(path.into()));
            }
            Ok(id)
        }
        _ if path.to_ascii_lowercase().ends_with(".prefab") => world.instantiate_prefab(path, None),
        _ => {
            let h = world
                .resource_mut::<AssetServer>()
                .load_model(path)
                .map_err(|e| format!("cannot import {path}: {e}"))?;
            oxide_engine::objects::instantiate_model(world, &h)
                .ok_or_else(|| format!("{path} has no objects"))
        }
    }
}

fn now() -> f64 {
    oxide_core::time::realtime_since_startup()
}

// ---- Scene view -------------------------------------------------------------------------

/// A drag in progress over the Scene view.
#[derive(Default)]
pub enum ScenePreview {
    #[default]
    None,
    /// A preview instance following the mouse (prefab, model or audio clip).
    Placing { path: String, root: GameObjectId },
    /// The asset couldn't be instantiated; don't retry every frame.
    Failed(String),
    /// A material shown on the object under the mouse, and what it had before.
    Material {
        path: String,
        target: GameObjectId,
        slot: usize,
        original: Vec<Handle<Material>>,
    },
}

impl ScenePreview {
    /// Removes the preview from the scene (drag left the view, was cancelled or dropped).
    pub fn clear(&mut self, world: &mut World) {
        match std::mem::take(self) {
            ScenePreview::Placing { root, .. } => world.destroy_immediate(root),
            ScenePreview::Material {
                target, original, ..
            } => set_materials(world, target, original),
            ScenePreview::None | ScenePreview::Failed(_) => {}
        }
    }
}

/// Drag and drop over the Scene view, once a frame. Returns true while an asset drag is over
/// the view (other Scene view tools stay out of the way).
pub fn scene_view(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &egui::Ui,
    resp: &egui::Response,
    ray_at: &dyn Fn(egui::Pos2) -> Ray,
) -> bool {
    let ctx = ui.ctx();
    let path = dragged_asset(ctx);
    let pos = ui.input(|i| i.pointer.hover_pos());
    let over = resp.contains_pointer();
    let (Some(path), Some(pos), true) = (path, pos, over) else {
        st.scene_preview.clear(&mut engine.world);
        return false;
    };
    let released = ctx.input(|i| i.pointer.any_released());
    let ray = ray_at(pos);
    let kind = scene_drop_kind(engine.world.resource::<AssetServer>(), &path);
    match kind {
        Some(SceneDrop::Instantiate | SceneDrop::AudioClip) => {
            place(st, engine, ctx, &path, kind, &ray, released);
        }
        Some(SceneDrop::Material) => material(st, engine, ctx, &path, &ray, released),
        Some(SceneDrop::Script) => {
            st.scene_preview.clear(&mut engine.world);
            let hit = crate::scene_view::pick(&engine.world, &ray).map(|(id, _)| id);
            let ok = hit.is_some() && !script_components(&engine.world, &path).is_empty();
            if ok {
                accept_cursor(ctx);
            } else {
                ctx.set_cursor_icon(CursorIcon::NoDrop);
            }
            if released && let Some(id) = hit {
                let r = add_script(st, &mut engine.world, id, &path);
                st.status = r.unwrap_or_else(|e| e);
            }
        }
        Some(SceneDrop::Scene) | None => {
            st.scene_preview.clear(&mut engine.world);
            ctx.set_cursor_icon(CursorIcon::NoDrop);
        }
    }
    if released {
        DragAndDrop::clear_payload(ctx);
    }
    true
}

/// Where a dragged instance lands: the surface under the mouse (ignoring the instance
/// itself), else the ground plane, else in front of the camera.
fn landing_point(
    st: &EditorState,
    world: &World,
    ray: &Ray,
    skip: &std::collections::HashSet<GameObjectId>,
) -> Vector3 {
    crate::scene_view::pick_where(world, ray, |id| {
        !skip.contains(&id) && st.visibility.can_pick(world, id)
    })
    .map(|(_, t)| ray.get_point(t))
    .or_else(|| {
        oxide_math::Plane::new(Vector3::Y, Vector3::ZERO)
            .raycast(ray)
            .map(|t| ray.get_point(t))
    })
    .unwrap_or_else(|| ray.get_point(10.0))
}

fn place(
    st: &mut EditorState,
    engine: &mut Engine,
    ctx: &egui::Context,
    path: &str,
    kind: Option<SceneDrop>,
    ray: &Ray,
    released: bool,
) {
    let w = &mut engine.world;
    let current = match &st.scene_preview {
        ScenePreview::Placing { path: p, root } if p == path && w.is_alive(*root) => Some(*root),
        ScenePreview::Failed(p) if p == path => {
            ctx.set_cursor_icon(CursorIcon::NoDrop);
            return;
        }
        _ => None,
    };
    let root = match current {
        Some(r) => r,
        None => {
            st.scene_preview.clear(w);
            match create_instance(w, path, kind) {
                Ok(r) => {
                    st.scene_preview = ScenePreview::Placing {
                        path: path.to_owned(),
                        root: r,
                    };
                    r
                }
                Err(e) => {
                    st.status = e;
                    st.scene_preview = ScenePreview::Failed(path.to_owned());
                    return;
                }
            }
        }
    };
    let skip: std::collections::HashSet<_> = w.descendants(root).into_iter().collect();
    let point = landing_point(st, w, ray, &skip);
    w.set_position(root, point);
    accept_cursor(ctx);
    if released {
        // The preview isn't part of Undo history: remove it, then create the real one.
        st.scene_preview.clear(w);
        match instantiate_asset(st, w, path, None, None, Some(point)) {
            Ok(_) => st.status = format!("Instantiated {path}"),
            Err(e) => st.status = e,
        }
    }
}

fn material(
    st: &mut EditorState,
    engine: &mut Engine,
    ctx: &egui::Context,
    path: &str,
    ray: &Ray,
    released: bool,
) {
    let w = &mut engine.world;
    // Pick against the original materials (the preview doesn't change shapes, only looks).
    let hit = crate::scene_view::pick_where(w, ray, |id| {
        st.visibility.can_pick(w, id) && materials_of(w, id).is_some()
    })
    .map(|(id, _)| (id, crate::scene_view::submesh_at(w, id, ray)));
    let same = matches!(
        (&st.scene_preview, hit),
        (ScenePreview::Material { path: p, target, slot, .. }, Some((id, s)))
            if p == path && *target == id && *slot == s
    );
    if !same {
        st.scene_preview.clear(w);
        if let Some((id, slot)) = hit
            && let Some(original) = materials_of(w, id)
            && w.resource_mut::<AssetServer>().load_material(path).is_ok()
        {
            set_materials(w, id, with_slot(original.clone(), slot, path));
            st.scene_preview = ScenePreview::Material {
                path: path.to_owned(),
                target: id,
                slot,
                original,
            };
        }
    }
    if hit.is_some() {
        accept_cursor(ctx);
    } else {
        ctx.set_cursor_icon(CursorIcon::NoDrop);
    }
    if released && let Some((id, slot)) = hit {
        st.scene_preview.clear(w);
        st.status = assign_material(st, w, id, slot, path).unwrap_or_else(|e| e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_assets::PrimitiveType;

    fn project(name: &str) -> (std::path::PathBuf, World) {
        let dir = std::env::temp_dir().join(format!("oxide-dnd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets/Scripts")).unwrap();
        let mut w = World::new();
        oxide_scene::register_builtin(&mut w.registry);
        let mut assets = AssetServer::new(&dir);
        assets
            .save_material("Assets/Red.mat", &Material::default())
            .unwrap();
        w.resources.insert(assets);
        (dir, w)
    }

    #[test]
    fn drop_kinds_and_field_types() {
        let (dir, mut w) = project("kinds");
        let cube = oxide_engine::objects::create_primitive(&mut w, PrimitiveType::Cube);
        w.create_prefab(cube, "Assets/Box.prefab").unwrap();
        std::fs::write(dir.join("Assets/Hit.wav"), b"RIFF").unwrap();
        let a = w.resource::<AssetServer>();
        let kind = |p| scene_drop_kind(a, p);
        assert_eq!(kind("Assets/Box.prefab"), Some(SceneDrop::Instantiate));
        assert_eq!(kind("Assets/Car.glb"), Some(SceneDrop::Instantiate));
        assert_eq!(kind("Assets/Red.mat"), Some(SceneDrop::Material));
        assert_eq!(kind("Assets/Scripts/player.rs"), Some(SceneDrop::Script));
        assert_eq!(kind("Assets/Hit.wav"), Some(SceneDrop::AudioClip));
        assert_eq!(kind("Assets/Main.scene"), Some(SceneDrop::Scene));
        assert_eq!(kind("Assets/notes.txt"), None);

        // Fields take only what they can hold.
        let mats = a.paths_of_type("Material");
        let field = |ty, p| resolve_for_field(a, Some(ty), p, &mats);
        assert_eq!(
            field("Material", "Assets/Red.mat").as_deref(),
            Some("Assets/Red.mat")
        );
        assert_eq!(field("Material", "Assets/Box.prefab"), None);
        let textures = a.paths_of_type("Texture");
        assert_eq!(
            resolve_for_field(a, Some("Texture2D"), "Assets/Red.mat", &textures),
            None
        );
        let prefabs = a.paths_of_type("Prefab");
        assert_eq!(
            resolve_for_field(a, Some("Prefab"), "Assets/Box.prefab", &prefabs).as_deref(),
            Some("Assets/Box.prefab")
        );
        assert_eq!(
            resolve_for_field(a, Some("Prefab"), "Assets/Red.mat", &prefabs),
            None
        );
        // A model on a Mesh field gives its first mesh (Unity).
        let meshes = vec!["Assets/Car.glb#Body".to_owned()];
        assert_eq!(
            resolve_for_field(a, Some("Mesh"), "Assets/Car.glb", &meshes).as_deref(),
            Some("Assets/Car.glb#Body")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn script_files_name_their_components() {
        let (dir, w) = project("script");
        std::fs::write(
            dir.join("Assets/Scripts/lamp.rs"),
            "use x;\n#[derive(Reflect)]\npub struct Light { a: f32 }\nstruct Helper;\n",
        )
        .unwrap();
        assert_eq!(
            script_components(&w, "Assets/Scripts/lamp.rs"),
            vec!["Light"]
        );
        assert!(script_components(&w, "Assets/Scripts/missing.rs").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drops_assign_materials_and_place_instances() {
        let (dir, mut w) = project("apply");
        let mut st = EditorState::new();
        let cube = oxide_engine::objects::create_primitive(&mut w, PrimitiveType::Cube);
        w.create_prefab(cube, "Assets/Box.prefab").unwrap();

        // Material onto slot 1 of a one-material renderer: slot 0 keeps its material.
        let before = materials_of(&w, cube).unwrap();
        assign_material(&mut st, &mut w, cube, 1, "Assets/Red.mat").unwrap();
        let after = materials_of(&w, cube).unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(after[0], before[0]);
        assert_eq!(after[1].path(), "Assets/Red.mat");

        // Prefab dropped between rows: a sibling at that index, under the same parent.
        let parent = w.create("Parent");
        let first = w.create("First");
        w.set_parent(first, Some(parent), false).unwrap();
        let id = instantiate_asset(
            &mut st,
            &mut w,
            "Assets/Box.prefab",
            Some(parent),
            Some(0),
            None,
        )
        .unwrap();
        assert_eq!(w.parent(id), Some(parent));
        assert_eq!(w.sibling_index(id), 0);
        assert_eq!(st.selection, vec![id]);
        assert!(w.get(id).unwrap().prefab_source.is_some());

        // Undo removes the instance in one step.
        let n = w.objects().count();
        let (label, _) = st.undo.undo(&mut w, &[]).unwrap();
        assert_eq!(label, "Instantiate Prefab");
        assert_eq!(w.objects().count(), n - 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scene_preview_is_removed_when_cleared() {
        let (dir, mut w) = project("preview");
        let cube = oxide_engine::objects::create_primitive(&mut w, PrimitiveType::Cube);
        let original = materials_of(&w, cube).unwrap();
        let mut p = ScenePreview::Material {
            path: "Assets/Red.mat".into(),
            target: cube,
            slot: 0,
            original: original.clone(),
        };
        set_materials(&mut w, cube, vec![Handle::from_path("Assets/Red.mat")]);
        p.clear(&mut w);
        assert_eq!(materials_of(&w, cube).unwrap(), original);
        let ghost = w.create("Ghost");
        let mut p = ScenePreview::Placing {
            path: "Assets/Box.prefab".into(),
            root: ghost,
        };
        p.clear(&mut w);
        assert!(!w.is_alive(ghost));
        let _ = std::fs::remove_dir_all(dir);
    }
}
