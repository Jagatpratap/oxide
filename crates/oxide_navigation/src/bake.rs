//! Baking NavMeshSurfaces on background threads (the Inspector's Bake button), and
//! synchronous runtime builds (Unity `NavMeshSurface.BuildNavMesh`).

use crate::build::{BuildProgress, build_tiles};
use crate::collect::{SurfaceBuildInput, collect};
use crate::components::NavMeshSurface;
use crate::data::{NavMeshData, NavMeshTile};
use crate::system::NavigationRuntime;
use oxide_assets::{AssetServer, Handle};
use oxide_scene::{GameObjectId, SceneHandle, World};
use std::sync::Arc;
use std::sync::atomic::Ordering;

struct Job {
    surface: GameObjectId,
    input: Arc<SurfaceBuildInput>,
    handle: std::thread::JoinHandle<Result<Vec<NavMeshTile>, String>>,
    progress: Arc<BuildProgress>,
    started: std::time::Instant,
    save: bool,
}

/// World resource tracking background bakes.
#[derive(Default)]
pub struct NavMeshBake {
    jobs: Vec<Job>,
    /// Message of the last finished bake (Ok) or its error.
    pub last: Option<Result<String, String>>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BakeStatus {
    pub running: bool,
    pub progress: f32,
    pub surfaces: usize,
    pub last: Option<String>,
    pub error: Option<String>,
}

/// Folder for a scene's baked data: `Assets/Scenes/City.scene` → `Assets/Scenes/City`.
fn data_folder(world: &World) -> String {
    world
        .scene_info(SceneHandle::MAIN)
        .and_then(|s| s.path)
        .map(|p| p.trim_end_matches(".scene").to_owned())
        .filter(|p| p.starts_with("Assets/"))
        .unwrap_or_else(|| "Assets/NavMesh".to_owned())
}

fn surfaces(world: &World, only: Option<&[GameObjectId]>) -> Vec<GameObjectId> {
    let mut v: Vec<GameObjectId> = world
        .query::<NavMeshSurface>()
        .map(|(id, _)| id)
        .filter(|id| only.is_none_or(|o| o.contains(id)))
        .collect();
    v.dedup();
    v
}

/// Starts baking the given surfaces (all active ones if `None`). With `save`, the results are
/// written to `.navmesh` files and referenced from the surfaces. Returns how many started.
pub fn start(
    world: &mut World,
    only: Option<&[GameObjectId]>,
    save: bool,
) -> Result<usize, String> {
    if world
        .resources
        .get::<NavMeshBake>()
        .is_some_and(|b| !b.jobs.is_empty())
    {
        return Err("a NavMesh bake is already running".into());
    }
    let ids = surfaces(world, only);
    if ids.is_empty() {
        return Err(
            "no active NavMeshSurface to bake: add one (Component → Navigation → NavMesh Surface)"
                .into(),
        );
    }
    let mut jobs = Vec::new();
    let mut errors = Vec::new();
    for id in ids {
        let Some(s) = world.get_component::<NavMeshSurface>(id) else {
            continue;
        };
        let input = match collect(world, id, s) {
            Ok(i) => Arc::new(i),
            Err(e) => {
                let name = world.get(id).map_or_else(String::new, |o| o.name.clone());
                errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        let progress = Arc::new(BuildProgress::default());
        let (inp, prog) = (input.clone(), progress.clone());
        let handle = std::thread::Builder::new()
            .name("navmesh-bake".into())
            .spawn(move || build_tiles(&inp.source, &inp.volumes, &inp.settings, &inp.grid, &prog))
            .map_err(|e| e.to_string())?;
        jobs.push(Job {
            surface: id,
            input,
            handle,
            progress,
            started: std::time::Instant::now(),
            save,
        });
    }
    if jobs.is_empty() {
        return Err(errors.join("; "));
    }
    let n = jobs.len();
    if !world.resources.contains::<NavMeshBake>() {
        world.resources.insert(NavMeshBake::default());
    }
    let b = world
        .resources
        .get_mut::<NavMeshBake>()
        .unwrap_or_else(|| unreachable!());
    b.jobs = jobs;
    b.last = (!errors.is_empty()).then(|| Err(errors.join("; ")));
    Ok(n)
}

pub fn cancel(world: &mut World) {
    if let Some(b) = world.resources.get_mut::<NavMeshBake>() {
        for j in &b.jobs {
            j.progress.cancel.store(true, Ordering::Relaxed);
        }
    }
}

pub fn status(world: &World) -> BakeStatus {
    let Some(b) = world.resources.get::<NavMeshBake>() else {
        return BakeStatus {
            running: false,
            progress: 0.0,
            surfaces: 0,
            last: None,
            error: None,
        };
    };
    let progress = if b.jobs.is_empty() {
        0.0
    } else {
        b.jobs.iter().map(|j| j.progress.fraction()).sum::<f32>() / b.jobs.len() as f32
    };
    BakeStatus {
        running: !b.jobs.is_empty(),
        progress,
        surfaces: b.jobs.len(),
        last: b.last.as_ref().and_then(|r| r.as_ref().ok().cloned()),
        error: b.last.as_ref().and_then(|r| r.as_ref().err().cloned()),
    }
}

fn make_data(input: &SurfaceBuildInput, tiles: Vec<NavMeshTile>) -> NavMeshData {
    NavMeshData {
        agent_type_id: input.agent_type,
        settings: input.settings.clone(),
        grid: input.grid,
        position: input.position,
        rotation: input.rotation,
        tiles,
    }
}

/// Collects finished bakes: saves them and hands them to their surfaces. Returns a message
/// when the whole bake finished.
pub fn poll(world: &mut World) -> Option<Result<String, String>> {
    let b = world.resources.get_mut::<NavMeshBake>()?;
    if b.jobs.is_empty() || !b.jobs.iter().all(|j| j.handle.is_finished()) {
        return None;
    }
    let jobs = std::mem::take(&mut b.jobs);
    let mut msgs = Vec::new();
    let mut errors = Vec::new();
    let folder = data_folder(world);
    for j in jobs {
        let secs = j.started.elapsed().as_secs_f32();
        let tiles = match j.handle.join() {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => {
                errors.push(e);
                continue;
            }
            Err(_) => {
                errors.push("the bake thread crashed".to_owned());
                continue;
            }
        };
        let data = Arc::new(make_data(&j.input, tiles));
        let name = world
            .get(j.surface)
            .map_or_else(|| "Surface".to_owned(), |o| o.name.clone());
        let polys = data.polygon_count();
        let mut handle = None;
        if j.save {
            let path = format!("{folder}/NavMesh-{}.navmesh", sanitize(&name));
            match save_data(world, &path, &data) {
                Ok(()) => handle = Some(Handle::<NavMeshData>::from_path(path.clone())),
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        if let Some(rt) = world.resources.get_mut::<NavigationRuntime>() {
            rt.sources.insert(j.surface, j.input.clone());
            if let Some(h) = &handle {
                rt.cache.insert(h.path().to_owned(), data.clone());
            }
        }
        if let Some(s) = world.get_component_mut::<NavMeshSurface>(j.surface) {
            if handle.is_some() {
                s.navmesh_data = handle;
                s.runtime.data = None;
            } else {
                s.runtime.data = Some(data.clone());
            }
        }
        msgs.push(format!(
            "{name}: {polys} polygons in {} tiles ({secs:.2} s)",
            data.tiles.len()
        ));
    }
    let r = if errors.is_empty() {
        Ok(format!("NavMesh baked. {}", msgs.join("; ")))
    } else if msgs.is_empty() {
        Err(errors.join("; "))
    } else {
        Err(format!(
            "{}; failed: {}",
            msgs.join("; "),
            errors.join("; ")
        ))
    };
    if let Some(b) = world.resources.get_mut::<NavMeshBake>() {
        b.last = Some(r.clone());
    }
    Some(r)
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn save_data(world: &World, path: &str, data: &NavMeshData) -> Result<(), String> {
    let root = world
        .resources
        .get::<AssetServer>()
        .map(|a| a.root.clone())
        .ok_or("no AssetServer")?;
    let abs = root.join(path);
    if let Some(dir) = abs.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&abs, data.to_bytes()).map_err(|e| format!("could not write {path}: {e}"))
}

/// Unity `NavMeshSurface.BuildNavMesh()`: builds now (blocking) and uses the result without
/// saving it. Returns the polygon count.
pub fn build_now(world: &mut World, surface: GameObjectId) -> Result<usize, String> {
    let s = world
        .get_component::<NavMeshSurface>(surface)
        .ok_or("no NavMeshSurface on that object")?;
    let input = Arc::new(collect(world, surface, s)?);
    let progress = BuildProgress::default();
    let tiles = build_tiles(
        &input.source,
        &input.volumes,
        &input.settings,
        &input.grid,
        &progress,
    )?;
    let data = Arc::new(make_data(&input, tiles));
    let n = data.polygon_count();
    if let Some(rt) = world.resources.get_mut::<NavigationRuntime>() {
        rt.sources.insert(surface, input);
    }
    if let Some(s) = world.get_component_mut::<NavMeshSurface>(surface) {
        s.runtime.data = Some(data);
    }
    Ok(n)
}

/// Unity's Clear button: forgets the surface's NavMesh and deletes its file.
pub fn clear(world: &mut World, surface: GameObjectId) -> Result<(), String> {
    let s = world
        .get_component_mut::<NavMeshSurface>(surface)
        .ok_or("no NavMeshSurface on that object")?;
    let old = s.navmesh_data.take();
    s.runtime.data = None;
    if let Some(h) = old {
        let root = world.resources.get::<AssetServer>().map(|a| a.root.clone());
        if let Some(rt) = world.resources.get_mut::<NavigationRuntime>() {
            rt.cache.remove(h.path());
        }
        if let Some(root) = root {
            let _ = std::fs::remove_file(root.join(h.path()));
        }
    }
    Ok(())
}
