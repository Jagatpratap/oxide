//! Undo/Redo by scene snapshots, and the Play mode snapshot.
//!
//! Each undo step stores the whole scene as a [`SceneFile`] taken just before the edit.
//! Restoring rebuilds the objects, so GameObject ids change; the selection is carried across
//! by position in hierarchy order. Reference: <https://docs.unity3d.com/Manual/UndoWindow.html>

use oxide_scene::{GameObjectId, SceneFile, World};

const MAX_STEPS: usize = 64;

pub struct Snapshot {
    pub label: String,
    scene: SceneFile,
    scene_name: String,
    scene_path: Option<String>,
    /// Selected objects as indices into `World::hierarchy_order`.
    selection: Vec<usize>,
}

impl Snapshot {
    pub fn take(world: &World, label: &str, selection: &[GameObjectId]) -> Self {
        let order = world.hierarchy_order();
        Self {
            label: label.to_owned(),
            scene: world.serialize(&[], &world.scene_name),
            scene_name: world.scene_name.clone(),
            scene_path: world.scene_path.clone(),
            selection: selection
                .iter()
                .filter_map(|id| order.iter().position(|o| o == id))
                .collect(),
        }
    }

    /// Replaces the scene with this snapshot. Returns the restored selection.
    pub fn restore(&self, world: &mut World) -> Vec<GameObjectId> {
        world.clear(false);
        let (_, warnings) = world.spawn(&self.scene, None);
        for w in warnings {
            oxide_core::debug_log_warning!("{w}");
        }
        world.scene_name = self.scene_name.clone();
        world.scene_path = self.scene_path.clone();
        let order = world.hierarchy_order();
        self.selection
            .iter()
            .filter_map(|&i| order.get(i).copied())
            .collect()
    }
}

#[derive(Default)]
pub struct UndoStack {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Coalescing key and time of the last record.
    last: Option<(u64, f64)>,
}

impl UndoStack {
    /// Pushes the current scene as an undo step. Returns false if it was merged into the
    /// previous step (same `coalesce` key within half a second).
    pub fn record(
        &mut self,
        world: &World,
        label: &str,
        selection: &[GameObjectId],
        coalesce: Option<u64>,
        now: f64,
    ) -> bool {
        if let (Some(k), Some((last, t))) = (coalesce, self.last)
            && k == last
            && now - t < 0.5
        {
            self.last = Some((k, now));
            return false;
        }
        self.last = coalesce.map(|k| (k, now));
        self.undo.push(Snapshot::take(world, label, selection));
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
        true
    }

    pub fn undo(
        &mut self,
        world: &mut World,
        selection: &[GameObjectId],
    ) -> Option<(String, Vec<GameObjectId>)> {
        let s = self.undo.pop()?;
        self.redo.push(Snapshot::take(world, &s.label, selection));
        self.last = None;
        Some((s.label.clone(), s.restore(world)))
    }

    pub fn redo(
        &mut self,
        world: &mut World,
        selection: &[GameObjectId],
    ) -> Option<(String, Vec<GameObjectId>)> {
        let s = self.redo.pop()?;
        self.undo.push(Snapshot::take(world, &s.label, selection));
        self.last = None;
        Some((s.label.clone(), s.restore(world)))
    }

    pub fn depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Drops steps above `depth` (and all redo steps).
    pub fn truncate(&mut self, depth: usize) {
        self.undo.truncate(depth);
        self.redo.clear();
        self.last = None;
    }

    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Labels, oldest first (the Undo History window).
    pub fn labels(&self) -> (Vec<&str>, Vec<&str>) {
        (
            self.undo.iter().map(|s| s.label.as_str()).collect(),
            self.redo.iter().rev().map(|s| s.label.as_str()).collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_math::Vector3;

    #[test]
    fn undo_redo_round_trip_keeps_selection() {
        let mut w = World::new();
        let a = w.create("A");
        let b = w.create("B");
        w.set_parent(b, Some(a), false).unwrap();
        let mut u = UndoStack::default();

        u.record(&w, "Move", &[b], None, 0.0);
        w.transform_mut(b).unwrap().local_position = Vector3::new(1.0, 2.0, 3.0);

        let (label, sel) = u.undo(&mut w, &[b]).unwrap();
        assert_eq!(label, "Move");
        assert_eq!(sel.len(), 1);
        assert_eq!(w.get(sel[0]).unwrap().name, "B");
        assert_eq!(w.transform(sel[0]).unwrap().local_position, Vector3::ZERO);
        assert_eq!(
            w.parent(sel[0]).map(|p| w.get(p).unwrap().name.clone()),
            Some("A".into())
        );

        let (_, sel) = u.redo(&mut w, &sel).unwrap();
        assert_eq!(
            w.transform(sel[0]).unwrap().local_position,
            Vector3::new(1.0, 2.0, 3.0)
        );
        assert_eq!(u.depth(), 1);
        assert_eq!(u.redo_depth(), 0);
    }

    #[test]
    fn drags_coalesce_into_one_step() {
        let w = World::new();
        let mut u = UndoStack::default();
        assert!(u.record(&w, "Move", &[], Some(7), 0.0));
        assert!(!u.record(&w, "Move", &[], Some(7), 0.2));
        assert!(!u.record(&w, "Move", &[], Some(7), 0.4));
        assert!(u.record(&w, "Move", &[], Some(7), 2.0));
        assert!(u.record(&w, "Rotate", &[], Some(8), 2.1));
        assert_eq!(u.depth(), 3);
    }
}
