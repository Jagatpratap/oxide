//! Unity `StateMachineBehaviour`: script logic attached to Animator states and sub-state
//! machines (`OnStateEnter`, `OnStateUpdate`, `OnStateExit`, `OnStateMove`, `OnStateIK`,
//! `OnStateMachineEnter`, `OnStateMachineExit`).
//!
//! Behaviour types are registered by name (scripts list them in `scripts! { ...;
//! behaviours: Type }`); a state stores the type name and its field values, and each
//! Animator gets its own instances (Unity's default, not shared between Animators).
//! Reference: <https://docs.unity3d.com/ScriptReference/StateMachineBehaviour.html>

use crate::AnimatorStateInfo;
use oxide_assets::StateBehaviour;
use oxide_core::reflect::Reflect;
use oxide_scene::{ComponentId, Ctx, GameObjectId};
use std::collections::{BTreeMap, HashMap};

/// Logic that runs while an Animator is in a state. `ctx` is the Animator's GameObject
/// (`ctx.get_component::<Animator>()` reaches the Animator itself).
#[allow(unused_variables)]
pub trait StateMachineBehaviour: Reflect {
    /// The first frame of a transition into the state (or when it is entered directly).
    fn on_state_enter(&mut self, ctx: &mut Ctx, state: &AnimatorStateInfo, layer: usize) {}
    /// Every frame the state is playing, except its first and last.
    fn on_state_update(&mut self, ctx: &mut Ctx, state: &AnimatorStateInfo, layer: usize) {}
    /// The last frame of a transition out of the state.
    fn on_state_exit(&mut self, ctx: &mut Ctx, state: &AnimatorStateInfo, layer: usize) {}
    /// After root motion each frame (Unity `OnStateMove`).
    fn on_state_move(&mut self, ctx: &mut Ctx, state: &AnimatorStateInfo, layer: usize) {}
    /// After the layer's IK pass (layers with IK Pass on).
    fn on_state_ik(&mut self, ctx: &mut Ctx, state: &AnimatorStateInfo, layer: usize) {}
    /// Entering a sub-state machine (behaviours on that machine).
    fn on_state_machine_enter(&mut self, ctx: &mut Ctx, machine: &str) {}
    fn on_state_machine_exit(&mut self, ctx: &mut Ctx, machine: &str) {}
}

type Factory = fn() -> Box<dyn StateMachineBehaviour>;

fn make<T: StateMachineBehaviour + Default>() -> Box<dyn StateMachineBehaviour> {
    Box::new(T::default())
}

/// Where an instance lives: the Animator, its layer, and the state (layer-flat index) or
/// sub-machine (`machine:` + path) it is attached to, and its index there.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct InstanceKey {
    pub animator: (GameObjectId, ComponentId),
    pub layer: usize,
    pub owner: String,
    pub index: usize,
}

/// World resource: the registered behaviour types and the live instances.
#[derive(Default)]
pub struct StateMachineBehaviours {
    types: BTreeMap<String, Factory>,
    pub(crate) instances: HashMap<InstanceKey, Box<dyn StateMachineBehaviour>>,
}

impl StateMachineBehaviours {
    pub fn register<T: StateMachineBehaviour + Default>(&mut self) {
        let name = T::default().type_info().name.to_owned();
        self.types.insert(name, make::<T>);
    }

    /// Forgets these types and their instances (a script library about to be reloaded).
    pub fn remove(&mut self, type_names: &[String]) {
        self.types.retain(|n, _| !type_names.contains(n));
        self.instances
            .retain(|_, b| !type_names.iter().any(|n| n == b.type_info().name));
    }

    pub fn names(&self) -> Vec<String> {
        self.types.keys().cloned().collect()
    }

    /// A new instance with default values (the Inspector shows its fields).
    pub fn create(&self, type_name: &str) -> Option<Box<dyn StateMachineBehaviour>> {
        self.types.get(type_name).map(|f| f())
    }

    /// A new instance with the saved values of `b`.
    pub fn instantiate(&self, b: &StateBehaviour) -> Option<Box<dyn StateMachineBehaviour>> {
        let mut inst = self.create(&b.type_name)?;
        for (k, v) in &b.fields {
            let _ = inst.set_field(k, v.clone());
        }
        Some(inst)
    }

    /// Drops instances of Animators that are gone or were reset (Stop leaves Play mode).
    pub fn clear(&mut self) {
        self.instances.clear();
    }
}
