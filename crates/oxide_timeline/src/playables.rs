//! Unity `PlayableBehaviour`: script logic for custom clips on Playable tracks.
//!
//! Scripts list their types in `scripts! { ...; playables: Type }`; a clip stores the type
//! name and its field values, and every clip of every Director gets its own instance.
//! Reference: <https://docs.unity3d.com/ScriptReference/Playables.PlayableBehaviour.html>

use crate::asset::BehaviourField;
use oxide_core::reflect::{Reflect, Value};
use oxide_scene::{GameObjectId, World};
use std::collections::{BTreeMap, HashMap};

/// What a behaviour is told each frame (Unity `Playable` + `FrameData`).
#[derive(Clone, Copy, Debug)]
pub struct FrameInfo {
    /// The Director playing it.
    pub director: GameObjectId,
    /// The track's bound object, if any.
    pub binding: Option<GameObjectId>,
    /// Seconds into the clip (after extrapolation and speed).
    pub time: f32,
    pub duration: f32,
    /// 0..1 from eases and blends.
    pub weight: f32,
    /// The Director is playing (false while scrubbed in the editor or paused).
    pub playing: bool,
    /// Seconds since the last frame (0 when scrubbing).
    pub delta_time: f32,
}

#[allow(unused_variables)]
pub trait PlayableBehaviour: Reflect {
    /// The playhead entered the clip (Unity `OnBehaviourPlay`).
    fn on_behaviour_play(&mut self, world: &mut World, info: &FrameInfo) {}
    /// Every frame the clip is under the playhead (Unity `ProcessFrame`).
    fn process_frame(&mut self, world: &mut World, info: &FrameInfo) {}
    /// The playhead left the clip or the Director stopped (Unity `OnBehaviourPause`).
    fn on_behaviour_pause(&mut self, world: &mut World, info: &FrameInfo) {}
}

type Factory = fn() -> Box<dyn PlayableBehaviour>;

fn make<T: PlayableBehaviour + Default>() -> Box<dyn PlayableBehaviour> {
    Box::new(T::default())
}

/// Instance key: the Director object, track index and clip index.
pub(crate) type InstanceKey = (GameObjectId, usize, usize);

/// World resource: registered behaviour types and the live instances.
#[derive(Default)]
pub struct PlayableBehaviours {
    types: BTreeMap<String, Factory>,
    pub(crate) instances: HashMap<InstanceKey, Box<dyn PlayableBehaviour>>,
}

impl PlayableBehaviours {
    pub fn register<T: PlayableBehaviour + Default>(&mut self) {
        let name = T::default().type_info().name.to_owned();
        self.types.insert(name, make::<T>);
    }

    /// Forgets these types and their instances (a script library about to reload).
    pub fn remove(&mut self, type_names: &[String]) {
        self.types.retain(|n, _| !type_names.contains(n));
        self.instances
            .retain(|_, b| !type_names.iter().any(|n| n == b.type_info().name));
    }

    pub fn names(&self) -> Vec<String> {
        self.types.keys().cloned().collect()
    }

    pub fn create(&self, type_name: &str) -> Option<Box<dyn PlayableBehaviour>> {
        self.types.get(type_name).map(|f| f())
    }

    /// A new instance with a clip's saved field values.
    pub fn instantiate(
        &self,
        type_name: &str,
        fields: &[BehaviourField],
    ) -> Option<Box<dyn PlayableBehaviour>> {
        let mut b = self.create(type_name)?;
        for f in fields {
            if let Ok(v) = serde_json::from_str::<Value>(&f.json) {
                let _ = b.set_field(&f.name, v);
            }
        }
        Some(b)
    }

    /// Drops every instance (Play mode stopped).
    pub fn clear(&mut self) {
        self.instances.clear();
    }
}

/// A behaviour's fields as saved clip fields.
pub fn save_fields(b: &dyn Reflect) -> Vec<BehaviourField> {
    b.fields()
        .into_iter()
        .filter_map(|(k, v)| {
            Some(BehaviourField {
                name: k.to_owned(),
                json: serde_json::to_string(&v).ok()?,
            })
        })
        .collect()
}
