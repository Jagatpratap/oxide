//! Delayed actions: `Destroy(obj, t)` and `Invoke(name, t)`.

use crate::id::GameObjectId;
use crate::world::World;

enum Action {
    Destroy,
    Message(String),
}

#[derive(Default)]
pub struct Timers {
    pending: Vec<(f32, GameObjectId, Action)>,
}

impl Timers {
    pub fn destroy_at(&mut self, id: GameObjectId, time: f32) {
        self.pending.push((time, id, Action::Destroy));
    }

    pub fn message_at(&mut self, id: GameObjectId, message: &str, time: f32) {
        self.pending
            .push((time, id, Action::Message(message.to_owned())));
    }

    /// Fires everything due at `now`. Called by the engine once per frame after `Update`.
    pub fn run(world: &mut World, now: f32) {
        let Some(t) = world.try_resource_mut::<Timers>() else {
            return;
        };
        let (due, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut t.pending)
            .into_iter()
            .partition(|p| p.0 <= now);
        t.pending = rest;
        for (_, id, action) in due {
            match action {
                Action::Destroy => world.destroy(id),
                Action::Message(m) => {
                    world.send_message(id, &m, oxide_core::reflect::Value::Null);
                }
            }
        }
        world.flush_destroyed();
    }
}
