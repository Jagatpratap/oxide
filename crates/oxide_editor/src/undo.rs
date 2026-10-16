//! Undo/Redo by scene snapshots, and the Play mode snapshot.
//!
//! Each undo step stores the whole scene as a [`SceneFile`] taken just before the edit.
//! Restoring rebuilds the objects, so GameObject ids change; the selection is carried across
//! by position in hierarchy order. Reference: <https://docs.unity3d.com/Manual/UndoWindow.html>

use oxide_assets::{AssetServer, Handle, TerrainData};
use oxide_scene::{GameObjectId, SceneFile, SceneHandle, SceneInfo, World};

const MAX_STEPS: usize = 64;

pub struct Snapshot {
    pub label: String,
    scene: SceneFile,
    scene_name: String,
    scene_path: Option<String>,
    /// Loaded scenes and each object's scene (multi-scene editing).
    scenes: Vec<SceneInfo>,
    scene_assignment: Vec<SceneHandle>,
    /// Selected objects as indices into `World::hierarchy_order`.
    selection: Vec<usize>,
    /// Terrain data edited by this step (terrain tools): restored with the scene. The
    /// big arrays are shared (`Arc`), so a copy is cheap.
    terrains: Vec<(Handle<TerrainData>, TerrainData)>,
}

impl Snapshot {
    pub fn take(world: &World, label: &str, selection: &[GameObjectId]) -> Self {
        let order = world.hierarchy_order();
        Self {
            label: label.to_owned(),
            scene: world.serialize(&[], &world.scene_name),
            scene_name: world.scene_name.clone(),
            scene_path: world.scene_path.clone(),
            scenes: world.loaded_scenes(),
            scene_assignment: world.scene_assignment(),
            selection: selection
                .iter()
                .filter_map(|id| order.iter().position(|o| o == id))
                .collect(),
            terrains: vec![],
        }
    }

    /// Adds the current state of these terrains to the step.
    fn with_terrains(mut self, world: &World, handles: &[Handle<TerrainData>]) -> Self {
        let Some(assets) = world.try_resource::<AssetServer>() else {
            return self;
        };
        self.terrains = handles
            .iter()
            .filter_map(|h| Some((h.clone(), assets.terrains.get(h)?.clone())))
            .collect();
        self
    }

    /// Terrains this step restores (marked unsaved by the caller).
    pub fn terrain_paths(&self) -> Vec<String> {
        self.terrains
            .iter()
            .map(|(h, _)| h.path().to_owned())
            .collect()
    }

    /// Rewrites asset paths (after assets moved). Returns how many references changed.
    pub fn remap_assets(&mut self, f: &mut dyn FnMut(&str) -> Option<String>) -> usize {
        self.scene.remap_asset_paths(f)
    }

    /// Replaces the scene with this snapshot. Returns the restored selection.
    pub fn restore(&self, world: &mut World) -> Vec<GameObjectId> {
        if let Some(assets) = world.try_resource_mut::<AssetServer>() {
            for (h, data) in &self.terrains {
                if let Some(cur) = assets.terrains.get_mut(h) {
                    // Newer versions than anything cached, so meshes and GPU data refresh.
                    let v = cur.versions;
                    let mut d = data.clone();
                    d.version = cur.version.max(d.version) + 1;
                    d.versions.heights = v.heights.max(d.versions.heights) + 1;
                    d.versions.alphamaps = v.alphamaps.max(d.versions.alphamaps) + 1;
                    d.versions.layers = v.layers.max(d.versions.layers) + 1;
                    d.versions.holes = v.holes.max(d.versions.holes) + 1;
                    d.versions.trees = v.trees.max(d.versions.trees) + 1;
                    d.versions.details = v.details.max(d.versions.details) + 1;
                    d.dirty_heights.clear();
                    d.hold_collider = false;
                    *cur = d;
                }
            }
        }
        world.clear(false);
        let (_, warnings) = world.spawn(&self.scene, None);
        for w in warnings {
            oxide_core::debug_log_warning!("{w}");
        }
        world.scene_name = self.scene_name.clone();
        world.scene_path = self.scene_path.clone();
        world.restore_scenes(&self.scenes, &self.scene_assignment);
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

    /// Like [`record`](Self::record) for a terrain tool: the step also restores the
    /// terrains' data.
    pub fn record_terrain(
        &mut self,
        world: &World,
        label: &str,
        selection: &[GameObjectId],
        terrains: &[Handle<TerrainData>],
    ) -> bool {
        self.last = None;
        self.undo
            .push(Snapshot::take(world, label, selection).with_terrains(world, terrains));
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
        let handles: Vec<_> = s.terrains.iter().map(|(h, _)| h.clone()).collect();
        self.redo
            .push(Snapshot::take(world, &s.label, selection).with_terrains(world, &handles));
        self.last = None;
        Some((s.label.clone(), s.restore(world)))
    }

    pub fn redo(
        &mut self,
        world: &mut World,
        selection: &[GameObjectId],
    ) -> Option<(String, Vec<GameObjectId>)> {
        let s = self.redo.pop()?;
        let handles: Vec<_> = s.terrains.iter().map(|(h, _)| h.clone()).collect();
        self.undo
            .push(Snapshot::take(world, &s.label, selection).with_terrains(world, &handles));
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

    /// Rewrites asset paths in every step, so Undo after a rename doesn't bring back
    /// references to the old path.
    pub fn remap_assets(&mut self, f: &mut dyn FnMut(&str) -> Option<String>) {
        for s in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            s.remap_assets(f);
        }
    }

    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Terrains the next undo would change (to mark them unsaved).
    pub fn top_terrains(&self) -> Vec<String> {
        self.undo
            .last()
            .map(Snapshot::terrain_paths)
            .unwrap_or_default()
    }

    /// Terrains the next redo would change.
    pub fn top_redo_terrains(&self) -> Vec<String> {
        self.redo
            .last()
            .map(Snapshot::terrain_paths)
            .unwrap_or_default()
    }

    /// Labels, oldest first (the Undo History window).
    pub fn labels(&self) -> (Vec<&str>, Vec<&str>) {
        (
            self.undo.iter().map(|s| s.label.as_str()).collect(),
            self.redo.iter().rev().map(|s| s.label.as_str()).collect(),
        )
    }
}

/// Undo History window: the initial state, every undo step, then the redo steps (greyed).
/// Clicking an entry undoes or redoes to just after it.
pub fn window_ui(st: &mut crate::EditorState, ui: &mut egui::Ui) {
    let (done, undone) = st.undo.labels();
    let depth = done.len();
    let mut go_to = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.style_mut().spacing.item_spacing.y = 0.0;
            let rows = std::iter::once("Scene opened")
                .chain(done.iter().copied())
                .chain(undone.iter().copied());
            for (i, label) in rows.enumerate() {
                let current = i == depth;
                let text = if i > depth {
                    egui::RichText::new(label).color(egui::Color32::from_gray(120))
                } else {
                    egui::RichText::new(label)
                };
                if ui
                    .with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                        ui.selectable_label(current, text)
                    })
                    .inner
                    .clicked()
                    && !current
                {
                    go_to = Some(i);
                }
            }
        });
    if let Some(i) = go_to {
        st.actions.push(crate::Action::UndoTo(i));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_math::Vector3;

    #[test]
    fn terrain_strokes_undo_and_redo() {
        let mut w = World::new();
        let mut assets = AssetServer::new(".");
        let h = assets
            .terrains
            .add(TerrainData::flat(Vector3::new(100.0, 50.0, 100.0), 33));
        w.resources.insert(assets);
        let mut u = UndoStack::default();
        u.record_terrain(&w, "Raise", &[], std::slice::from_ref(&h));
        let brush = oxide_assets::Brush::default();
        let d = w
            .resource_mut::<AssetServer>()
            .terrains
            .get_mut(&h)
            .unwrap();
        d.paint_height(0.5, 0.5, &brush, oxide_assets::HeightOp::Raise(10.0));
        let raised = d.interpolated_height(0.5, 0.5);
        assert!(raised > 1.0);
        assert_eq!(u.top_terrains(), vec![h.path().to_owned()]);
        u.undo(&mut w, &[]).unwrap();
        let d = w.resource::<AssetServer>().terrains.get(&h).unwrap();
        assert_eq!(
            d.interpolated_height(0.5, 0.5),
            0.0,
            "undo restores the heights"
        );
        u.redo(&mut w, &[]).unwrap();
        let d = w.resource::<AssetServer>().terrains.get(&h).unwrap();
        assert!(
            (d.interpolated_height(0.5, 0.5) - raised).abs() < 1e-4,
            "redo brings the stroke back"
        );
    }

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
