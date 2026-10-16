//! [`WorldStreamer`]: keeps the scene cells near a target loaded and unloads far ones, so a
//! big map is never in memory all at once. Built on additive `LoadSceneAsync` /
//! `UnloadSceneAsync`, the way Unity open-world projects stream.
//!
//! Reference: <https://docs.unity3d.com/Manual/MultiSceneEditing.html>

use oxide_core::Reflect;
use oxide_math::{Color, Vector3};
use oxide_scene::{AsyncOperation, Component, Ctx, GameObjectId, Gizmos, SceneHandle, World};

/// One streamed part of the world: a scene file and the area it covers.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(default)]
pub struct StreamingCell {
    /// Scene file with the cell's objects (relative to the project).
    pub scene: String,
    /// World-space center and size of the area the cell covers.
    pub center: Vector3,
    pub size: Vector3,
}

impl Default for StreamingCell {
    fn default() -> Self {
        Self {
            scene: String::new(),
            center: Vector3::ZERO,
            size: Vector3::new(100.0, 100.0, 100.0),
        }
    }
}

impl StreamingCell {
    /// Horizontal distance from `p` to the cell's area (0 inside it).
    pub fn distance(&self, p: Vector3) -> f32 {
        let h = self.size * 0.5;
        let dx = ((p.x - self.center.x).abs() - h.x).max(0.0);
        let dz = ((p.z - self.center.z).abs() - h.z).max(0.0);
        (dx * dx + dz * dz).sqrt()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum CellState {
    #[default]
    Unloaded,
    Loading(AsyncOperation),
    Loaded(SceneHandle),
    Unloading(AsyncOperation),
    /// The scene couldn't load; not retried until the streamer is re-enabled.
    Failed,
}

/// Streams [`StreamingCell`]s around a target: cells closer than `load_distance` load in the
/// background, cells farther than `unload_distance` unload. Runs in Play mode.
#[derive(Clone, Debug, Reflect)]
pub struct WorldStreamer {
    pub cells: Vec<StreamingCell>,
    /// What the world streams around (empty = the camera tagged MainCamera).
    pub target: Option<GameObjectId>,
    /// Cells whose area comes this close (horizontally) load.
    pub load_distance: f32,
    /// Loaded cells farther than this unload (larger than `load_distance` so a target on
    /// the boundary doesn't load and unload every frame).
    pub unload_distance: f32,
    /// Background loads running at once at most.
    pub max_concurrent_loads: u32,
    #[reflect(skip)]
    state: Vec<CellState>,
}

impl Default for WorldStreamer {
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            target: None,
            load_distance: 300.0,
            unload_distance: 400.0,
            max_concurrent_loads: 2,
            state: Vec::new(),
        }
    }
}

impl WorldStreamer {
    fn target_position(&self, world: &World) -> Option<Vector3> {
        let id = self
            .target
            .filter(|&t| world.is_alive(t))
            .or_else(|| world.find_with_tag("MainCamera").first().copied())?;
        Some(world.position(id))
    }

    /// Each cell's state: `unloaded`, `loading`, `loaded`, `unloading` or `failed`.
    pub fn cell_states(&self) -> Vec<&'static str> {
        (0..self.cells.len())
            .map(|i| match self.state.get(i).copied().unwrap_or_default() {
                CellState::Unloaded => "unloaded",
                CellState::Loading(_) => "loading",
                CellState::Loaded(_) => "loaded",
                CellState::Unloading(_) => "unloading",
                CellState::Failed => "failed",
            })
            .collect()
    }
}

impl Component for WorldStreamer {
    fn update(&mut self, ctx: &mut Ctx) {
        self.state.resize(self.cells.len(), CellState::Unloaded);
        let Some(p) = self.target_position(ctx.world) else {
            return;
        };
        // Advance running operations.
        for (i, s) in self.state.iter_mut().enumerate() {
            match *s {
                CellState::Loading(op) => {
                    if let Some(st) = ctx.scene_operation(op).filter(|st| st.is_done) {
                        *s = match (st.scene, st.error) {
                            (Some(h), None) => CellState::Loaded(h),
                            (_, e) => {
                                oxide_core::debug_log_error!(
                                    "WorldStreamer: cell {} ({}) failed to load: {}",
                                    i,
                                    self.cells[i].scene,
                                    e.unwrap_or_default()
                                );
                                CellState::Failed
                            }
                        };
                    }
                }
                CellState::Unloading(op) => {
                    if ctx.scene_operation(op).is_none_or(|st| st.is_done) {
                        *s = CellState::Unloaded;
                    }
                }
                CellState::Loaded(h) if ctx.world.scene_info(h).is_none() => {
                    // Unloaded by something else.
                    *s = CellState::Unloaded;
                }
                _ => {}
            }
        }
        // Unload far cells.
        for (cell, s) in self.cells.iter().zip(self.state.iter_mut()) {
            if let CellState::Loaded(h) = *s
                && cell.distance(p) > self.unload_distance
            {
                *s = CellState::Unloading(ctx.unload_scene_async(h));
            }
        }
        // Load the nearest unloaded cells in range.
        let mut wanted: Vec<(f32, usize)> = self
            .cells
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                self.state[*i] == CellState::Unloaded
                    && !c.scene.is_empty()
                    && c.distance(p) <= self.load_distance
            })
            .map(|(i, c)| (c.distance(p), i))
            .collect();
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut in_flight = self
            .state
            .iter()
            .filter(|s| matches!(s, CellState::Loading(_)))
            .count() as u32;
        for (_, i) in wanted {
            // Already open (loaded by hand, or before a script reload): adopt it.
            if let Some(h) = ctx.world.find_scene(&self.cells[i].scene)
                && h != SceneHandle::MAIN
            {
                self.state[i] = CellState::Loaded(h);
                continue;
            }
            if in_flight >= self.max_concurrent_loads.max(1) {
                break;
            }
            self.state[i] = CellState::Loading(ctx.load_scene_async(&self.cells[i].scene, true));
            in_flight += 1;
        }
    }

    /// Unloads every streamed cell (they come back when the streamer is enabled again).
    fn on_disable(&mut self, ctx: &mut Ctx) {
        for s in &mut self.state {
            if let CellState::Loaded(h) = *s {
                ctx.unload_scene_async(h);
            }
            *s = CellState::Unloaded;
        }
    }

    fn on_draw_gizmos(&self, world: &World, _: GameObjectId, gizmos: &mut Gizmos) {
        for (i, c) in self.cells.iter().enumerate() {
            gizmos.color = match self.state.get(i).copied().unwrap_or_default() {
                CellState::Loaded(_) => Color::rgb(0.3, 1.0, 0.4),
                CellState::Loading(_) | CellState::Unloading(_) => Color::rgb(1.0, 0.8, 0.2),
                CellState::Failed => Color::rgb(1.0, 0.25, 0.25),
                CellState::Unloaded => Color::rgb(0.55, 0.55, 0.6),
            };
            let h = c.size * 0.5;
            let y = c.center.y - h.y;
            let corner = |sx: f32, sz: f32| {
                Vector3::new(
                    c.center.x + sx * (h.x - 0.5),
                    y,
                    c.center.z + sz * (h.z - 0.5),
                )
            };
            let (a, b, d, e) = (
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            );
            gizmos.draw_line(a, b);
            gizmos.draw_line(b, d);
            gizmos.draw_line(d, e);
            gizmos.draw_line(e, a);
        }
        if let Some(p) = self.target_position(world) {
            gizmos.color = Color::rgb(0.4, 0.7, 1.0);
            let mut prev = p + Vector3::X * self.load_distance;
            for k in 1..=64 {
                let a = k as f32 / 64.0 * std::f32::consts::TAU;
                let q = p + Vector3::new(a.cos(), 0.0, a.sin()) * self.load_distance;
                gizmos.draw_line(prev, q);
                prev = q;
            }
        }
    }

    fn allow_multiple(&self) -> bool {
        false
    }
}
