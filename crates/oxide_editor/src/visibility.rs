//! Scene visibility and pickability: the Hierarchy's eye and hand toggles. Hidden objects
//! aren't drawn in the Scene view (the Game view still shows them); unpickable objects can't
//! be clicked in the Scene view. Children follow their ancestors. Editor-only state, like
//! Unity's `SceneVisibilityManager`.
//! Reference: <https://docs.unity3d.com/Manual/SceneVisibility.html>

use oxide_scene::{GameObjectId, World};
use std::collections::HashSet;

#[derive(Default)]
pub struct SceneVisibility {
    /// Objects hidden directly (their descendants are hidden too).
    pub hidden: HashSet<GameObjectId>,
    /// Objects with picking disabled directly (descendants too).
    pub unpickable: HashSet<GameObjectId>,
    /// The Scene view's visibility toggle: off shows every object regardless.
    pub show_all: bool,
}

fn in_chain(world: &World, set: &HashSet<GameObjectId>, mut id: GameObjectId) -> bool {
    if set.is_empty() {
        return false;
    }
    loop {
        if set.contains(&id) {
            return true;
        }
        match world.parent(id) {
            Some(p) => id = p,
            None => return false,
        }
    }
}

impl SceneVisibility {
    pub fn is_hidden(&self, world: &World, id: GameObjectId) -> bool {
        !self.show_all && in_chain(world, &self.hidden, id)
    }

    pub fn is_unpickable(&self, world: &World, id: GameObjectId) -> bool {
        in_chain(world, &self.unpickable, id)
    }

    /// Hidden objects can't be picked either.
    pub fn can_pick(&self, world: &World, id: GameObjectId) -> bool {
        !self.is_hidden(world, id) && !self.is_unpickable(world, id)
    }

    /// Hidden or not, as the eye toggle shows it (ignores `show_all`).
    pub fn hidden_in_chain(&self, world: &World, id: GameObjectId) -> bool {
        in_chain(world, &self.hidden, id)
    }

    /// Eye click: hides the object (and so its children), or shows it again. Showing a
    /// child of a hidden parent un-hides the parent's other descendants too, as in Unity's
    /// object-and-children toggle.
    pub fn toggle_hidden(&mut self, world: &World, id: GameObjectId) {
        toggle(&mut self.hidden, world, id);
    }

    pub fn toggle_pickable(&mut self, world: &World, id: GameObjectId) {
        toggle(&mut self.unpickable, world, id);
    }

    /// Bits of every object hidden in the Scene view (for filtering draws).
    pub fn hidden_bits(&self, world: &World) -> HashSet<u64> {
        if self.show_all || self.hidden.is_empty() {
            return HashSet::new();
        }
        world
            .objects()
            .map(|(id, _)| id)
            .filter(|&id| in_chain(world, &self.hidden, id))
            .map(GameObjectId::to_bits)
            .collect()
    }

    /// Drops objects that no longer exist.
    pub fn prune(&mut self, world: &World) {
        self.hidden.retain(|&id| world.is_alive(id));
        self.unpickable.retain(|&id| world.is_alive(id));
    }
}

fn toggle(set: &mut HashSet<GameObjectId>, world: &World, id: GameObjectId) {
    if in_chain(world, set, id) {
        // Clear the object, its ancestors and its descendants.
        let mut a = Some(id);
        while let Some(x) = a {
            set.remove(&x);
            a = world.parent(x);
        }
        for d in world.descendants(id) {
            set.remove(&d);
        }
    } else {
        set.insert(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_follow_parents() {
        let mut w = World::new();
        let parent = w.create("Parent");
        let child = w.create("Child");
        w.set_parent(child, Some(parent), false).unwrap();
        let other = w.create("Other");
        let mut v = SceneVisibility::default();
        v.toggle_hidden(&w, parent);
        assert!(v.is_hidden(&w, child));
        assert!(!v.is_hidden(&w, other));
        assert_eq!(v.hidden_bits(&w).len(), 2);
        // Showing the child clears the hidden parent.
        v.toggle_hidden(&w, child);
        assert!(!v.is_hidden(&w, parent) && !v.is_hidden(&w, child));
        // The Scene view toggle shows everything.
        v.toggle_hidden(&w, parent);
        v.show_all = true;
        assert!(!v.is_hidden(&w, child) && v.hidden_bits(&w).is_empty());
        v.toggle_pickable(&w, parent);
        assert!(v.is_unpickable(&w, child) && !v.can_pick(&w, child));
    }
}
