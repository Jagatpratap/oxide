//! Scene and prefab files, plus `Instantiate`.
//!
//! A scene file lists GameObjects with a file-local id, their parent, transform and
//! components (type name + serialized fields), like Unity's YAML scene format. References
//! between objects are stored as file ids and remapped on load, so they survive
//! save/load and instantiation. References: <https://docs.unity3d.com/Manual/CreatingScenes.html>,
//! <https://docs.unity3d.com/Manual/Prefabs.html>

use crate::id::GameObjectId;
use crate::world::World;
use oxide_core::reflect::{Value, ValueKind};
use oxide_math::{Quaternion, Vector3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ComponentData {
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
}

fn yes() -> bool {
    true
}

fn untagged() -> String {
    "Untagged".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObjectData {
    /// File-local id (Unity's `fileID`).
    pub id: u64,
    pub name: String,
    #[serde(default = "yes")]
    pub active: bool,
    #[serde(default = "untagged")]
    pub tag: String,
    #[serde(default)]
    pub layer: u8,
    #[serde(default, rename = "static")]
    pub is_static: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefab: Option<String>,
    /// Id of the corresponding object inside its prefab asset (Unity's
    /// `m_CorrespondingSourceObject`), for objects that belong to a prefab instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefab_id: Option<u64>,
    #[serde(default)]
    pub components: Vec<ComponentData>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct SceneFile {
    #[serde(default)]
    pub format: u32,
    pub name: String,
    /// Prefab variants: the base prefab this one is built on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub objects: Vec<ObjectData>,
    /// Contents of the prefabs used here, as they were when this file was saved. On load,
    /// instances are updated with whatever changed in the prefab since then.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prefabs: BTreeMap<String, SceneFile>,
    /// Scene lighting and environment (`RenderSettings` fields).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub render_settings: BTreeMap<String, Value>,
}

pub const SCENE_FORMAT: u32 = 1;

impl SceneFile {
    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml_ng::to_string(self).map_err(|e| e.to_string())
    }

    pub fn from_yaml(s: &str) -> Result<Self, String> {
        serde_yaml_ng::from_str(s).map_err(|e| e.to_string())
    }
}

/// Rewrites object references inside a value using `map`; unmapped references become null.
pub(crate) fn remap(v: Value, map: &dyn Fn(u64) -> Option<u64>) -> Value {
    match v {
        Value::ObjectRef(b) => match map(b) {
            Some(n) => Value::ObjectRef(n),
            None => Value::Null,
        },
        Value::List(items) => Value::List(items.into_iter().map(|i| remap(i, map)).collect()),
        other => other,
    }
}

fn has_refs(kind: ValueKind) -> bool {
    matches!(kind, ValueKind::ObjectRef | ValueKind::List)
}

impl World {
    /// Serializes the subtrees under `roots` (or the whole scene if `roots` is empty).
    pub fn serialize(&self, roots: &[GameObjectId], name: &str) -> SceneFile {
        let order: Vec<GameObjectId> = if roots.is_empty() {
            self.hierarchy_order()
        } else {
            roots.iter().flat_map(|&r| self.descendants(r)).collect()
        };
        let file_ids: HashMap<GameObjectId, u64> = order
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i as u64 + 1))
            .collect();
        let to_file = |bits: u64| file_ids.get(&GameObjectId::from_bits(bits)).copied();
        let objects = order
            .iter()
            .filter_map(|&id| {
                let o = self.get(id)?;
                let t = &o.transform;
                let components = o
                    .components
                    .iter()
                    .filter(|e| !e.destroy_pending)
                    .filter_map(|e| {
                        let c = e.comp.as_deref()?;
                        // A missing script saves as the script it stands for.
                        if let Some(m) = crate::component::downcast_ref::<crate::MissingScript>(c) {
                            return Some(ComponentData {
                                type_name: m.script.clone(),
                                enabled: e.enabled,
                                fields: m
                                    .data
                                    .iter()
                                    .map(|(k, v)| (k.clone(), remap(v.clone(), &to_file)))
                                    .collect(),
                            });
                        }
                        let fields = c
                            .type_info()
                            .fields
                            .iter()
                            .filter_map(|f| {
                                let v = c.get_field(f.name)?;
                                let v = if has_refs(f.kind) {
                                    remap(v, &to_file)
                                } else {
                                    v
                                };
                                Some((f.name.to_string(), v))
                            })
                            .collect();
                        Some(ComponentData {
                            type_name: e.type_name.to_string(),
                            enabled: e.enabled,
                            fields,
                        })
                    })
                    .collect();
                Some(ObjectData {
                    id: file_ids[&id],
                    name: o.name.clone(),
                    active: o.active_self(),
                    tag: o.tag.clone(),
                    layer: o.layer,
                    is_static: o.is_static,
                    parent: o.parent().and_then(|p| file_ids.get(&p).copied()),
                    position: t.local_position.to_array(),
                    rotation: t.local_rotation.to_array(),
                    scale: t.local_scale.to_array(),
                    prefab: o.prefab_source.clone(),
                    prefab_id: o.prefab_link,
                    components,
                })
            })
            .collect();
        SceneFile {
            format: SCENE_FORMAT,
            name: name.into(),
            base: None,
            objects,
            prefabs: BTreeMap::new(),
            render_settings: BTreeMap::new(),
        }
    }

    /// Creates the objects in `file` (additively) under `parent`. Returns the new root
    /// objects. Awake/OnEnable run after the whole set is built, like Unity.
    /// Unknown component types are reported in the returned warnings and skipped.
    pub fn spawn(
        &mut self,
        file: &SceneFile,
        parent: Option<GameObjectId>,
    ) -> (Vec<GameObjectId>, Vec<String>) {
        let mut warnings = Vec::new();
        let mut map: HashMap<u64, GameObjectId> = HashMap::new();
        let mut roots = Vec::new();
        let mut active = Vec::new();
        for od in &file.objects {
            let id = self.create(&od.name);
            map.insert(od.id, id);
            let o = self.get_mut(id).expect("created");
            o.tag = od.tag.clone();
            o.layer = od.layer;
            o.is_static = od.is_static;
            o.prefab_source = od.prefab.clone();
            o.prefab_link = od.prefab_id;
            o.transform.local_position = Vector3::from_array(od.position);
            o.transform.local_rotation = Quaternion::from_array(od.rotation).normalize();
            o.transform.local_scale = Vector3::from_array(od.scale);
            active.push((id, od.active));
        }
        // Build the hierarchy with every object inactive so no Awake runs early.
        for &(id, _) in &active {
            self.set_active(id, false);
        }
        for od in &file.objects {
            let id = map[&od.id];
            let p = match od.parent {
                Some(fp) => map.get(&fp).copied(),
                None => parent,
            };
            if od.parent.is_none() || p.is_none() {
                roots.push(id);
            }
            if let Some(p) = p {
                let _ = self.set_parent(id, Some(p), false);
            }
        }
        let to_runtime = |f: u64| {
            if f & EXTERNAL_BIT != 0 {
                Some(f & !EXTERNAL_BIT)
            } else {
                map.get(&f).map(|id| id.to_bits())
            }
        };
        for od in &file.objects {
            let id = map[&od.id];
            for cd in &od.components {
                let Some(mut comp) = self.registry.create(&cd.type_name) else {
                    warnings.push(format!(
                        "{}: script `{}` is not loaded; its data is kept",
                        od.name, cd.type_name
                    ));
                    let missing = crate::MissingScript {
                        script: cd.type_name.clone(),
                        data: cd
                            .fields
                            .iter()
                            .map(|(k, v)| (k.clone(), remap(v.clone(), &to_runtime)))
                            .collect(),
                        awoken: false,
                        started: false,
                    };
                    if let Some(cid) = self.add_component_boxed(id, Box::new(missing))
                        && !cd.enabled
                    {
                        self.set_component_enabled(id, cid, false);
                    }
                    continue;
                };
                for (k, v) in &cd.fields {
                    let v = remap(v.clone(), &to_runtime);
                    if let Err(e) = comp.set_field(k, v) {
                        warnings.push(format!("{}.{}: {e}", od.name, cd.type_name));
                    }
                }
                if let Some(cid) = self.add_component_boxed(id, comp)
                    && !cd.enabled
                {
                    self.set_component_enabled(id, cid, false);
                }
            }
        }
        // Activate top-down so parents wake before children.
        for &(id, a) in &active {
            if a {
                self.set_active(id, true);
            }
        }
        (roots, warnings)
    }

    /// Unity `Instantiate(original)`: deep-copies the object and its children (with
    /// "(Clone)" appended to the root's name). References inside the copied hierarchy are
    /// remapped to the copies; references outside it are kept.
    pub fn instantiate(&mut self, original: GameObjectId) -> Option<GameObjectId> {
        if !self.is_alive(original) {
            return None;
        }
        let mut file = self.serialize_keep_external(original);
        if let Some(root) = file.objects.first_mut() {
            root.name.push_str("(Clone)");
            root.parent = None;
        }
        let parent = self.parent(original);
        let (roots, _) = self.spawn(&file, parent);
        roots.first().copied()
    }

    /// Serializes `root`'s subtree, keeping references to objects outside it (tagged with
    /// `EXTERNAL_BIT`) instead of nulling them.
    pub(crate) fn serialize_keep_external(&self, root: GameObjectId) -> SceneFile {
        let subtree = self.descendants(root);
        let mut file = self.serialize(&[root], "");
        for (od, &src) in file.objects.iter_mut().zip(&subtree) {
            let Some(o) = self.get(src) else { continue };
            for (cd, e) in od.components.iter_mut().zip(
                o.components
                    .iter()
                    .filter(|e| !e.destroy_pending && e.comp.is_some()),
            ) {
                let Some(c) = e.comp.as_deref() else { continue };
                for f in c.type_info().fields.iter().filter(|f| has_refs(f.kind)) {
                    let Some(orig) = c.get_field(f.name) else {
                        continue;
                    };
                    let fixed = merge_external(cd.fields.get(f.name).cloned(), orig);
                    cd.fields.insert(f.name.to_string(), fixed);
                }
            }
        }
        file
    }
}

/// External object references (outside the copied subtree) were nulled by `serialize`;
/// restore them from the original value, tagging them so `spawn` keeps them.
fn merge_external(serialized: Option<Value>, original: Value) -> Value {
    match (serialized, original) {
        (Some(Value::Null), Value::ObjectRef(b)) => Value::ObjectRef(b | EXTERNAL_BIT),
        (Some(Value::List(s)), Value::List(o)) => Value::List(
            s.into_iter()
                .zip(o)
                .map(|(s, o)| merge_external(Some(s), o))
                .collect(),
        ),
        (Some(s), _) => s,
        (None, o) => o,
    }
}

/// Marks an object reference that points outside a copied hierarchy (kept verbatim).
pub(crate) const EXTERNAL_BIT: u64 = 1 << 63;

impl World {
    /// Writes materials created in code (`runtime:Material/...`) that `file` uses to `.mat`
    /// files in `folder`, and points `file` and the scene's components at them (Unity keeps
    /// materials as assets, so a saved scene or prefab must not depend on code having run).
    /// Returns warnings for other code-created assets, which aren't saved yet.
    pub fn persist_runtime_assets(&mut self, file: &mut SceneFile, folder: &str) -> Vec<String> {
        fn collect(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Asset(p) if p.starts_with("runtime:") => out.push(p.clone()),
                Value::List(items) => items.iter().for_each(|i| collect(i, out)),
                Value::Object(m) => m.values().for_each(|i| collect(i, out)),
                _ => {}
            }
        }
        fn rewrite(v: Value, map: &HashMap<String, String>) -> Value {
            match v {
                Value::Asset(p) => Value::Asset(map.get(&p).cloned().unwrap_or(p)),
                Value::List(items) => {
                    Value::List(items.into_iter().map(|i| rewrite(i, map)).collect())
                }
                Value::Object(m) => {
                    Value::Object(m.into_iter().map(|(k, i)| (k, rewrite(i, map))).collect())
                }
                other => other,
            }
        }
        let mut used = Vec::new();
        for o in &file.objects {
            for c in &o.components {
                c.fields.values().for_each(|v| collect(v, &mut used));
            }
        }
        used.sort();
        used.dedup();
        let mut map = HashMap::new();
        let mut warnings = Vec::new();
        let mut taken = std::collections::HashSet::new();
        for p in used {
            let assets = self.resource_mut::<oxide_assets::AssetServer>();
            let Some(m) = assets
                .materials
                .handle_of(&p)
                .and_then(|h| assets.materials.get(&h).cloned())
            else {
                warnings.push(format!(
                    "{p} was created in code and is not saved with the scene; it will be missing when the scene is reloaded"
                ));
                continue;
            };
            let base: String = m
                .name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || " -_".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let base = if base.trim().is_empty() {
                "Material".to_owned()
            } else {
                base.trim().to_owned()
            };
            let mut path = format!("{folder}/{base}.mat");
            let mut n = 1;
            while taken.contains(&path) || assets.full_path(&path).exists() {
                path = format!("{folder}/{base} {n}.mat");
                n += 1;
            }
            match assets.save_material(&path, &m) {
                Ok(_) => {
                    taken.insert(path.clone());
                    map.insert(p, path);
                }
                Err(e) => warnings.push(e),
            }
        }
        if map.is_empty() {
            return warnings;
        }
        for o in &mut file.objects {
            for c in &mut o.components {
                for v in c.fields.values_mut() {
                    *v = rewrite(std::mem::replace(v, Value::Null), &map);
                }
            }
        }
        // The open scene now uses the saved files too.
        for id in self.hierarchy_order() {
            let cids: Vec<_> = self
                .get(id)
                .map(|o| o.components.iter().map(|c| c.id).collect())
                .unwrap_or_default();
            for cid in cids {
                let Some(c) = self.component_dyn_mut(id, cid) else {
                    continue;
                };
                let fields: Vec<(&'static str, Value)> = c.fields().into_iter().collect();
                for (k, v) in fields {
                    let mut found = Vec::new();
                    collect(&v, &mut found);
                    if found.iter().any(|f| map.contains_key(f)) {
                        let _ = c.set_field(k, rewrite(v, &map));
                    }
                }
            }
        }
        warnings
    }
}
