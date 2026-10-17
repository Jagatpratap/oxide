//! Prefabs: assets made from a GameObject subtree, linked instances, overrides,
//! Apply/Revert/Unpack, and prefab variants.
//!
//! A prefab file is a [`SceneFile`] whose object ids are the prefab's own ids. Every object
//! of an instance remembers its prefab id ([`GameObject::prefab_link`]), so an instance can
//! be compared with its prefab field by field ([`overrides`]) and updated in place when the
//! prefab changes ([`World::sync_instance`]): prefab changes come through, the instance's own
//! overrides stay. Scenes and variants embed the prefab content they were saved against, so
//! changes made to a prefab later reach them when they are loaded.
//!
//! References: <https://docs.unity3d.com/Manual/Prefabs.html>,
//! <https://docs.unity3d.com/Manual/PrefabInstanceOverrides.html>,
//! <https://docs.unity3d.com/Manual/PrefabVariants.html>

use crate::id::GameObjectId;
use crate::serialize::{ComponentData, EXTERNAL_BIT, ObjectData, SceneFile, remap};
use crate::world::World;
use oxide_core::reflect::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// First id given to instance objects that aren't part of the prefab (added objects).
pub const ADDED_BASE: u64 = 1 << 40;

/// An object property an instance can override.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectProperty {
    Name(String),
    Active(bool),
    Tag(String),
    Layer(u8),
    Static(u32),
    Position([f32; 3]),
    Rotation([f32; 4]),
    Scale([f32; 3]),
    Parent(Option<u64>),
}

/// One difference between an instance and its prefab. Object ids are prefab ids.
#[derive(Clone, Debug, PartialEq)]
pub enum PrefabOverride {
    Property {
        object: u64,
        property: ObjectProperty,
    },
    /// Field of the `index`-th component of type `component` on the object.
    Field {
        object: u64,
        component: String,
        index: usize,
        field: String,
        value: Value,
    },
    Enabled {
        object: u64,
        component: String,
        index: usize,
        enabled: bool,
    },
    AddedComponent {
        object: u64,
        component: ComponentData,
    },
    RemovedComponent {
        object: u64,
        component: String,
        index: usize,
    },
    AddedObject {
        data: ObjectData,
    },
    RemovedObject {
        object: u64,
    },
}

impl PrefabOverride {
    pub fn object(&self) -> u64 {
        match self {
            PrefabOverride::Property { object, .. }
            | PrefabOverride::Field { object, .. }
            | PrefabOverride::Enabled { object, .. }
            | PrefabOverride::AddedComponent { object, .. }
            | PrefabOverride::RemovedComponent { object, .. }
            | PrefabOverride::RemovedObject { object } => *object,
            PrefabOverride::AddedObject { data } => data.id,
        }
    }

    /// Root position/rotation always differ per instance; they aren't shown as overrides.
    pub fn is_root_placement(&self, root: u64) -> bool {
        matches!(self, PrefabOverride::Property { object, property: ObjectProperty::Position(_) | ObjectProperty::Rotation(_) }
            if *object == root)
    }
}

fn close<const N: usize>(a: &[f32; N], b: &[f32; N]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-5)
}

/// Components of an object grouped by type, in order: `(type, index within type)`.
fn indexed(components: &[ComponentData]) -> Vec<(&str, usize, &ComponentData)> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    components
        .iter()
        .map(|c| {
            let n = seen.entry(c.type_name.as_str()).or_default();
            let i = *n;
            *n += 1;
            (c.type_name.as_str(), i, c)
        })
        .collect()
}

/// How `instance` differs from `prefab` (both in prefab-id space).
pub fn overrides(instance: &SceneFile, prefab: &SceneFile) -> Vec<PrefabOverride> {
    use ObjectProperty as P;
    let by_id: HashMap<u64, &ObjectData> = prefab.objects.iter().map(|o| (o.id, o)).collect();
    let present: HashSet<u64> = instance.objects.iter().map(|o| o.id).collect();
    let mut out = Vec::new();
    for od in &instance.objects {
        let Some(po) = by_id.get(&od.id) else {
            out.push(PrefabOverride::AddedObject { data: od.clone() });
            continue;
        };
        let object = od.id;
        let mut prop = |property| out.push(PrefabOverride::Property { object, property });
        if od.name != po.name {
            prop(P::Name(od.name.clone()));
        }
        if od.active != po.active {
            prop(P::Active(od.active));
        }
        if od.tag != po.tag {
            prop(P::Tag(od.tag.clone()));
        }
        if od.layer != po.layer {
            prop(P::Layer(od.layer));
        }
        if od.static_flags != po.static_flags {
            prop(P::Static(od.static_flags));
        }
        if !close(&od.position, &po.position) {
            prop(P::Position(od.position));
        }
        if !close(&od.rotation, &po.rotation) {
            prop(P::Rotation(od.rotation));
        }
        if !close(&od.scale, &po.scale) {
            prop(P::Scale(od.scale));
        }
        if od.parent != po.parent {
            prop(P::Parent(od.parent));
        }
        let mine = indexed(&od.components);
        let theirs = indexed(&po.components);
        for (t, i, cd) in &mine {
            match theirs.iter().find(|(pt, pi, _)| pt == t && pi == i) {
                Some((_, _, pc)) => {
                    if cd.enabled != pc.enabled {
                        out.push(PrefabOverride::Enabled {
                            object,
                            component: t.to_string(),
                            index: *i,
                            enabled: cd.enabled,
                        });
                    }
                    for (f, v) in &cd.fields {
                        if pc.fields.get(f) != Some(v) {
                            out.push(PrefabOverride::Field {
                                object,
                                component: t.to_string(),
                                index: *i,
                                field: f.clone(),
                                value: v.clone(),
                            });
                        }
                    }
                }
                None => out.push(PrefabOverride::AddedComponent {
                    object,
                    component: (*cd).clone(),
                }),
            }
        }
        for (t, i, _) in &theirs {
            if !mine.iter().any(|(mt, mi, _)| mt == t && mi == i) {
                out.push(PrefabOverride::RemovedComponent {
                    object,
                    component: t.to_string(),
                    index: *i,
                });
            }
        }
    }
    for po in &prefab.objects {
        if !present.contains(&po.id) {
            out.push(PrefabOverride::RemovedObject { object: po.id });
        }
    }
    out
}

/// `prefab` with `ovs` applied: what an instance with those overrides should look like.
pub fn apply_overrides(prefab: &SceneFile, ovs: &[PrefabOverride]) -> SceneFile {
    use ObjectProperty as P;
    let mut t = prefab.clone();
    let find = |t: &mut SceneFile, id: u64| t.objects.iter().position(|o| o.id == id);
    let mut removed_components: Vec<(u64, String, usize)> = Vec::new();
    let mut removed_objects = Vec::new();
    for ov in ovs {
        match ov {
            PrefabOverride::Property { object, property } => {
                let Some(i) = find(&mut t, *object) else {
                    continue;
                };
                let o = &mut t.objects[i];
                match property {
                    P::Name(v) => o.name = v.clone(),
                    P::Active(v) => o.active = *v,
                    P::Tag(v) => o.tag = v.clone(),
                    P::Layer(v) => o.layer = *v,
                    P::Static(v) => o.static_flags = *v,
                    P::Position(v) => o.position = *v,
                    P::Rotation(v) => o.rotation = *v,
                    P::Scale(v) => o.scale = *v,
                    P::Parent(v) => o.parent = *v,
                }
            }
            PrefabOverride::Field {
                object,
                component,
                index,
                field,
                value,
            } => {
                let Some(i) = find(&mut t, *object) else {
                    continue;
                };
                if let Some(c) = t.objects[i]
                    .components
                    .iter_mut()
                    .filter(|c| c.type_name == *component)
                    .nth(*index)
                {
                    c.fields.insert(field.clone(), value.clone());
                }
            }
            PrefabOverride::Enabled {
                object,
                component,
                index,
                enabled,
            } => {
                let Some(i) = find(&mut t, *object) else {
                    continue;
                };
                if let Some(c) = t.objects[i]
                    .components
                    .iter_mut()
                    .filter(|c| c.type_name == *component)
                    .nth(*index)
                {
                    c.enabled = *enabled;
                }
            }
            PrefabOverride::AddedComponent { object, component } => {
                if let Some(i) = find(&mut t, *object) {
                    t.objects[i].components.push(component.clone());
                }
            }
            PrefabOverride::RemovedComponent {
                object,
                component,
                index,
            } => removed_components.push((*object, component.clone(), *index)),
            PrefabOverride::AddedObject { data } => t.objects.push(data.clone()),
            PrefabOverride::RemovedObject { object } => removed_objects.push(*object),
        }
    }
    // Remove components highest index first so earlier indices stay valid.
    removed_components.sort_by_key(|r| std::cmp::Reverse(r.2));
    for (object, ty, index) in removed_components {
        let Some(i) = find(&mut t, object) else {
            continue;
        };
        let comps = &mut t.objects[i].components;
        if let Some(pos) = comps
            .iter()
            .enumerate()
            .filter(|(_, c)| c.type_name == ty)
            .nth(index)
            .map(|(p, _)| p)
        {
            comps.remove(pos);
        }
    }
    // Removing an object removes its children.
    let mut gone: HashSet<u64> = removed_objects.into_iter().collect();
    loop {
        let before = gone.len();
        for o in &t.objects {
            if o.parent.is_some_and(|p| gone.contains(&p)) {
                gone.insert(o.id);
            }
        }
        if gone.len() == before {
            break;
        }
    }
    t.objects.retain(|o| !gone.contains(&o.id));
    t
}

/// Rewrites object references in every component field.
fn remap_fields(file: &mut SceneFile, map: &dyn Fn(u64) -> Option<u64>) {
    for od in &mut file.objects {
        for cd in &mut od.components {
            for v in cd.fields.values_mut() {
                *v = remap(std::mem::replace(v, Value::Null), map);
            }
        }
    }
}

impl World {
    /// `root`'s subtree with prefab ids: linked objects keep their prefab id, others get new
    /// ids from `fresh_from`. Returns the file and prefab id → object.
    /// With `keep_external`, references to objects outside the subtree are kept (tagged with
    /// `EXTERNAL_BIT`); otherwise they become null, as in prefab assets.
    pub fn prefab_content(
        &self,
        root: GameObjectId,
        fresh_from: u64,
        keep_external: bool,
    ) -> (SceneFile, HashMap<u64, GameObjectId>) {
        let order = self.descendants(root);
        let mut file = if keep_external {
            self.serialize_keep_external(root)
        } else {
            self.serialize(&[root], "")
        };
        let mut taken = HashSet::new();
        let mut next = fresh_from;
        let mut file_to_prefab = HashMap::new();
        let mut objects = HashMap::new();
        for (i, &id) in order.iter().enumerate() {
            let link = self
                .get(id)
                .and_then(|o| o.prefab_link)
                .filter(|l| !taken.contains(l));
            let pid = match link {
                Some(l) => l,
                None => {
                    while taken.contains(&next) {
                        next += 1;
                    }
                    next
                }
            };
            taken.insert(pid);
            file_to_prefab.insert(i as u64 + 1, pid);
            objects.insert(pid, id);
        }
        for od in &mut file.objects {
            od.id = file_to_prefab[&od.id];
            od.parent = od.parent.and_then(|p| file_to_prefab.get(&p).copied());
            od.prefab_id = None;
        }
        if let Some(r) = file.objects.first_mut() {
            r.parent = None;
            r.prefab = None;
        }
        remap_fields(&mut file, &|r| {
            if r & EXTERNAL_BIT != 0 {
                Some(r)
            } else {
                file_to_prefab.get(&r).copied()
            }
        });
        (file, objects)
    }

    /// Links `root`'s subtree to a prefab: each object gets the prefab id from `objects`.
    pub fn link_prefab(
        &mut self,
        root: GameObjectId,
        path: &str,
        objects: &HashMap<u64, GameObjectId>,
    ) {
        for id in self.descendants(root) {
            if let Some(o) = self.get_mut(id) {
                o.prefab_link = None;
            }
        }
        for (&pid, &id) in objects {
            if let Some(o) = self.get_mut(id) {
                o.prefab_link = Some(pid);
            }
        }
        if let Some(o) = self.get_mut(root) {
            o.prefab_source = Some(path.to_owned());
        }
    }

    /// Spawns a prefab's content as a linked instance under `parent`.
    pub fn spawn_prefab(
        &mut self,
        prefab: &SceneFile,
        path: &str,
        parent: Option<GameObjectId>,
    ) -> Option<GameObjectId> {
        let mut file = prefab.clone();
        for od in &mut file.objects {
            od.prefab_id = Some(od.id);
        }
        if let Some(r) = file.objects.first_mut() {
            r.prefab = Some(path.to_owned());
        }
        let (roots, warnings) = self.spawn(&file, parent);
        for w in warnings {
            oxide_core::debug_log_warning!("{path}: {w}");
        }
        roots.first().copied()
    }

    /// The prefab instance `id` belongs to: the outermost ancestor (or `id` itself) that was
    /// instantiated from a prefab file. Nested instances count as part of the outer one.
    pub fn prefab_instance_root(&self, id: GameObjectId) -> Option<GameObjectId> {
        let mut found = None;
        let mut cur = Some(id);
        while let Some(c) = cur {
            let o = self.get(c)?;
            if o.prefab_source.as_deref().is_some_and(is_prefab_path) {
                found = Some(c);
            }
            cur = o.parent();
        }
        found
    }

    /// Updates instance `root` in place to match `target` (prefab-id space). `objects` maps
    /// prefab ids to the instance's objects (from [`World::prefab_content`]). Objects that
    /// aren't part of the prefab are kept unless `remove_added`.
    pub fn sync_instance(
        &mut self,
        root: GameObjectId,
        target: &SceneFile,
        objects: &HashMap<u64, GameObjectId>,
        remove_added: bool,
    ) {
        let mut map: HashMap<u64, GameObjectId> = objects
            .iter()
            .filter(|(_, id)| self.is_alive(**id))
            .map(|(k, v)| (*k, *v))
            .collect();
        let wanted: HashSet<u64> = target.objects.iter().map(|o| o.id).collect();
        // Objects no longer in the target.
        let doomed: Vec<GameObjectId> = map
            .iter()
            .filter(|(pid, id)| {
                **id != root && !wanted.contains(pid) && (**pid < ADDED_BASE || remove_added)
            })
            .map(|(_, id)| *id)
            .collect();
        for id in doomed {
            if self.is_alive(id) {
                self.destroy_immediate(id);
            }
        }
        map.retain(|_, id| self.is_alive(*id));

        let root_pid = target.objects.first().map(|o| o.id);
        for od in &target.objects {
            let id = match map.get(&od.id) {
                Some(&id) => id,
                None if od.id >= ADDED_BASE => continue,
                None => {
                    let id = self.create(&od.name);
                    if let Some(o) = self.get_mut(id) {
                        o.prefab_link = Some(od.id);
                    }
                    map.insert(od.id, id);
                    id
                }
            };
            if Some(od.id) != root_pid {
                let parent = od.parent.and_then(|p| map.get(&p).copied()).or(Some(root));
                if self.parent(id) != parent {
                    let _ = self.set_parent(id, parent, false);
                }
            }
            if let Some(o) = self.get_mut(id) {
                o.name = od.name.clone();
                o.tag = od.tag.clone();
                o.layer = od.layer;
                o.static_flags = crate::StaticEditorFlags(od.static_flags);
                o.transform.local_position = oxide_math::Vector3::from_array(od.position);
                o.transform.local_rotation =
                    oxide_math::Quaternion::from_array(od.rotation).normalize();
                o.transform.local_scale = oxide_math::Vector3::from_array(od.scale);
            }
            self.sync_components(id, &od.components, &map);
            if self.get(id).is_some_and(|o| o.active_self() != od.active) {
                self.set_active(id, od.active);
            }
        }
    }

    fn sync_components(
        &mut self,
        id: GameObjectId,
        target: &[ComponentData],
        map: &HashMap<u64, GameObjectId>,
    ) {
        let to_runtime = |r: u64| {
            if r & EXTERNAL_BIT != 0 {
                Some(r & !EXTERNAL_BIT)
            } else {
                map.get(&r).map(|id| id.to_bits())
            }
        };
        let current: Vec<(crate::ComponentId, &'static str)> = self
            .get(id)
            .map(|o| {
                o.component_list()
                    .into_iter()
                    .map(|(c, t, _)| (c, t))
                    .collect()
            })
            .unwrap_or_default();
        let mut used = HashSet::new();
        for (ty, index, cd) in indexed(target) {
            let existing = current
                .iter()
                .filter(|(_, t)| *t == ty)
                .nth(index)
                .map(|(c, _)| *c);
            let cid = match existing {
                Some(c) => c,
                None => match self.add_component_by_name(id, ty) {
                    Ok(c) => c,
                    Err(e) => {
                        oxide_core::debug_log_warning!("prefab: {e}");
                        continue;
                    }
                },
            };
            used.insert(cid);
            if let Some(c) = self.component_dyn_mut(id, cid) {
                for (f, v) in &cd.fields {
                    let v = remap(v.clone(), &to_runtime);
                    if c.get_field(f).as_ref() != Some(&v) {
                        let _ = c.set_field(f, v);
                    }
                }
            }
            if self.is_component_enabled(id, cid) != cd.enabled {
                self.set_component_enabled(id, cid, cd.enabled);
            }
        }
        for (cid, _) in current {
            if !used.contains(&cid) {
                self.remove_component_now(id, cid);
            }
        }
    }

    /// Breaks the link between an instance and its prefab (Unity "Unpack Completely").
    pub fn unpack_prefab(&mut self, root: GameObjectId) {
        for id in self.descendants(root) {
            if let Some(o) = self.get_mut(id) {
                o.prefab_link = None;
                if o.prefab_source.as_deref().is_some_and(is_prefab_path) {
                    o.prefab_source = None;
                }
            }
        }
    }
}

pub fn is_prefab_path(p: &str) -> bool {
    p.to_ascii_lowercase().ends_with(".prefab")
}

// ---- files --------------------------------------------------------------------------------

/// Parsed prefab files by path, refreshed when the file changes on disk.
#[derive(Default)]
pub struct PrefabCache {
    files: HashMap<String, (Option<std::time::SystemTime>, SceneFile)>,
}

impl World {
    fn prefab_full_path(&self, path: &str) -> std::path::PathBuf {
        self.resource::<oxide_assets::AssetServer>().full_path(path)
    }

    /// Reads a prefab (resolving variants against their base), loading the assets it uses.
    pub fn load_prefab(&mut self, path: &str) -> Result<SceneFile, String> {
        self.load_prefab_depth(path, 0)
    }

    fn load_prefab_depth(&mut self, path: &str, depth: usize) -> Result<SceneFile, String> {
        if depth > 16 {
            return Err(format!("{path}: prefab variants nest too deeply (cycle?)"));
        }
        let full = self.prefab_full_path(path);
        let mtime = std::fs::metadata(&full).and_then(|m| m.modified()).ok();
        if self.try_resource::<PrefabCache>().is_none() {
            self.resources.insert(PrefabCache::default());
        }
        let cached = self
            .resource::<PrefabCache>()
            .files
            .get(path)
            .filter(|(t, _)| *t == mtime && mtime.is_some())
            .map(|(_, f)| f.clone());
        let mut file = match cached {
            Some(f) => f,
            None => {
                let text = std::fs::read_to_string(&full)
                    .map_err(|e| format!("cannot read prefab {}: {e}", full.display()))?;
                let mut f = SceneFile::from_yaml(&text).map_err(|e| format!("{path}: {e}"))?;
                if let Some(fixed) = self.with_moved_assets_fixed(&f) {
                    f = fixed;
                }
                self.resource_mut::<PrefabCache>()
                    .files
                    .insert(path.to_owned(), (mtime, f.clone()));
                f
            }
        };
        if let Some(base) = file.base.clone() {
            let base_now = self.load_prefab_depth(&base, depth + 1)?;
            if let Some(saved) = file.prefabs.get(&base)
                && *saved != base_now
            {
                let ovs = overrides(&file, saved);
                file.objects = apply_overrides(&base_now, &ovs).objects;
            }
        }
        self.ensure_assets(&file);
        Ok(file)
    }

    /// Loads every asset a file's components reference, logging failures.
    pub fn ensure_assets(&mut self, file: &SceneFile) {
        for w in self.load_assets_of(file) {
            oxide_core::debug_log_warning!("{w}");
        }
    }

    /// Loads every asset a file's components reference. Returns what failed to load.
    pub fn load_assets_of(&mut self, file: &SceneFile) -> Vec<String> {
        fn walk(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Asset(p) => out.push(p.clone()),
                Value::List(items) => items.iter().for_each(|i| walk(i, out)),
                Value::Object(m) => m.values().for_each(|i| walk(i, out)),
                _ => {}
            }
        }
        let mut paths = Vec::new();
        for o in &file.objects {
            for c in &o.components {
                c.fields.values().for_each(|v| walk(v, &mut paths));
            }
        }
        paths.sort();
        paths.dedup();
        let Some(assets) = self.try_resource_mut::<oxide_assets::AssetServer>() else {
            return Vec::new();
        };
        paths
            .into_iter()
            .filter_map(|p| assets.ensure_loaded(&p).err())
            .collect()
    }

    fn write_prefab(&mut self, path: &str, file: &SceneFile) -> Result<(), String> {
        let full = self.prefab_full_path(path);
        if let Some(dir) = full.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut file = file.clone();
        self.record_guids(&mut file);
        std::fs::write(&full, file.to_yaml()?)
            .map_err(|e| format!("cannot write {}: {e}", full.display()))?;
        if let Some(c) = self.try_resource_mut::<PrefabCache>() {
            c.files.remove(path);
        }
        Ok(())
    }

    /// Unity `Instantiate(prefab)` for a prefab file.
    pub fn instantiate_prefab(
        &mut self,
        path: &str,
        parent: Option<GameObjectId>,
    ) -> Result<GameObjectId, String> {
        let file = self.load_prefab(path)?;
        self.spawn_prefab(&file, path, parent)
            .ok_or_else(|| format!("{path} has no objects"))
    }

    /// Saves `root` as a prefab asset at `path` and makes `root` an instance of it (Unity:
    /// dragging an object into the Project window). Replaces any file there.
    pub fn create_prefab(&mut self, root: GameObjectId, path: &str) -> Result<(), String> {
        if !self.is_alive(root) {
            return Err("no such GameObject".into());
        }
        // A new asset: fresh ids for everything.
        for id in self.descendants(root) {
            if let Some(o) = self.get_mut(id) {
                o.prefab_link = None;
            }
        }
        let (mut file, objects) = self.prefab_content(root, 1, false);
        file.name = prefab_name(path);
        for w in self.persist_runtime_assets(&mut file, &asset_folder(path)) {
            oxide_core::debug_log_warning!("{w}");
        }
        self.write_prefab(path, &file)?;
        self.link_prefab(root, path, &objects);
        Ok(())
    }

    /// Saves instance `root` (based on prefab `base`) as a new variant prefab at `path`, and
    /// makes `root` an instance of the variant.
    pub fn create_prefab_variant(&mut self, root: GameObjectId, path: &str) -> Result<(), String> {
        let base = self
            .get(root)
            .and_then(|o| o.prefab_source.clone())
            .filter(|p| is_prefab_path(p))
            .ok_or("the object is not a prefab instance")?;
        let base_file = self.load_prefab(&base)?;
        let (mut file, objects) = self.prefab_content(root, ADDED_BASE, false);
        file.name = prefab_name(path);
        for w in self.persist_runtime_assets(&mut file, &asset_folder(path)) {
            oxide_core::debug_log_warning!("{w}");
        }
        file.base = Some(base.clone());
        file.prefabs.insert(base, base_file);
        self.write_prefab(path, &file)?;
        self.link_prefab(root, path, &objects);
        Ok(())
    }

    /// Instance root of `id`, its prefab path, and its current content in prefab-id space.
    fn instance_parts(
        &self,
        id: GameObjectId,
    ) -> Result<(GameObjectId, String, SceneFile, HashMap<u64, GameObjectId>), String> {
        let root = self
            .prefab_instance_root(id)
            .ok_or("the object is not part of a prefab instance")?;
        let path = self
            .get(root)
            .and_then(|o| o.prefab_source.clone())
            .ok_or("missing prefab path")?;
        let (content, objects) = self.prefab_content(root, ADDED_BASE, true);
        Ok((root, path, content, objects))
    }

    /// Overrides of the prefab instance that `id` belongs to, without root placement.
    pub fn prefab_overrides(&mut self, id: GameObjectId) -> Result<Vec<PrefabOverride>, String> {
        let (_, path, content, _) = self.instance_parts(id)?;
        let prefab = self.load_prefab(&path)?;
        let root_pid = content.objects.first().map_or(0, |o| o.id);
        Ok(overrides(&content, &prefab)
            .into_iter()
            .filter(|o| !o.is_root_placement(root_pid))
            .collect())
    }

    /// Reverts every override of the instance `id` belongs to (keeps where it is placed).
    pub fn revert_prefab_instance(&mut self, id: GameObjectId) -> Result<(), String> {
        let (root, path, content, objects) = self.instance_parts(id)?;
        let prefab = self.load_prefab(&path)?;
        let root_pid = content.objects.first().map_or(0, |o| o.id);
        let keep: Vec<_> = overrides(&content, &prefab)
            .into_iter()
            .filter(|o| o.is_root_placement(root_pid))
            .collect();
        let target = apply_overrides(&prefab, &keep);
        self.sync_instance(root, &target, &objects, true);
        Ok(())
    }

    /// Writes the instance `id` belongs to back into its prefab (Unity "Apply All"), then
    /// updates the other instances in the scene, keeping their own overrides. Returns how
    /// many other instances were updated.
    pub fn apply_prefab_instance(&mut self, id: GameObjectId) -> Result<usize, String> {
        let (root, path, _, _) = self.instance_parts(id)?;
        let old = self.load_prefab(&path)?;
        let raw_old = self.read_raw_prefab(&path)?;
        let max_id = old
            .objects
            .iter()
            .map(|o| o.id)
            .filter(|&i| i < ADDED_BASE)
            .max()
            .unwrap_or(0);
        let (mut new, objects) = self.prefab_content(root, max_id + 1, false);
        // The root keeps the prefab's own placement; the instance's is an override.
        if let (Some(n), Some(o)) = (new.objects.first_mut(), old.objects.first()) {
            n.position = o.position;
            n.rotation = o.rotation;
        }
        new.name = prefab_name(&path);
        for w in self.persist_runtime_assets(&mut new, &asset_folder(&path)) {
            oxide_core::debug_log_warning!("{w}");
        }
        // Variants stay variants: keep the base link and its snapshot.
        new.base = raw_old.base.clone();
        new.prefabs = raw_old.prefabs.clone();
        self.write_prefab(&path, &new)?;
        self.link_prefab(root, &path, &objects);

        let others: Vec<GameObjectId> = self
            .hierarchy_order()
            .into_iter()
            .filter(|&o| {
                o != root
                    && self.get(o).and_then(|x| x.prefab_source.as_deref()) == Some(path.as_str())
            })
            .collect();
        for other in &others {
            let (content, objs) = self.prefab_content(*other, ADDED_BASE, true);
            let ovs = overrides(&content, &old);
            let target = apply_overrides(&new, &ovs);
            self.sync_instance(*other, &target, &objs, false);
        }
        Ok(others.len())
    }

    fn read_raw_prefab(&self, path: &str) -> Result<SceneFile, String> {
        let full = self.prefab_full_path(path);
        let text = std::fs::read_to_string(&full)
            .map_err(|e| format!("cannot read prefab {}: {e}", full.display()))?;
        let f = SceneFile::from_yaml(&text)?;
        Ok(self.with_moved_assets_fixed(&f).unwrap_or(f))
    }

    /// A copy of `file` with references to assets that moved outside the editor pointed at
    /// their new paths (by GUID), or `None` if nothing moved.
    pub fn with_moved_assets_fixed(&self, file: &SceneFile) -> Option<SceneFile> {
        let db = &self.try_resource::<oxide_assets::AssetServer>()?.database;
        let mut f = file.clone();
        (f.fix_moved_assets(db) > 0).then_some(f)
    }

    /// Records the GUIDs of the assets `file` refers to (before writing it).
    pub fn record_guids(&self, file: &mut SceneFile) {
        if let Some(a) = self.try_resource::<oxide_assets::AssetServer>()
            && a.database.is_scanned()
        {
            file.record_guids(&a.database);
        }
    }

    /// Prefab contents used by instances in this scene (embedded when the scene is saved).
    pub fn prefab_snapshots(&mut self) -> BTreeMap<String, SceneFile> {
        let paths: HashSet<String> = self
            .objects()
            .filter_map(|(_, o)| o.prefab_source.clone())
            .filter(|p| is_prefab_path(p))
            .collect();
        let mut out = BTreeMap::new();
        for p in paths {
            match self.load_prefab(&p) {
                Ok(f) => {
                    out.insert(p, f);
                }
                Err(e) => oxide_core::debug_log_warning!("{e}"),
            }
        }
        out
    }

    /// After loading a scene: updates outermost prefab instances whose prefab changed since
    /// the scene was saved (`saved` = the scene's embedded snapshots).
    pub fn update_prefab_instances(&mut self, saved: &BTreeMap<String, SceneFile>) -> usize {
        let roots: Vec<(GameObjectId, String)> = self
            .hierarchy_order()
            .into_iter()
            .filter_map(|id| {
                let p = self.get(id)?.prefab_source.clone()?;
                (is_prefab_path(&p) && self.prefab_instance_root(id) == Some(id)).then_some((id, p))
            })
            .collect();
        let mut updated = 0;
        for (root, path) in roots {
            let Some(snapshot) = saved.get(&path) else {
                continue;
            };
            let now = match self.load_prefab(&path) {
                Ok(f) => f,
                Err(e) => {
                    oxide_core::debug_log_warning!("{e}");
                    continue;
                }
            };
            if now == *snapshot {
                continue;
            }
            let (content, objs) = self.prefab_content(root, ADDED_BASE, true);
            let ovs = overrides(&content, snapshot);
            let target = apply_overrides(&now, &ovs);
            self.sync_instance(root, &target, &objs, false);
            updated += 1;
        }
        updated
    }
}

/// Where assets extracted for a scene or prefab at `path` go: `Materials/` next to it.
pub fn asset_folder(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/Materials"),
        None => "Materials".into(),
    }
}

fn prefab_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Prefab")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Light, register_builtin};
    use oxide_math::Vector3;

    /// A world whose project folder is a fresh temp dir.
    fn world(tag: &str) -> (World, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("oxide-prefab-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = World::new();
        register_builtin(&mut w.registry);
        w.run_scripts = false;
        w.resources
            .insert(oxide_assets::AssetServer::new(dir.clone()));
        (w, dir)
    }

    /// "Lamp" root with a "Bulb" child carrying a Light.
    fn lamp(w: &mut World) -> GameObjectId {
        let root = w.create("Lamp");
        let bulb = w.create("Bulb");
        w.set_parent(bulb, Some(root), false).unwrap();
        w.transform_mut(bulb).unwrap().local_position = Vector3::new(0.0, 2.0, 0.0);
        w.add_component(
            bulb,
            Light {
                intensity: 1.0,
                range: 5.0,
                ..Default::default()
            },
        );
        root
    }

    fn bulb_light(w: &World, root: GameObjectId) -> &Light {
        let bulb = w.find_child(root, "Bulb").unwrap();
        w.get_component::<Light>(bulb).unwrap()
    }

    #[test]
    fn apply_updates_other_instances_but_keeps_their_overrides() {
        let (mut w, dir) = world("apply");
        let src = lamp(&mut w);
        w.create_prefab(src, "Assets/Lamp.prefab").unwrap();
        assert_eq!(
            w.get(src).unwrap().prefab_source.as_deref(),
            Some("Assets/Lamp.prefab")
        );

        let a = w.instantiate_prefab("Assets/Lamp.prefab", None).unwrap();
        let b = w.instantiate_prefab("Assets/Lamp.prefab", None).unwrap();
        w.set_position(a, Vector3::new(10.0, 0.0, 0.0));
        // Instance A overrides the range; placement alone is not an override.
        let bulb_a = w.find_child(a, "Bulb").unwrap();
        w.get_component_mut::<Light>(bulb_a).unwrap().range = 9.0;
        let ovs = w.prefab_overrides(a).unwrap();
        assert_eq!(ovs.len(), 1, "{ovs:?}");
        assert!(matches!(&ovs[0], PrefabOverride::Field { field, .. } if field == "range"));
        assert!(w.prefab_overrides(b).unwrap().is_empty());

        // B changes the intensity and adds a child, then applies.
        let bulb_b = w.find_child(b, "Bulb").unwrap();
        w.get_component_mut::<Light>(bulb_b).unwrap().intensity = 3.0;
        let extra = w.create("Shade");
        w.set_parent(extra, Some(b), false).unwrap();
        let n = w.apply_prefab_instance(b).unwrap();
        assert_eq!(n, 2, "src and a are the other instances");
        assert!(w.prefab_overrides(b).unwrap().is_empty());

        // A got the new intensity and child, kept its range and placement and its object ids.
        assert!(w.is_alive(bulb_a));
        assert_eq!(bulb_light(&w, a).intensity, 3.0);
        assert_eq!(bulb_light(&w, a).range, 9.0);
        assert!(w.find_child(a, "Shade").is_some());
        assert_eq!(w.position(a), Vector3::new(10.0, 0.0, 0.0));

        // Revert drops A's override but not where it is.
        w.revert_prefab_instance(bulb_a).unwrap();
        assert_eq!(bulb_light(&w, a).range, 5.0);
        assert_eq!(w.position(a), Vector3::new(10.0, 0.0, 0.0));
        assert!(w.prefab_overrides(a).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn removed_and_added_components_are_overrides() {
        let (mut w, dir) = world("components");
        let src = lamp(&mut w);
        w.create_prefab(src, "Assets/Lamp.prefab").unwrap();
        let a = w.instantiate_prefab("Assets/Lamp.prefab", None).unwrap();
        let bulb = w.find_child(a, "Bulb").unwrap();
        let cid = w.component_id::<Light>(bulb).unwrap();
        w.remove_component_now(bulb, cid);
        w.add_component(a, crate::Camera::default());
        let ovs = w.prefab_overrides(a).unwrap();
        assert!(ovs.iter().any(|o| matches!(o, PrefabOverride::RemovedComponent { component, .. } if component == "Light")));
        assert!(ovs.iter().any(|o| matches!(o, PrefabOverride::AddedComponent { component, .. } if component.type_name == "Camera")));
        w.revert_prefab_instance(a).unwrap();
        assert!(w.get_component::<Light>(bulb).is_some());
        assert!(w.get_component::<crate::Camera>(a).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn variants_follow_their_base() {
        let (mut w, dir) = world("variant");
        let src = lamp(&mut w);
        w.create_prefab(src, "Assets/Lamp.prefab").unwrap();
        // Variant: a red lamp.
        let v = w.instantiate_prefab("Assets/Lamp.prefab", None).unwrap();
        let bulb = w.find_child(v, "Bulb").unwrap();
        w.get_component_mut::<Light>(bulb).unwrap().color = oxide_math::Color::RED;
        w.create_prefab_variant(v, "Assets/Red Lamp.prefab")
            .unwrap();

        // The base gets brighter; the variant picks that up and stays red.
        let bulb_src = w.find_child(src, "Bulb").unwrap();
        w.get_component_mut::<Light>(bulb_src).unwrap().intensity = 4.0;
        w.apply_prefab_instance(src).unwrap();
        let red = w
            .instantiate_prefab("Assets/Red Lamp.prefab", None)
            .unwrap();
        assert_eq!(bulb_light(&w, red).intensity, 4.0);
        assert_eq!(bulb_light(&w, red).color, oxide_math::Color::RED);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scenes_pick_up_prefab_changes_on_load() {
        let (mut w, dir) = world("scene");
        let src = lamp(&mut w);
        w.create_prefab(src, "Assets/Lamp.prefab").unwrap();
        let a = w.instantiate_prefab("Assets/Lamp.prefab", None).unwrap();
        let bulb = w.find_child(a, "Bulb").unwrap();
        w.get_component_mut::<Light>(bulb).unwrap().range = 7.0;
        // Save the scene with its prefab snapshots.
        let mut scene = w.serialize(&[a], "Level");
        scene.prefabs = w.prefab_snapshots();

        // Someone edits the prefab afterwards.
        w.get_component_mut::<Light>(w.find_child(src, "Bulb").unwrap())
            .unwrap()
            .intensity = 2.5;
        w.apply_prefab_instance(src).unwrap();

        // Load the scene into a fresh world sharing the project.
        let mut w2 = World::new();
        register_builtin(&mut w2.registry);
        w2.run_scripts = false;
        w2.resources
            .insert(oxide_assets::AssetServer::new(dir.clone()));
        let (roots, _) = w2.spawn(&scene, None);
        assert_eq!(bulb_light(&w2, roots[0]).intensity, 1.0, "saved data");
        assert_eq!(w2.update_prefab_instances(&scene.prefabs), 1);
        assert_eq!(
            bulb_light(&w2, roots[0]).intensity,
            2.5,
            "prefab change arrived"
        );
        assert_eq!(bulb_light(&w2, roots[0]).range, 7.0, "override kept");
        let _ = std::fs::remove_dir_all(dir);
    }
}
