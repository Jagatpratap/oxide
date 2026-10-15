//! Components (Unity `MonoBehaviour` and built-in components) and their registry.
//!
//! A component is any `#[derive(Reflect, Clone, Default)]` type implementing [`Component`].
//! Every message has a default empty body, so implement only what you need — exactly like
//! Unity's `Awake`/`Start`/`Update` methods. Reference:
//! <https://docs.unity3d.com/Manual/execution-order.html>

use crate::ctx::Ctx;
use crate::id::GameObjectId;
use oxide_core::reflect::{Reflect, TypeInfo};
use oxide_math::Vector3;
use std::any::{Any, TypeId};
use std::collections::BTreeMap;

/// One contact point of a collision.
#[derive(Clone, Copy, Debug)]
pub struct ContactPoint {
    pub point: Vector3,
    pub normal: Vector3,
    pub separation: f32,
}

/// Unity `Collision`, passed to `on_collision_*`.
#[derive(Clone, Debug)]
pub struct Collision {
    /// The other object involved.
    pub game_object: GameObjectId,
    pub contacts: Vec<ContactPoint>,
    /// Velocity of the other body relative to this one.
    pub relative_velocity: Vector3,
    pub impulse: Vector3,
}

#[allow(unused_variables)]
pub trait Component: Reflect + ComponentClone {
    /// Called once when the component first becomes active (after being added, or when its
    /// GameObject is first activated).
    fn awake(&mut self, ctx: &mut Ctx) {}
    fn on_enable(&mut self, ctx: &mut Ctx) {}
    /// Called before the first `update` after the component is enabled.
    fn start(&mut self, ctx: &mut Ctx) {}
    /// Fixed timestep (physics) update.
    fn fixed_update(&mut self, ctx: &mut Ctx) {}
    fn update(&mut self, ctx: &mut Ctx) {}
    fn late_update(&mut self, ctx: &mut Ctx) {}
    fn on_disable(&mut self, ctx: &mut Ctx) {}
    fn on_destroy(&mut self, ctx: &mut Ctx) {}

    fn on_collision_enter(&mut self, ctx: &mut Ctx, collision: &Collision) {}
    fn on_collision_stay(&mut self, ctx: &mut Ctx, collision: &Collision) {}
    fn on_collision_exit(&mut self, ctx: &mut Ctx, collision: &Collision) {}
    fn on_trigger_enter(&mut self, ctx: &mut Ctx, other: GameObjectId) {}
    fn on_trigger_stay(&mut self, ctx: &mut Ctx, other: GameObjectId) {}
    fn on_trigger_exit(&mut self, ctx: &mut Ctx, other: GameObjectId) {}

    /// Named message (Unity `SendMessage`). Return true if handled.
    fn on_message(
        &mut self,
        ctx: &mut Ctx,
        message: &str,
        arg: &oxide_core::reflect::Value,
    ) -> bool {
        false
    }

    /// Whether more than one of this component may live on one GameObject
    /// (Unity `[DisallowMultipleComponent]` when false).
    fn allow_multiple(&self) -> bool {
        true
    }
}

/// Lets `Box<dyn Component>` be cloned (needed by `Instantiate`). Implemented for every
/// `Component + Clone`.
pub trait ComponentClone {
    fn clone_box(&self) -> Box<dyn Component>;
}

impl<T: Component + Clone> ComponentClone for T {
    fn clone_box(&self) -> Box<dyn Component> {
        Box::new(self.clone())
    }
}

/// `TypeId` of the concrete type behind a component trait object.
pub fn component_type_id(c: &dyn Component) -> TypeId {
    (c as &dyn Any).type_id()
}

pub fn downcast_ref<T: Component>(c: &dyn Component) -> Option<&T> {
    (c as &dyn Any).downcast_ref()
}

pub fn downcast_mut<T: Component>(c: &mut dyn Component) -> Option<&mut T> {
    (c as &mut dyn Any).downcast_mut()
}

pub struct ComponentRegistration {
    pub info: TypeInfo,
    pub type_id: TypeId,
    /// Menu path for the Inspector's "Add Component" (e.g. `Physics/Rigidbody`).
    pub menu: String,
    pub create: fn() -> Box<dyn Component>,
}

/// Every component type that can be added by name (Add Component menu, scene files, MCP).
#[derive(Default)]
pub struct ComponentRegistry {
    types: BTreeMap<&'static str, ComponentRegistration>,
}

impl ComponentRegistry {
    pub fn register<T: Component + Default>(&mut self, menu: &str) {
        let probe = T::default();
        let info = probe.type_info();
        let create: fn() -> Box<dyn Component> = || Box::new(T::default());
        self.types.insert(
            info.name,
            ComponentRegistration {
                menu: if menu.is_empty() {
                    format!("Scripts/{}", info.name)
                } else {
                    menu.to_owned()
                },
                info,
                type_id: TypeId::of::<T>(),
                create,
            },
        );
    }

    pub fn create(&self, name: &str) -> Option<Box<dyn Component>> {
        self.get(name).map(|r| (r.create)())
    }

    /// Looks a type up by exact name, then case-insensitively.
    pub fn get(&self, name: &str) -> Option<&ComponentRegistration> {
        self.types.get(name).or_else(|| {
            self.types
                .values()
                .find(|r| r.info.name.eq_ignore_ascii_case(name))
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &ComponentRegistration> {
        self.types.values()
    }
}
