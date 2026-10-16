//! Unity `SceneManager`: loading scene files (now or in the background), additive scenes and
//! unloading. A background load reads and parses the file on another thread; the engine
//! activates it (loads its assets and creates its objects) at the end of a frame, in the
//! order the loads were requested, through [`World::pump_scene_operations`].
//!
//! Reference: <https://docs.unity3d.com/Manual/MultiSceneEditing.html>

use crate::{RenderSettings, SceneFile, SceneHandle, World};
use oxide_core::reflect::Reflect;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, TryRecvError};

/// Unity `AsyncOperation` for a scene load or unload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AsyncOperation(pub u64);

/// Where a scene operation is.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneOpStatus {
    /// 0..0.9 while loading, 0.9 while waiting for activation, 1 when done (Unity's values).
    pub progress: f32,
    pub is_done: bool,
    /// The loaded scene, once a load is done.
    pub scene: Option<SceneHandle>,
    pub error: Option<String>,
}

enum Kind {
    Load { path: String, additive: bool },
    Unload(SceneHandle),
}

struct PendingOp {
    id: u64,
    kind: Kind,
    rx: Option<Receiver<Result<SceneFile, String>>>,
    parsed: Option<Result<SceneFile, String>>,
    allow_activation: bool,
}

/// Background scene operations (a World resource, created on first use).
#[derive(Default)]
pub struct SceneOperations {
    next: u64,
    pending: VecDeque<PendingOp>,
    finished: HashMap<u64, SceneOpStatus>,
    finished_order: VecDeque<u64>,
}

/// How many finished operations keep their status for queries.
const KEEP_FINISHED: usize = 256;

impl SceneOperations {
    fn finish(&mut self, id: u64, status: SceneOpStatus) {
        self.finished.insert(id, status);
        self.finished_order.push_back(id);
        while self.finished_order.len() > KEEP_FINISHED {
            if let Some(old) = self.finished_order.pop_front() {
                self.finished.remove(&old);
            }
        }
    }

    /// Operations still running.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

/// What [`World::pump_scene_operations`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PumpResult {
    pub activated: Vec<SceneHandle>,
    pub unloaded: Vec<SceneHandle>,
    /// A non-additive load replaced the scene (engine systems should restart).
    pub replaced: bool,
}

impl World {
    fn scene_ops(&mut self) -> &mut SceneOperations {
        if self.try_resource::<SceneOperations>().is_none() {
            self.resources.insert(SceneOperations::default());
        }
        self.resource_mut::<SceneOperations>()
    }

    /// Unity `SceneManager.LoadSceneAsync(path, mode)`: reads and parses `path` (relative to
    /// the project) on a background thread; it becomes active at the end of a later frame.
    pub fn load_scene_async(&mut self, path: &str, additive: bool) -> AsyncOperation {
        let full = self
            .try_resource::<oxide_assets::AssetServer>()
            .map(|a| a.full_path(path))
            .unwrap_or_else(|| path.into());
        let (tx, rx) = std::sync::mpsc::channel();
        let shown = path.to_owned();
        std::thread::Builder::new()
            .name(format!("load {shown}"))
            .spawn(move || {
                let r = std::fs::read_to_string(&full)
                    .map_err(|e| format!("cannot read {}: {e}", full.display()))
                    .and_then(|t| SceneFile::from_yaml(&t));
                let _ = tx.send(r);
            })
            .expect("spawn scene loader thread");
        let ops = self.scene_ops();
        ops.next += 1;
        let id = ops.next;
        ops.pending.push_back(PendingOp {
            id,
            kind: Kind::Load {
                path: path.to_owned(),
                additive,
            },
            rx: Some(rx),
            parsed: None,
            allow_activation: true,
        });
        AsyncOperation(id)
    }

    /// Unity `SceneManager.UnloadSceneAsync`: unloads an additive scene at the end of the frame.
    pub fn unload_scene_async(&mut self, scene: SceneHandle) -> AsyncOperation {
        let ops = self.scene_ops();
        ops.next += 1;
        let id = ops.next;
        ops.pending.push_back(PendingOp {
            id,
            kind: Kind::Unload(scene),
            rx: None,
            parsed: None,
            allow_activation: true,
        });
        AsyncOperation(id)
    }

    /// Unity `AsyncOperation.allowSceneActivation`: while false, a finished load waits at
    /// progress 0.9 (and so do loads requested after it).
    pub fn set_allow_scene_activation(&mut self, op: AsyncOperation, allow: bool) {
        if let Some(p) = self.scene_ops().pending.iter_mut().find(|p| p.id == op.0) {
            p.allow_activation = allow;
        }
    }

    /// Status of an operation (`None` for unknown or long-finished ones).
    pub fn scene_operation(&self, op: AsyncOperation) -> Option<SceneOpStatus> {
        let ops = self.try_resource::<SceneOperations>()?;
        if let Some(s) = ops.finished.get(&op.0) {
            return Some(s.clone());
        }
        let p = ops.pending.iter().find(|p| p.id == op.0)?;
        Some(SceneOpStatus {
            progress: if p.parsed.is_some() { 0.9 } else { 0.0 },
            is_done: false,
            scene: None,
            error: None,
        })
    }

    /// Advances background scene operations: collects parsed files and activates up to
    /// `max_activations` loads, in request order. Unloads run as soon as they reach the
    /// front. The engine calls this once per frame, outside script callbacks.
    pub fn pump_scene_operations(&mut self, max_activations: usize) -> PumpResult {
        let mut result = PumpResult::default();
        let Some(mut ops) = self.resources.remove::<SceneOperations>() else {
            return result;
        };
        for p in ops.pending.iter_mut() {
            if let Some(rx) = &p.rx {
                match rx.try_recv() {
                    Ok(r) => {
                        p.parsed = Some(r);
                        p.rx = None;
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        p.parsed = Some(Err("the loader thread stopped".into()));
                        p.rx = None;
                    }
                }
            }
        }
        let mut activations = 0;
        while let Some(front) = ops.pending.front() {
            let done = match &front.kind {
                Kind::Unload(h) => {
                    let h = *h;
                    let r = self.unload_scene(h);
                    if r.is_ok() {
                        result.unloaded.push(h);
                    }
                    Some(SceneOpStatus {
                        progress: 1.0,
                        is_done: true,
                        scene: Some(h),
                        error: r.err(),
                    })
                }
                Kind::Load { path, additive } => {
                    if front.parsed.is_none()
                        || !front.allow_activation
                        || activations >= max_activations
                    {
                        break;
                    }
                    let (path, additive) = (path.clone(), *additive);
                    let p = ops.pending.front_mut().expect("front");
                    let parsed = p.parsed.take().expect("checked");
                    activations += 1;
                    Some(match parsed {
                        Ok(file) => {
                            let (h, warnings) = self.activate_scene_file(&file, &path, additive);
                            for w in &warnings {
                                oxide_core::debug_log_warning!("{w}");
                            }
                            result.activated.push(h);
                            result.replaced |= !additive;
                            SceneOpStatus {
                                progress: 1.0,
                                is_done: true,
                                scene: Some(h),
                                error: None,
                            }
                        }
                        Err(e) => {
                            oxide_core::debug_log_error!("LoadSceneAsync {path}: {e}");
                            SceneOpStatus {
                                progress: 1.0,
                                is_done: true,
                                scene: None,
                                error: Some(e),
                            }
                        }
                    })
                }
            };
            if let Some(status) = done {
                let id = ops.pending.pop_front().expect("front").id;
                ops.finish(id, status);
            }
        }
        self.resources.insert(ops);
        result
    }

    /// Makes a parsed scene file live: loads the assets it uses and creates its objects,
    /// either replacing the open scenes or as a new additive scene. Returns the scene and
    /// any warnings (missing assets or script types).
    pub fn activate_scene_file(
        &mut self,
        file: &SceneFile,
        path: &str,
        additive: bool,
    ) -> (SceneHandle, Vec<String>) {
        let mut warnings = self.load_assets_of(file);
        let handle = if additive {
            self.create_scene(&file.name, Some(path))
        } else {
            self.clear(true);
            self.scene_name = file.name.clone();
            self.scene_path = Some(path.to_owned());
            // Lighting settings belong to the scene (Unity's Lighting window).
            let mut rs = RenderSettings::default();
            for (k, v) in &file.render_settings {
                // Moved to the project's Quality settings; older scenes still have them.
                if matches!(k.as_str(), "shadow_distance" | "lod_bias" | "msaa") {
                    continue;
                }
                if let Err(e) = rs.set_field(k, v.clone()) {
                    warnings.push(format!("render settings {k}: {e}"));
                }
            }
            self.resources.insert(rs);
            SceneHandle::MAIN
        };
        let (_, w) = self.with_spawn_scene(handle, |w| w.spawn(file, None));
        warnings.extend(w);
        // Bring in changes made to prefabs since the scene was saved.
        let updated = self.update_prefab_instances(&file.prefabs);
        if updated > 0 {
            oxide_core::debug_log!("Updated {updated} prefab instance(s) from their prefabs");
        }
        (handle, warnings)
    }

    /// Serializes one loaded scene (its root objects and their children).
    pub fn serialize_scene(&self, h: SceneHandle, name: &str) -> SceneFile {
        let roots = self.scene_roots(h);
        let mut file = self.serialize(&roots, name);
        if roots.is_empty() {
            // `serialize(&[])` means every object.
            file.objects.clear();
        }
        file
    }
}
