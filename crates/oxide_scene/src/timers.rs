//! Delayed actions: `Destroy(obj, t)`, `Invoke(name, t)` and `InvokeRepeating`.

use crate::id::GameObjectId;
use crate::world::World;

enum Action {
    Destroy,
    Message(String),
    /// Sent every `interval` seconds until cancelled.
    Repeat(String, f32),
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

    /// Unity `InvokeRepeating`: `message` at `time`, then every `interval` seconds.
    pub fn repeat_at(&mut self, id: GameObjectId, message: &str, time: f32, interval: f32) {
        self.pending.push((
            time,
            id,
            Action::Repeat(message.to_owned(), interval.max(1e-4)),
        ));
    }

    /// Unity `CancelInvoke(name)` (`None`: every pending invoke of `id`).
    pub fn cancel(&mut self, id: GameObjectId, message: Option<&str>) {
        self.pending.retain(|(_, o, a)| {
            *o != id
                || match a {
                    Action::Destroy => true,
                    Action::Message(m) | Action::Repeat(m, _) => {
                        message.is_some_and(|name| name != m)
                    }
                }
        });
    }

    /// Unity `IsInvoking(name)` (`None`: any).
    pub fn is_invoking(&self, id: GameObjectId, message: Option<&str>) -> bool {
        self.pending.iter().any(|(_, o, a)| {
            *o == id
                && match a {
                    Action::Destroy => false,
                    Action::Message(m) | Action::Repeat(m, _) => message.is_none_or(|n| n == m),
                }
        })
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
                Action::Repeat(m, interval) => {
                    world.send_message(id, &m, oxide_core::reflect::Value::Null);
                    // Rescheduled unless the object is gone.
                    if world.is_alive(id)
                        && let Some(t) = world.try_resource_mut::<Timers>()
                    {
                        t.pending
                            .push((now + interval, id, Action::Repeat(m, interval)));
                    }
                }
            }
        }
        world.flush_destroyed();
    }
}
