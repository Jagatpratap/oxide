//! The [`World`]: every GameObject in the loaded scenes, their Transform hierarchy and
//! components, plus engine resources.
//!
//! References: <https://docs.unity3d.com/Manual/GameObjects.html>,
//! <https://docs.unity3d.com/Manual/class-Transform.html>,
//! <https://docs.unity3d.com/Manual/execution-order.html>

use crate::component::{
    Component, ComponentRegistry, component_type_id, downcast_mut, downcast_ref,
};
use crate::ctx::Ctx;
use crate::id::{ComponentId, GameObjectId};
use crate::resources::Resources;
use oxide_math::{Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3};
use std::any::TypeId;

/// Local position/rotation/scale relative to the parent (Unity `Transform` local values).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub local_position: Vector3,
    pub local_rotation: Quaternion,
    pub local_scale: Vector3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            local_position: Vector3::ZERO,
            local_rotation: Quaternion::IDENTITY,
            local_scale: Vector3::ONE,
        }
    }
}

impl Transform {
    pub fn local_matrix(&self) -> Matrix4x4 {
        Matrix4x4::trs(self.local_position, self.local_rotation, self.local_scale)
    }
}

/// Unity `StaticEditorFlags`: which systems may treat a GameObject as never moving.
/// The Inspector's "Static" checkbox sets all of them.
/// <https://docs.unity3d.com/Manual/StaticObjects.html>
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StaticEditorFlags(pub u32);

impl StaticEditorFlags {
    pub const NOTHING: Self = Self(0);
    pub const CONTRIBUTE_GI: Self = Self(1);
    /// Hides what is behind it (occlusion culling).
    pub const OCCLUDER_STATIC: Self = Self(2);
    pub const BATCHING_STATIC: Self = Self(4);
    pub const NAVIGATION_STATIC: Self = Self(8);
    /// Can be hidden by occluders.
    pub const OCCLUDEE_STATIC: Self = Self(16);
    pub const OFF_MESH_LINK_GENERATION: Self = Self(32);
    pub const REFLECTION_PROBE_STATIC: Self = Self(64);
    pub const EVERYTHING: Self = Self(127);
    /// Every flag with its Inspector name.
    pub const ALL: [(Self, &'static str); 7] = [
        (Self::CONTRIBUTE_GI, "Contribute GI"),
        (Self::OCCLUDER_STATIC, "Occluder Static"),
        (Self::BATCHING_STATIC, "Batching Static"),
        (Self::NAVIGATION_STATIC, "Navigation Static"),
        (Self::OCCLUDEE_STATIC, "Occludee Static"),
        (Self::OFF_MESH_LINK_GENERATION, "Off Mesh Link Generation"),
        (Self::REFLECTION_PROBE_STATIC, "Reflection Probe Static"),
    ];

    pub fn contains(self, f: Self) -> bool {
        self.0 & f.0 == f.0
    }
    pub fn set(&mut self, f: Self, on: bool) {
        if on { self.0 |= f.0 } else { self.0 &= !f.0 }
    }
    /// Flag names, for tools and the Inspector.
    pub fn names(self) -> Vec<&'static str> {
        Self::ALL
            .iter()
            .filter(|(f, _)| self.contains(*f))
            .map(|(_, n)| *n)
            .collect()
    }
    /// Parses a flag by its name (`"Occluder Static"`, `"occluder_static"`, ...).
    pub fn from_name(name: &str) -> Option<Self> {
        let norm = |s: &str| s.to_ascii_lowercase().replace([' ', '_'], "");
        let n = norm(name);
        if n == "everything" {
            return Some(Self::EVERYTHING);
        }
        Self::ALL
            .iter()
            .find(|(_, s)| norm(s) == n)
            .map(|(f, _)| *f)
    }
}

/// Unity `Space`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    World,
    Self_,
}

pub(crate) struct ComponentEntry {
    pub id: ComponentId,
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub enabled: bool,
    pub awoken: bool,
    pub started: bool,
    /// `on_enable` delivered and not yet matched by `on_disable`.
    pub live: bool,
    pub destroy_pending: bool,
    /// `None` while the component is running one of its own callbacks.
    pub comp: Option<Box<dyn Component>>,
}

pub struct GameObject {
    pub name: String,
    pub tag: String,
    /// 0..31, names in [`LayerNames`].
    pub layer: u8,
    pub static_flags: StaticEditorFlags,
    pub transform: Transform,
    pub dont_destroy_on_load: bool,
    active_self: bool,
    active_in_hierarchy: bool,
    parent: Option<GameObjectId>,
    children: Vec<GameObjectId>,
    pub(crate) components: Vec<ComponentEntry>,
    destroy_pending: bool,
    /// Prefab asset this object was instantiated from (root of the instance only).
    pub prefab_source: Option<String>,
    /// For objects in a prefab instance: the id of the matching object in the prefab asset.
    pub prefab_link: Option<u64>,
}

impl GameObject {
    pub fn active_self(&self) -> bool {
        self.active_self
    }
    pub fn active_in_hierarchy(&self) -> bool {
        self.active_in_hierarchy
    }
    pub fn parent(&self) -> Option<GameObjectId> {
        self.parent
    }
    pub fn children(&self) -> &[GameObjectId] {
        &self.children
    }
    pub fn component_count(&self) -> usize {
        self.components.len()
    }
    /// `(id, type name, enabled)` for each component, in Inspector order.
    pub fn component_list(&self) -> Vec<(ComponentId, &'static str, bool)> {
        self.components
            .iter()
            .filter(|c| !c.destroy_pending)
            .map(|c| (c.id, c.type_name, c.enabled))
            .collect()
    }
    /// Unity `GameObject.isStatic`: any static flag set.
    pub fn is_static(&self) -> bool {
        self.static_flags.0 != 0
    }
    pub fn compare_tag(&self, tag: &str) -> bool {
        self.tag == tag
    }
}

/// A component taken out for a script reload (see [`World::take_components_for_reload`]).
#[derive(Clone, Debug)]
pub struct ReloadedComponent {
    pub object: GameObjectId,
    pub index: usize,
    pub type_name: String,
    pub enabled: bool,
    pub awoken: bool,
    pub started: bool,
    pub fields: std::collections::BTreeMap<String, oxide_core::reflect::Value>,
}

/// Unity's Tags & Layers settings.
pub struct LayerNames(pub [String; 32]);

impl Default for LayerNames {
    fn default() -> Self {
        let mut n: [String; 32] = std::array::from_fn(|_| String::new());
        for (i, s) in [
            "Default",
            "TransparentFX",
            "Ignore Raycast",
            "",
            "Water",
            "UI",
        ]
        .into_iter()
        .enumerate()
        {
            n[i] = s.into();
        }
        n[6] = "Vehicle".into();
        n[7] = "Player".into();
        n[8] = "Ground".into();
        Self(n)
    }
}

impl LayerNames {
    /// Unity `LayerMask.NameToLayer`.
    pub fn name_to_layer(&self, name: &str) -> Option<u8> {
        self.0
            .iter()
            .position(|n| !n.is_empty() && n.eq_ignore_ascii_case(name))
            .map(|i| i as u8)
    }
    /// Unity `LayerMask.GetMask`.
    pub fn get_mask(&self, names: &[&str]) -> u32 {
        names
            .iter()
            .filter_map(|n| self.name_to_layer(n))
            .fold(0, |m, l| m | (1 << l))
    }
}

struct Slot {
    generation: u32,
    object: Option<GameObject>,
}

/// Which per-frame message to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    FixedUpdate,
    Update,
    LateUpdate,
}

pub struct World {
    slots: Vec<Slot>,
    free: Vec<u32>,
    roots: Vec<GameObjectId>,
    next_component: u64,
    pending_destroy: Vec<GameObjectId>,
    pending_component_destroy: Vec<(GameObjectId, ComponentId)>,
    /// When false, `awake`/`update`/... are not delivered (edit mode in the editor).
    /// Built-in components are still added; their callbacks run once play starts.
    pub run_scripts: bool,
    pub registry: ComponentRegistry,
    pub resources: Resources,
    pub layers: LayerNames,
    /// Name of the active scene (Unity `SceneManager.GetActiveScene().name`).
    pub scene_name: String,
    pub scene_path: Option<String>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            roots: Vec::new(),
            next_component: 1,
            pending_destroy: Vec::new(),
            pending_component_destroy: Vec::new(),
            run_scripts: true,
            registry: ComponentRegistry::default(),
            resources: Resources::default(),
            layers: LayerNames::default(),
            scene_name: "Untitled".into(),
            scene_path: None,
        }
    }

    // ---- resources -------------------------------------------------------------------

    pub fn resource<T: 'static>(&self) -> &T {
        self.resources
            .get::<T>()
            .unwrap_or_else(|| panic!("resource {} is not installed", std::any::type_name::<T>()))
    }

    pub fn resource_mut<T: 'static>(&mut self) -> &mut T {
        self.resources
            .get_mut::<T>()
            .unwrap_or_else(|| panic!("resource {} is not installed", std::any::type_name::<T>()))
    }

    pub fn try_resource<T: 'static>(&self) -> Option<&T> {
        self.resources.get::<T>()
    }

    pub fn try_resource_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.resources.get_mut::<T>()
    }

    // ---- objects ---------------------------------------------------------------------

    pub fn get(&self, id: GameObjectId) -> Option<&GameObject> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.object.as_ref()
    }

    pub fn get_mut(&mut self, id: GameObjectId) -> Option<&mut GameObject> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.object.as_mut()
    }

    /// Unity's `obj != null`: the object exists and is not pending destruction.
    pub fn is_alive(&self, id: GameObjectId) -> bool {
        self.get(id).is_some_and(|o| !o.destroy_pending)
    }

    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.object.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every live object, in storage order.
    pub fn objects(&self) -> impl Iterator<Item = (GameObjectId, &GameObject)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            let o = s.object.as_ref()?;
            (!o.destroy_pending).then_some((
                GameObjectId {
                    index: i as u32,
                    generation: s.generation,
                },
                o,
            ))
        })
    }

    pub fn ids(&self) -> Vec<GameObjectId> {
        self.objects().map(|(id, _)| id).collect()
    }

    /// Root objects in Hierarchy order.
    pub fn roots(&self) -> &[GameObjectId] {
        &self.roots
    }

    /// All objects depth-first in Hierarchy order.
    pub fn hierarchy_order(&self) -> Vec<GameObjectId> {
        let mut out = Vec::new();
        let mut stack: Vec<GameObjectId> = self.roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(o) = self.get(id) else { continue };
            if o.destroy_pending {
                continue;
            }
            out.push(id);
            stack.extend(o.children.iter().rev());
        }
        out
    }

    /// Unity `new GameObject(name)`: an empty, active root object.
    pub fn create(&mut self, name: &str) -> GameObjectId {
        let object = GameObject {
            name: name.into(),
            tag: "Untagged".into(),
            layer: 0,
            static_flags: StaticEditorFlags::NOTHING,
            transform: Transform::default(),
            dont_destroy_on_load: false,
            active_self: true,
            active_in_hierarchy: true,
            parent: None,
            children: Vec::new(),
            components: Vec::new(),
            destroy_pending: false,
            prefab_source: None,
            prefab_link: None,
        };
        let id = match self.free.pop() {
            Some(i) => {
                let slot = &mut self.slots[i as usize];
                slot.object = Some(object);
                GameObjectId {
                    index: i,
                    generation: slot.generation,
                }
            }
            None => {
                self.slots.push(Slot {
                    generation: 1,
                    object: Some(object),
                });
                GameObjectId {
                    index: self.slots.len() as u32 - 1,
                    generation: 1,
                }
            }
        };
        self.roots.push(id);
        id
    }

    /// Like [`World::create`] with a local transform.
    pub fn create_at(
        &mut self,
        name: &str,
        position: Vector3,
        rotation: Quaternion,
    ) -> GameObjectId {
        let id = self.create(name);
        let t = &mut self.get_mut(id).expect("just created").transform;
        t.local_position = position;
        t.local_rotation = rotation;
        id
    }

    /// Unity `GameObject.Find`: first active object with this name (or `a/b/c` path).
    pub fn find(&self, name: &str) -> Option<GameObjectId> {
        if name.contains('/') {
            return self.find_path(name);
        }
        self.hierarchy_order().into_iter().find(|&id| {
            self.get(id)
                .is_some_and(|o| o.active_in_hierarchy && o.name == name)
        })
    }

    /// Finds by name including inactive objects.
    pub fn find_any(&self, name: &str) -> Option<GameObjectId> {
        self.hierarchy_order()
            .into_iter()
            .find(|&id| self.get(id).is_some_and(|o| o.name == name))
    }

    /// `"Root/Child/Grandchild"` from the scene root.
    pub fn find_path(&self, path: &str) -> Option<GameObjectId> {
        let mut parts = path.trim_start_matches('/').split('/');
        let first = parts.next()?;
        let mut cur = *self
            .roots
            .iter()
            .find(|&&r| self.get(r).is_some_and(|o| o.name == first))?;
        for p in parts {
            cur = self.find_child(cur, p)?;
        }
        Some(cur)
    }

    /// Unity `Transform.Find`: direct child by name, or a relative `a/b` path.
    pub fn find_child(&self, parent: GameObjectId, path: &str) -> Option<GameObjectId> {
        let mut cur = parent;
        for p in path.split('/') {
            cur = *self
                .get(cur)?
                .children
                .iter()
                .find(|&&c| self.get(c).is_some_and(|o| o.name == p))?;
        }
        Some(cur)
    }

    /// Depth-first search below `root` (inclusive) by name.
    pub fn find_in_children(&self, root: GameObjectId, name: &str) -> Option<GameObjectId> {
        self.descendants(root)
            .into_iter()
            .find(|&id| self.get(id).is_some_and(|o| o.name == name))
    }

    /// Unity `GameObject.FindGameObjectsWithTag`.
    pub fn find_with_tag(&self, tag: &str) -> Vec<GameObjectId> {
        self.objects()
            .filter(|(_, o)| o.active_in_hierarchy && o.tag == tag)
            .map(|(id, _)| id)
            .collect()
    }

    /// `root` and everything below it, depth-first.
    pub fn descendants(&self, root: GameObjectId) -> Vec<GameObjectId> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if let Some(o) = self.get(id) {
                out.push(id);
                stack.extend(o.children.iter().rev());
            }
        }
        out
    }

    /// Slash-separated path from the scene root (`"Car/Body/Wheel FL"`).
    pub fn path_of(&self, id: GameObjectId) -> String {
        let mut parts = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(o) = self.get(c) else { break };
            parts.push(o.name.as_str());
            cur = o.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// Path of `id` relative to `root` (empty for `root` itself), for animation bindings.
    pub fn relative_path(&self, root: GameObjectId, id: GameObjectId) -> Option<String> {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != root {
            let o = self.get(cur)?;
            parts.push(o.name.as_str());
            cur = o.parent?;
        }
        parts.reverse();
        Some(parts.join("/"))
    }

    // ---- activation -------------------------------------------------------------------

    /// Unity `GameObject.SetActive`.
    pub fn set_active(&mut self, id: GameObjectId, active: bool) {
        let Some(o) = self.get_mut(id) else { return };
        if o.active_self == active {
            return;
        }
        o.active_self = active;
        self.refresh_active(id);
    }

    fn parent_active(&self, id: GameObjectId) -> bool {
        self.get(id)
            .and_then(|o| o.parent)
            .and_then(|p| self.get(p))
            .is_none_or(|p| p.active_in_hierarchy)
    }

    fn refresh_active(&mut self, id: GameObjectId) {
        let parent_active = self.parent_active(id);
        let Some(o) = self.get_mut(id) else { return };
        let now = o.active_self && parent_active;
        if now == o.active_in_hierarchy {
            return;
        }
        o.active_in_hierarchy = now;
        let cids: Vec<ComponentId> = o.components.iter().map(|c| c.id).collect();
        let children = o.children.clone();
        if now {
            for cid in cids {
                self.activate_component(id, cid);
            }
            for c in children {
                self.refresh_active(c);
            }
        } else {
            for c in children {
                self.refresh_active(c);
            }
            for cid in cids {
                self.deactivate_component(id, cid);
            }
        }
    }

    fn entry(&self, id: GameObjectId, cid: ComponentId) -> Option<&ComponentEntry> {
        self.get(id)?.components.iter().find(|c| c.id == cid)
    }

    fn entry_mut(&mut self, id: GameObjectId, cid: ComponentId) -> Option<&mut ComponentEntry> {
        self.get_mut(id)?
            .components
            .iter_mut()
            .find(|c| c.id == cid)
    }

    /// Awake (once) + OnEnable for a component on an active object.
    fn activate_component(&mut self, id: GameObjectId, cid: ComponentId) {
        if !self.run_scripts {
            return;
        }
        let Some(e) = self.entry(id, cid) else { return };
        if e.destroy_pending {
            return;
        }
        if !e.awoken {
            self.entry_mut(id, cid).expect("entry").awoken = true;
            self.call(id, cid, |c, ctx| c.awake(ctx));
        }
        let Some(e) = self.entry(id, cid) else { return };
        let active = self.get(id).is_some_and(|o| o.active_in_hierarchy);
        if e.enabled && !e.live && active {
            self.entry_mut(id, cid).expect("entry").live = true;
            self.call(id, cid, |c, ctx| c.on_enable(ctx));
        }
    }

    fn deactivate_component(&mut self, id: GameObjectId, cid: ComponentId) {
        if self.entry(id, cid).is_some_and(|e| e.live) {
            self.entry_mut(id, cid).expect("entry").live = false;
            self.call(id, cid, |c, ctx| c.on_disable(ctx));
        }
    }

    /// Runs `f` on one component with a [`Ctx`] for its object. The component is taken out
    /// of the object while `f` runs, so `f` may freely use the world.
    pub fn call(
        &mut self,
        id: GameObjectId,
        cid: ComponentId,
        f: impl FnOnce(&mut dyn Component, &mut Ctx),
    ) -> bool {
        let Some(mut comp) = self.entry_mut(id, cid).and_then(|e| e.comp.take()) else {
            return false;
        };
        let result = {
            let mut ctx = Ctx {
                world: self,
                this: id,
                component: cid,
            };
            // A panicking script is reported like a Unity exception; the game keeps running.
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(comp.as_mut(), &mut ctx)))
        };
        if let Err(p) = result {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            let name = self.get(id).map_or("?", |o| o.name.as_str());
            oxide_core::debug_log_error!("{} on \"{name}\": {msg}", comp.type_info().name);
        }
        // The object or component may have been removed during the call.
        if let Some(e) = self.entry_mut(id, cid) {
            e.comp = Some(comp);
        }
        true
    }

    // ---- components -------------------------------------------------------------------

    /// Unity `AddComponent`. Awake/OnEnable run immediately if the object is active.
    pub fn add_component<T: Component>(
        &mut self,
        id: GameObjectId,
        comp: T,
    ) -> Option<ComponentId> {
        self.add_component_boxed(id, Box::new(comp))
    }

    pub fn add_component_boxed(
        &mut self,
        id: GameObjectId,
        comp: Box<dyn Component>,
    ) -> Option<ComponentId> {
        let cid = ComponentId(self.next_component);
        self.next_component += 1;
        let type_id = component_type_id(comp.as_ref());
        let type_name = comp.type_info().name;
        let o = self.get_mut(id)?;
        o.components.push(ComponentEntry {
            id: cid,
            type_id,
            type_name,
            enabled: true,
            awoken: false,
            started: false,
            live: false,
            destroy_pending: false,
            comp: Some(comp),
        });
        if o.active_in_hierarchy {
            self.activate_component(id, cid);
        }
        Some(cid)
    }

    /// Adds a registered component by type name (scene files, editor, MCP).
    pub fn add_component_by_name(
        &mut self,
        id: GameObjectId,
        name: &str,
    ) -> Result<ComponentId, String> {
        let comp = self
            .registry
            .create(name)
            .ok_or_else(|| format!("no component type named `{name}`"))?;
        self.add_component_boxed(id, comp)
            .ok_or_else(|| "no such GameObject".to_string())
    }

    pub fn get_component<T: Component>(&self, id: GameObjectId) -> Option<&T> {
        let tid = TypeId::of::<T>();
        self.get(id)?
            .components
            .iter()
            .filter(|c| c.type_id == tid && !c.destroy_pending)
            .find_map(|c| c.comp.as_deref().and_then(downcast_ref::<T>))
    }

    pub fn get_component_mut<T: Component>(&mut self, id: GameObjectId) -> Option<&mut T> {
        let tid = TypeId::of::<T>();
        self.get_mut(id)?
            .components
            .iter_mut()
            .filter(|c| c.type_id == tid && !c.destroy_pending)
            .find_map(|c| c.comp.as_deref_mut().and_then(downcast_mut::<T>))
    }

    pub fn has_component<T: Component>(&self, id: GameObjectId) -> bool {
        let tid = TypeId::of::<T>();
        self.get(id).is_some_and(|o| {
            o.components
                .iter()
                .any(|c| c.type_id == tid && !c.destroy_pending)
        })
    }

    /// Id of the first `T` on the object.
    pub fn component_id<T: Component>(&self, id: GameObjectId) -> Option<ComponentId> {
        let tid = TypeId::of::<T>();
        self.get(id)?
            .components
            .iter()
            .find(|c| c.type_id == tid && !c.destroy_pending)
            .map(|c| c.id)
    }

    /// Unity `GetComponentInChildren` (includes the object itself, active objects only).
    pub fn get_component_in_children<T: Component>(
        &self,
        id: GameObjectId,
    ) -> Option<(GameObjectId, &T)> {
        self.descendants(id).into_iter().find_map(|d| {
            let o = self.get(d)?;
            if !o.active_in_hierarchy {
                return None;
            }
            self.get_component::<T>(d).map(|c| (d, c))
        })
    }

    /// Unity `GetComponentInParent` (includes the object itself).
    pub fn get_component_in_parent<T: Component>(
        &self,
        id: GameObjectId,
    ) -> Option<(GameObjectId, &T)> {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if let Some(comp) = self.get_component::<T>(c) {
                return Some((c, comp));
            }
            cur = self.get(c)?.parent;
        }
        None
    }

    /// Component by id as a trait object (for reflection-driven tools).
    pub fn component_dyn(&self, id: GameObjectId, cid: ComponentId) -> Option<&dyn Component> {
        self.entry(id, cid)?.comp.as_deref()
    }

    pub fn component_dyn_mut(
        &mut self,
        id: GameObjectId,
        cid: ComponentId,
    ) -> Option<&mut dyn Component> {
        self.entry_mut(id, cid)?.comp.as_deref_mut()
    }

    /// Component ids on an object whose type name matches.
    pub fn components_named(&self, id: GameObjectId, type_name: &str) -> Vec<ComponentId> {
        self.get(id).map_or_else(Vec::new, |o| {
            o.components
                .iter()
                .filter(|c| !c.destroy_pending && c.type_name.eq_ignore_ascii_case(type_name))
                .map(|c| c.id)
                .collect()
        })
    }

    /// Every `(object, component)` of type `T` on active objects with the component enabled.
    pub fn query<T: Component>(&self) -> impl Iterator<Item = (GameObjectId, &T)> {
        let tid = TypeId::of::<T>();
        self.objects()
            .filter(|(_, o)| o.active_in_hierarchy)
            .flat_map(move |(id, o)| {
                o.components
                    .iter()
                    .filter(move |c| c.type_id == tid && c.enabled && !c.destroy_pending)
                    .filter_map(move |c| {
                        c.comp
                            .as_deref()
                            .and_then(downcast_ref::<T>)
                            .map(|t| (id, t))
                    })
            })
    }

    /// `(object, component id)` of every enabled `T` on an active object.
    pub fn query_ids<T: Component>(&self) -> Vec<(GameObjectId, ComponentId)> {
        let tid = TypeId::of::<T>();
        self.objects()
            .filter(|(_, o)| o.active_in_hierarchy)
            .flat_map(|(id, o)| {
                o.components
                    .iter()
                    .filter(move |c| c.type_id == tid && c.enabled && !c.destroy_pending)
                    .map(move |c| (id, c.id))
            })
            .collect()
    }

    /// A specific component instance, typed.
    pub fn component<T: Component>(&self, id: GameObjectId, cid: ComponentId) -> Option<&T> {
        self.entry(id, cid)?
            .comp
            .as_deref()
            .and_then(downcast_ref::<T>)
    }

    pub fn component_mut<T: Component>(
        &mut self,
        id: GameObjectId,
        cid: ComponentId,
    ) -> Option<&mut T> {
        self.entry_mut(id, cid)?
            .comp
            .as_deref_mut()
            .and_then(downcast_mut::<T>)
    }

    /// Nearest object at or above `id` carrying a `T` (Unity's rigidbody lookup for colliders).
    pub fn ancestor_with<T: Component>(&self, id: GameObjectId) -> Option<GameObjectId> {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if self.has_component::<T>(c) {
                return Some(c);
            }
            cur = self.parent(c);
        }
        None
    }

    /// Ids of objects carrying an enabled `T` (including inactive objects if `include_inactive`).
    pub fn find_objects_of_type<T: Component>(&self, include_inactive: bool) -> Vec<GameObjectId> {
        let tid = TypeId::of::<T>();
        self.objects()
            .filter(|(_, o)| include_inactive || o.active_in_hierarchy)
            .filter(|(_, o)| {
                o.components
                    .iter()
                    .any(|c| c.type_id == tid && !c.destroy_pending)
            })
            .map(|(id, _)| id)
            .collect()
    }

    /// Unity `Behaviour.enabled`.
    pub fn set_component_enabled(&mut self, id: GameObjectId, cid: ComponentId, enabled: bool) {
        let Some(e) = self.entry_mut(id, cid) else {
            return;
        };
        if e.enabled == enabled {
            return;
        }
        e.enabled = enabled;
        if enabled {
            if self.get(id).is_some_and(|o| o.active_in_hierarchy) {
                self.activate_component(id, cid);
            }
        } else {
            self.deactivate_component(id, cid);
        }
    }

    pub fn is_component_enabled(&self, id: GameObjectId, cid: ComponentId) -> bool {
        self.entry(id, cid).is_some_and(|e| e.enabled)
    }

    /// Unity `Destroy(component)`, deferred to the end of the current phase.
    pub fn destroy_component(&mut self, id: GameObjectId, cid: ComponentId) {
        if let Some(e) = self.entry_mut(id, cid) {
            e.destroy_pending = true;
            self.pending_component_destroy.push((id, cid));
        }
    }

    /// Removes a component right away (editor / MCP).
    pub fn remove_component_now(&mut self, id: GameObjectId, cid: ComponentId) {
        self.deactivate_component(id, cid);
        if self.entry(id, cid).is_some_and(|e| e.awoken) {
            self.call(id, cid, |c, ctx| c.on_destroy(ctx));
        }
        if let Some(o) = self.get_mut(id) {
            o.components.retain(|c| c.id != cid);
        }
    }

    /// Collects every active component's [`Component::on_draw_gizmos`] lines.
    pub fn collect_gizmos(&self) -> crate::Gizmos {
        let mut g = crate::Gizmos {
            color: oxide_math::Color::WHITE,
            ..Default::default()
        };
        for (id, o) in self.objects() {
            if !o.active_in_hierarchy {
                continue;
            }
            for e in o
                .components
                .iter()
                .filter(|e| e.enabled && !e.destroy_pending)
            {
                if let Some(c) = e.comp.as_deref() {
                    g.color = oxide_math::Color::WHITE;
                    c.on_draw_gizmos(self, id, &mut g);
                }
            }
        }
        g
    }

    /// Takes out every component whose type `is_script` (before a script reload), keeping
    /// what is needed to put it back: position, fields and lifecycle state. Scripts get
    /// `OnDisable`, not `OnDestroy`, like Unity's domain reload.
    pub fn take_components_for_reload(
        &mut self,
        is_script: impl Fn(&str) -> bool,
    ) -> Vec<ReloadedComponent> {
        let mut out = Vec::new();
        for id in self.hierarchy_order() {
            let list: Vec<(usize, ComponentId, &'static str)> = self
                .get(id)
                .map(|o| {
                    o.components
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| !c.destroy_pending)
                        .map(|(i, c)| (i, c.id, c.type_name))
                        .collect()
                })
                .unwrap_or_default();
            for (index, cid, ty) in list {
                let missing = self
                    .component::<crate::MissingScript>(id, cid)
                    .map(|m| (m.script.clone(), m.data.clone(), m.awoken, m.started));
                if missing.is_none() && !is_script(ty) {
                    continue;
                }
                self.deactivate_component(id, cid);
                let Some(e) = self.entry(id, cid) else {
                    continue;
                };
                let (enabled, mut awoken, mut started) = (e.enabled, e.awoken, e.started);
                let (type_name, fields) = match missing {
                    Some((script, data, a, s)) => {
                        (awoken, started) = (a, s);
                        (script, data)
                    }
                    None => (
                        ty.to_owned(),
                        e.comp
                            .as_deref()
                            .map(|c| {
                                c.fields()
                                    .into_iter()
                                    .map(|(k, v)| (k.to_owned(), v))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    ),
                };
                out.push(ReloadedComponent {
                    object: id,
                    index,
                    type_name,
                    enabled,
                    awoken,
                    started,
                    fields,
                });
                if let Some(o) = self.get_mut(id) {
                    o.components.retain(|c| c.id != cid);
                }
            }
        }
        out
    }

    /// Puts components taken by [`World::take_components_for_reload`] back, created from the
    /// (new) registry. Scripts that no longer exist become [`MissingScript`]s, so their data
    /// survives. Live scripts get `OnEnable` (not `Awake`/`Start` again). Returns the type
    /// names that are missing.
    pub fn restore_components_after_reload(
        &mut self,
        taken: Vec<ReloadedComponent>,
    ) -> Vec<String> {
        let mut missing = Vec::new();
        // Lower indices first so each insert lands where it was.
        let mut taken = taken;
        taken.sort_by_key(|t| (t.object.to_bits(), t.index));
        for t in taken {
            if !self.is_alive(t.object) {
                continue;
            }
            let (comp, ty): (Box<dyn Component>, &'static str) =
                match self.registry.get(&t.type_name) {
                    Some(reg) => {
                        let mut c = (reg.create)();
                        for (k, v) in &t.fields {
                            let _ = c.set_field(k, v.clone());
                        }
                        (c, reg.info.name)
                    }
                    None => {
                        missing.push(t.type_name.clone());
                        (
                            Box::new(crate::MissingScript {
                                script: t.type_name.clone(),
                                data: t.fields.clone(),
                                awoken: t.awoken,
                                started: t.started,
                            }),
                            "MissingScript",
                        )
                    }
                };
            let real = ty != "MissingScript";
            let cid = ComponentId(self.next_component);
            self.next_component += 1;
            let entry = ComponentEntry {
                id: cid,
                type_id: crate::component::component_type_id(comp.as_ref()),
                type_name: ty,
                enabled: t.enabled,
                awoken: t.awoken && real,
                started: t.started && real,
                live: false,
                destroy_pending: false,
                comp: Some(comp),
            };
            let Some(o) = self.get_mut(t.object) else {
                continue;
            };
            let at = t.index.min(o.components.len());
            o.components.insert(at, entry);
            if real && t.awoken {
                self.activate_component(t.object, cid);
            }
        }
        missing.sort();
        missing.dedup();
        missing
    }

    /// Moves a component up/down in the Inspector order.
    pub fn move_component(&mut self, id: GameObjectId, cid: ComponentId, delta: i32) {
        let Some(o) = self.get_mut(id) else { return };
        let Some(i) = o.components.iter().position(|c| c.id == cid) else {
            return;
        };
        let j = (i as i32 + delta).clamp(0, o.components.len() as i32 - 1) as usize;
        let e = o.components.remove(i);
        o.components.insert(j, e);
    }

    // ---- hierarchy ---------------------------------------------------------------------

    /// Unity `Transform.SetParent(parent, worldPositionStays)`.
    pub fn set_parent(
        &mut self,
        id: GameObjectId,
        parent: Option<GameObjectId>,
        world_position_stays: bool,
    ) -> Result<(), String> {
        if !self.is_alive(id) {
            return Err("no such GameObject".into());
        }
        if let Some(p) = parent {
            if !self.is_alive(p) {
                return Err("parent does not exist".into());
            }
            // No cycles: the new parent may not be inside `id`'s subtree.
            let mut cur = Some(p);
            while let Some(c) = cur {
                if c == id {
                    return Err("cannot parent an object to itself or its descendant".into());
                }
                cur = self.get(c).and_then(|o| o.parent);
            }
        }
        let world = world_position_stays
            .then(|| (self.position(id), self.rotation(id), self.lossy_scale(id)));
        let old = self.get(id).and_then(|o| o.parent);
        match old {
            Some(op) => {
                if let Some(po) = self.get_mut(op) {
                    po.children.retain(|&c| c != id);
                }
            }
            None => self.roots.retain(|&c| c != id),
        }
        match parent {
            Some(p) => self.get_mut(p).expect("checked").children.push(id),
            None => self.roots.push(id),
        }
        self.get_mut(id).expect("checked").parent = parent;
        if let Some((pos, rot, scale)) = world {
            self.set_rotation(id, rot);
            self.set_position(id, pos);
            let parent_scale = parent.map_or(Vector3::ONE, |p| self.lossy_scale(p));
            self.get_mut(id).expect("checked").transform.local_scale =
                scale / parent_scale.max(Vector3::splat(1e-6));
        }
        self.refresh_active(id);
        Ok(())
    }

    pub fn parent(&self, id: GameObjectId) -> Option<GameObjectId> {
        self.get(id)?.parent
    }

    pub fn children(&self, id: GameObjectId) -> &[GameObjectId] {
        self.get(id).map_or(&[], |o| &o.children)
    }

    /// Topmost ancestor (Unity `Transform.root`).
    pub fn root_of(&self, id: GameObjectId) -> GameObjectId {
        let mut cur = id;
        while let Some(p) = self.parent(cur) {
            cur = p;
        }
        cur
    }

    pub fn sibling_index(&self, id: GameObjectId) -> usize {
        let list = match self.parent(id) {
            Some(p) => self.children(p),
            None => &self.roots,
        };
        list.iter().position(|&c| c == id).unwrap_or(0)
    }

    pub fn set_sibling_index(&mut self, id: GameObjectId, index: usize) {
        let list = match self.parent(id) {
            Some(p) => &mut self.get_mut(p).expect("parent").children,
            None => &mut self.roots,
        };
        if let Some(i) = list.iter().position(|&c| c == id) {
            list.remove(i);
            list.insert(index.min(list.len()), id);
        }
    }

    // ---- transforms --------------------------------------------------------------------

    pub fn local_to_world(&self, id: GameObjectId) -> Matrix4x4 {
        let Some(o) = self.get(id) else {
            return Matrix4x4::IDENTITY;
        };
        let local = o.transform.local_matrix();
        match o.parent {
            Some(p) => self.local_to_world(p) * local,
            None => local,
        }
    }

    pub fn world_to_local(&self, id: GameObjectId) -> Matrix4x4 {
        self.local_to_world(id).inverse()
    }

    /// World matrices for every live object, computed top-down in one pass.
    pub fn all_world_matrices(&self) -> Vec<Option<Matrix4x4>> {
        let mut out = vec![None; self.slots.len()];
        let mut stack: Vec<(GameObjectId, Matrix4x4)> = self
            .roots
            .iter()
            .map(|&r| (r, Matrix4x4::IDENTITY))
            .collect();
        while let Some((id, parent)) = stack.pop() {
            let Some(o) = self.get(id) else { continue };
            let m = parent * o.transform.local_matrix();
            out[id.index as usize] = Some(m);
            stack.extend(o.children.iter().map(|&c| (c, m)));
        }
        out
    }

    pub fn transform(&self, id: GameObjectId) -> Option<&Transform> {
        self.get(id).map(|o| &o.transform)
    }

    pub fn transform_mut(&mut self, id: GameObjectId) -> Option<&mut Transform> {
        self.get_mut(id).map(|o| &mut o.transform)
    }

    /// World-space position (Unity `transform.position`).
    pub fn position(&self, id: GameObjectId) -> Vector3 {
        match self.get(id) {
            Some(o) => match o.parent {
                Some(p) => self
                    .local_to_world(p)
                    .transform_point3(o.transform.local_position),
                None => o.transform.local_position,
            },
            None => Vector3::ZERO,
        }
    }

    pub fn set_position(&mut self, id: GameObjectId, p: Vector3) {
        let local = match self.parent(id) {
            Some(parent) => self.world_to_local(parent).transform_point3(p),
            None => p,
        };
        if let Some(t) = self.transform_mut(id) {
            t.local_position = local;
        }
    }

    /// World-space rotation (Unity `transform.rotation`).
    pub fn rotation(&self, id: GameObjectId) -> Quaternion {
        let mut q = Quaternion::IDENTITY;
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(o) = self.get(c) else { break };
            q = o.transform.local_rotation * q;
            cur = o.parent;
        }
        q.normalize()
    }

    pub fn set_rotation(&mut self, id: GameObjectId, q: Quaternion) {
        let parent_rot = self
            .parent(id)
            .map_or(Quaternion::IDENTITY, |p| self.rotation(p));
        if let Some(t) = self.transform_mut(id) {
            t.local_rotation = (parent_rot.inverse() * q).normalize();
        }
    }

    /// World Euler angles in degrees (Unity `transform.eulerAngles`).
    pub fn euler_angles(&self, id: GameObjectId) -> Vector3 {
        self.rotation(id).euler_angles()
    }

    pub fn set_euler_angles(&mut self, id: GameObjectId, e: Vector3) {
        self.set_rotation(id, Quaternion::euler_v(e));
    }

    /// Approximate world scale (Unity `transform.lossyScale`).
    pub fn lossy_scale(&self, id: GameObjectId) -> Vector3 {
        let mut s = Vector3::ONE;
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(o) = self.get(c) else { break };
            s *= o.transform.local_scale;
            cur = o.parent;
        }
        s
    }

    pub fn forward(&self, id: GameObjectId) -> Vector3 {
        self.rotation(id) * Vector3::Z
    }
    pub fn right(&self, id: GameObjectId) -> Vector3 {
        self.rotation(id) * Vector3::X
    }
    pub fn up(&self, id: GameObjectId) -> Vector3 {
        self.rotation(id) * Vector3::Y
    }

    /// Unity `Transform.TransformPoint`.
    pub fn transform_point(&self, id: GameObjectId, p: Vector3) -> Vector3 {
        self.local_to_world(id).transform_point3(p)
    }
    /// Unity `Transform.TransformDirection` (rotation only).
    pub fn transform_direction(&self, id: GameObjectId, d: Vector3) -> Vector3 {
        self.rotation(id) * d
    }
    /// Unity `Transform.InverseTransformPoint`.
    pub fn inverse_transform_point(&self, id: GameObjectId, p: Vector3) -> Vector3 {
        self.world_to_local(id).transform_point3(p)
    }
    /// Unity `Transform.InverseTransformDirection`.
    pub fn inverse_transform_direction(&self, id: GameObjectId, d: Vector3) -> Vector3 {
        self.rotation(id).inverse() * d
    }

    /// Unity `Transform.Translate`.
    pub fn translate(&mut self, id: GameObjectId, delta: Vector3, space: Space) {
        let d = match space {
            Space::Self_ => self.rotation(id) * delta,
            Space::World => delta,
        };
        let p = self.position(id);
        self.set_position(id, p + d);
    }

    /// Unity `Transform.Rotate(eulers, space)`.
    pub fn rotate(&mut self, id: GameObjectId, euler: Vector3, space: Space) {
        let r = Quaternion::euler_v(euler);
        match space {
            Space::Self_ => {
                if let Some(t) = self.transform_mut(id) {
                    t.local_rotation = (t.local_rotation * r).normalize();
                }
            }
            Space::World => {
                let q = self.rotation(id);
                self.set_rotation(id, r * q);
            }
        }
    }

    /// Unity `Transform.RotateAround`.
    pub fn rotate_around(&mut self, id: GameObjectId, point: Vector3, axis: Vector3, angle: f32) {
        let q = Quaternion::angle_axis(angle, axis);
        let p = self.position(id);
        self.set_position(id, point + q * (p - point));
        let r = self.rotation(id);
        self.set_rotation(id, q * r);
    }

    /// Unity `Transform.LookAt`.
    pub fn look_at(&mut self, id: GameObjectId, target: Vector3, up: Vector3) {
        let dir = target - self.position(id);
        if dir.length_squared() > 1e-12 {
            self.set_rotation(id, Quaternion::look_rotation(dir, up));
        }
    }

    // ---- destruction ---------------------------------------------------------------------

    /// Unity `Destroy(gameObject)`: removed at the end of the current phase.
    pub fn destroy(&mut self, id: GameObjectId) {
        if let Some(o) = self.get_mut(id)
            && !o.destroy_pending
        {
            o.destroy_pending = true;
            self.pending_destroy.push(id);
        }
    }

    /// Unity `DestroyImmediate`.
    pub fn destroy_immediate(&mut self, id: GameObjectId) {
        if !self.get(id).is_some() {
            return;
        }
        let subtree = self.descendants(id);
        // OnDisable then OnDestroy on everything, children included.
        for &d in subtree.iter().rev() {
            let cids: Vec<ComponentId> = self
                .get(d)
                .map(|o| o.components.iter().map(|c| c.id).collect())
                .unwrap_or_default();
            for cid in cids.iter().copied() {
                self.deactivate_component(d, cid);
            }
            for cid in cids {
                if self.entry(d, cid).is_some_and(|e| e.awoken) {
                    self.call(d, cid, |c, ctx| c.on_destroy(ctx));
                }
            }
        }
        match self.parent(id) {
            Some(p) => {
                if let Some(po) = self.get_mut(p) {
                    po.children.retain(|&c| c != id);
                }
            }
            None => self.roots.retain(|&c| c != id),
        }
        for d in subtree {
            let slot = &mut self.slots[d.index as usize];
            if slot.generation == d.generation {
                slot.object = None;
                slot.generation = slot.generation.wrapping_add(1).max(1);
                self.free.push(d.index);
            }
        }
    }

    /// Applies deferred `destroy` calls. The engine calls this after each phase.
    pub fn flush_destroyed(&mut self) {
        for (id, cid) in std::mem::take(&mut self.pending_component_destroy) {
            self.remove_component_now(id, cid);
        }
        for id in std::mem::take(&mut self.pending_destroy) {
            self.destroy_immediate(id);
        }
    }

    /// Removes every object (except `dont_destroy_on_load` roots when `keep_persistent`).
    pub fn clear(&mut self, keep_persistent: bool) {
        let roots = self.roots.clone();
        for r in roots {
            if keep_persistent && self.get(r).is_some_and(|o| o.dont_destroy_on_load) {
                continue;
            }
            self.destroy_immediate(r);
        }
    }

    // ---- player loop -----------------------------------------------------------------

    /// Calls `start` on every enabled component that has not started yet.
    pub fn run_start(&mut self) {
        if !self.run_scripts {
            return;
        }
        let mut todo = Vec::new();
        for (id, o) in self.objects() {
            if !o.active_in_hierarchy {
                continue;
            }
            for c in &o.components {
                if c.enabled && c.awoken && !c.started && !c.destroy_pending {
                    todo.push((id, c.id));
                }
            }
        }
        for (id, cid) in todo {
            if let Some(e) = self.entry_mut(id, cid) {
                if e.started {
                    continue;
                }
                e.started = true;
            }
            self.call(id, cid, |c, ctx| c.start(ctx));
        }
        self.flush_destroyed();
    }

    /// Runs one message on every started, enabled component of every active object.
    pub fn run_phase(&mut self, phase: Phase) {
        if !self.run_scripts {
            return;
        }
        let mut todo = Vec::new();
        for (id, o) in self.objects() {
            if !o.active_in_hierarchy {
                continue;
            }
            for c in &o.components {
                if c.enabled && c.started && !c.destroy_pending {
                    todo.push((id, c.id));
                }
            }
        }
        for (id, cid) in todo {
            // Re-check: an earlier script may have disabled or deactivated this one.
            let ok = self.get(id).is_some_and(|o| o.active_in_hierarchy)
                && self
                    .entry(id, cid)
                    .is_some_and(|e| e.enabled && !e.destroy_pending);
            if !ok {
                continue;
            }
            self.call(id, cid, |c, ctx| match phase {
                Phase::FixedUpdate => c.fixed_update(ctx),
                Phase::Update => c.update(ctx),
                Phase::LateUpdate => c.late_update(ctx),
            });
        }
        self.flush_destroyed();
    }

    /// Delivers a callback to every enabled component on `id` (collision/trigger messages,
    /// `SendMessage`).
    pub fn broadcast(&mut self, id: GameObjectId, mut f: impl FnMut(&mut dyn Component, &mut Ctx)) {
        if !self.run_scripts {
            return;
        }
        let cids: Vec<ComponentId> = match self.get(id) {
            Some(o) if o.active_in_hierarchy => o
                .components
                .iter()
                .filter(|c| c.enabled && c.awoken && !c.destroy_pending)
                .map(|c| c.id)
                .collect(),
            _ => return,
        };
        for cid in cids {
            self.call(id, cid, &mut f);
        }
    }

    /// Unity `SendMessage(name, arg)`; true if any component handled it.
    pub fn send_message(
        &mut self,
        id: GameObjectId,
        message: &str,
        arg: oxide_core::reflect::Value,
    ) -> bool {
        let mut handled = false;
        self.broadcast(id, |c, ctx| handled |= c.on_message(ctx, message, &arg));
        handled
    }

    /// Switches script execution on (entering Play mode): awakens and enables everything
    /// that is active.
    pub fn start_scripts(&mut self) {
        self.run_scripts = true;
        for id in self.hierarchy_order() {
            if !self.get(id).is_some_and(|o| o.active_in_hierarchy) {
                continue;
            }
            let cids: Vec<ComponentId> = self
                .get(id)
                .map(|o| o.components.iter().map(|c| c.id).collect())
                .unwrap_or_default();
            for cid in cids {
                self.activate_component(id, cid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_math::QuaternionExt;

    fn close(a: Vector3, b: Vector3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn hierarchy_transforms() {
        let mut w = World::new();
        let parent = w.create("Parent");
        let child = w.create("Child");
        w.transform_mut(parent).unwrap().local_position = Vector3::new(10.0, 0.0, 0.0);
        w.transform_mut(parent).unwrap().local_rotation = Quaternion::euler(0.0, 90.0, 0.0);
        w.set_parent(child, Some(parent), false).unwrap();
        w.transform_mut(child).unwrap().local_position = Vector3::new(0.0, 0.0, 1.0);
        // Child is 1 unit "forward" of a parent that faces +X.
        assert!(close(w.position(child), Vector3::new(11.0, 0.0, 0.0)));
        assert!(close(w.forward(child), Vector3::X));
        w.set_position(child, Vector3::new(10.0, 5.0, 0.0));
        assert!(close(
            w.transform(child).unwrap().local_position,
            Vector3::new(0.0, 5.0, 0.0)
        ));
        // Re-parent keeping the world position.
        w.set_parent(child, None, true).unwrap();
        assert!(close(w.position(child), Vector3::new(10.0, 5.0, 0.0)));
        assert_eq!(w.roots().len(), 2);
        assert!(w.set_parent(parent, Some(parent), true).is_err());
    }

    #[test]
    fn find_and_paths() {
        let mut w = World::new();
        let car = w.create("Car");
        let body = w.create("Body");
        let wheel = w.create("Wheel");
        w.set_parent(body, Some(car), false).unwrap();
        w.set_parent(wheel, Some(body), false).unwrap();
        assert_eq!(w.find_path("Car/Body/Wheel"), Some(wheel));
        assert_eq!(w.find("Wheel"), Some(wheel));
        assert_eq!(w.path_of(wheel), "Car/Body/Wheel");
        assert_eq!(w.relative_path(car, wheel).as_deref(), Some("Body/Wheel"));
        w.set_active(body, false);
        assert_eq!(w.find("Wheel"), None);
        assert!(!w.get(wheel).unwrap().active_in_hierarchy());
        w.destroy(car);
        assert!(!w.is_alive(car));
        w.flush_destroyed();
        assert!(w.get(wheel).is_none());
        assert_eq!(w.len(), 0);
    }
}
