//! World streaming authoring: splits a big scene into grid cells that a [`WorldStreamer`]
//! loads around the player at runtime.

use oxide_math::Vector3;
use oxide_scene::{Camera, GameObjectId, Light, SceneHandle, World};
use oxide_standard::{StreamingCell, WorldStreamer};
use std::collections::BTreeMap;

/// What [`split_scene_into_cells`] moves and where it writes.
#[derive(Clone, Debug)]
pub struct SplitOptions {
    /// Cell width and depth in meters.
    pub cell_size: f32,
    /// Folder for the cell scenes (relative to the project).
    pub folder: String,
    /// Move only root objects with static flags (level geometry); dynamic objects stay.
    pub only_static: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SplitCell {
    pub scene: String,
    pub objects: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SplitReport {
    pub cells: Vec<SplitCell>,
    pub moved_roots: usize,
    pub kept_roots: usize,
}

/// Whether a root object should stay in the main scene (cameras, the sun, terrain,
/// streamers: things the whole world needs).
fn stays_in_main(world: &World, id: GameObjectId) -> bool {
    world.has_component::<Camera>(id)
        || world.has_component::<WorldStreamer>(id)
        || world.has_component::<oxide_terrain::Terrain>(id)
        || world
            .get_component::<Light>(id)
            .is_some_and(|l| l.light_type == oxide_scene::LightType::Directional)
}

/// Moves the main scene's root objects into one scene per `cell_size` grid cell (by each
/// root's position), saves the cells under `folder`, unloads them and adds a
/// [`WorldStreamer`] listing them to the main scene (or updates an existing one). Save the
/// main scene afterwards to keep the change.
pub fn split_scene_into_cells(
    world: &mut World,
    opts: &SplitOptions,
) -> Result<SplitReport, String> {
    if opts.cell_size < 1.0 {
        return Err("cell_size must be at least 1 m".into());
    }
    let folder = opts.folder.trim_end_matches('/').to_owned();
    if folder.is_empty() {
        return Err("folder is required".into());
    }
    let roots = world.scene_roots(SceneHandle::MAIN);
    let mut groups: BTreeMap<(i32, i32), Vec<GameObjectId>> = BTreeMap::new();
    let mut kept = 0;
    for r in roots {
        let movable = !stays_in_main(world, r)
            && (!opts.only_static || world.get(r).is_some_and(|o| o.is_static()));
        if !movable {
            kept += 1;
            continue;
        }
        let p = world.position(r);
        let key = (
            (p.x / opts.cell_size).floor() as i32,
            (p.z / opts.cell_size).floor() as i32,
        );
        groups.entry(key).or_default().push(r);
    }
    if groups.is_empty() {
        return Err(if opts.only_static {
            "no static root objects to move (mark level geometry Static, or pass only_static: false)"
                .into()
        } else {
            "no root objects to move".into()
        });
    }
    let base = world.scene_name.clone();
    let mut cells = Vec::new();
    let mut streamed = Vec::new();
    let mut moved = 0;
    for ((ix, iz), ids) in groups {
        let name = format!("{base}_Cell_{ix}_{iz}");
        let path = format!("{folder}/{name}.scene");
        let h = world.create_scene(&name, Some(&path));
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for &id in &ids {
            let y = world.position(id).y;
            lo = lo.min(y);
            hi = hi.max(y);
            world.move_to_scene(id, h)?;
        }
        let objects = crate::save_scene_as(world, h, &path)?;
        world.unload_scene(h)?;
        moved += ids.len();
        let s = opts.cell_size;
        streamed.push(StreamingCell {
            scene: path.clone(),
            center: Vector3::new(
                (ix as f32 + 0.5) * s,
                (lo + hi) * 0.5,
                (iz as f32 + 0.5) * s,
            ),
            size: Vector3::new(s, (hi - lo).max(1.0), s),
        });
        cells.push(SplitCell {
            scene: path,
            objects,
        });
    }
    let streamer = match world.find_objects_of_type::<WorldStreamer>(true).first() {
        Some(&id) => id,
        None => {
            let id = world.create("World Streamer");
            let mut ws = WorldStreamer::default();
            ws.load_distance = opts.cell_size * 1.5;
            ws.unload_distance = opts.cell_size * 2.0;
            world.add_component(id, ws);
            id
        }
    };
    if let Some(ws) = world.get_component_mut::<WorldStreamer>(streamer) {
        // Replace cells from an earlier split of the same folder; keep others.
        ws.cells
            .retain(|c| !c.scene.starts_with(&format!("{folder}/")));
        ws.cells.extend(streamed);
    }
    Ok(SplitReport {
        cells,
        moved_roots: moved,
        kept_roots: kept,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_scene::{Phase, StaticEditorFlags};

    /// A 300×300 m map: a static prop in the middle of each 100 m cell, a dynamic car and a
    /// MainCamera at (50, 1, 50).
    fn map(tag: &str) -> World {
        let dir = std::env::temp_dir().join(format!("oxide-stream-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets")).unwrap();
        let mut w = World::new();
        crate::engine::init_world(&mut w, dir);
        oxide_standard::register(&mut w.registry);
        w.scene_name = "Map".into();
        for i in 0..3 {
            for j in 0..3 {
                let p = w.create(&format!("Prop {i} {j}"));
                w.get_mut(p).unwrap().static_flags = StaticEditorFlags::EVERYTHING;
                w.set_position(
                    p,
                    Vector3::new(50.0 + i as f32 * 100.0, 0.0, 50.0 + j as f32 * 100.0),
                );
                let child = w.create("Detail");
                w.set_parent(child, Some(p), false).unwrap();
            }
        }
        w.create("Car");
        let cam = w.create("Main Camera");
        w.get_mut(cam).unwrap().tag = "MainCamera".into();
        w.add_component(cam, Camera::default());
        w.set_position(cam, Vector3::new(50.0, 1.0, 50.0));
        w
    }

    /// Runs frames until scene operations settle.
    fn settle(w: &mut World) {
        for _ in 0..400 {
            w.run_start();
            w.run_phase(Phase::Update);
            w.pump_scene_operations(8);
            let busy = w
                .try_resource::<oxide_scene::SceneOperations>()
                .is_some_and(|o| o.pending_count() > 0)
                || w.query::<WorldStreamer>().any(|(_, s)| {
                    s.cell_states()
                        .iter()
                        .any(|s| *s == "loading" || *s == "unloading")
                });
            if !busy {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("streaming did not settle");
    }

    fn present(w: &World) -> Vec<String> {
        let mut v: Vec<String> = w
            .objects()
            .filter(|(_, o)| o.name.starts_with("Prop"))
            .map(|(_, o)| o.name.clone())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn split_then_stream_around_the_camera() {
        let mut w = map("split");
        let opts = SplitOptions {
            cell_size: 100.0,
            folder: "Assets/Scenes/Map_Cells".into(),
            only_static: true,
        };
        let r = split_scene_into_cells(&mut w, &opts).unwrap();
        assert_eq!(r.cells.len(), 9);
        assert_eq!(r.moved_roots, 9);
        assert_eq!(r.kept_roots, 2); // the car and the camera
        assert!(r.cells.iter().all(|c| c.objects == 2));
        let root = w.resource::<oxide_assets::AssetServer>().root.clone();
        assert!(
            root.join("Assets/Scenes/Map_Cells/Map_Cell_2_1.scene")
                .exists()
        );
        assert!(present(&w).is_empty());
        assert_eq!(w.loaded_scenes().len(), 1);
        let streamer = w.find("World Streamer").unwrap();
        {
            let ws = w.get_component_mut::<WorldStreamer>(streamer).unwrap();
            assert_eq!(ws.cells.len(), 9);
            ws.load_distance = 60.0;
            ws.unload_distance = 120.0;
            ws.max_concurrent_loads = 9;
        }

        w.start_scripts();
        settle(&mut w);
        // Within 60 m of (50, 50): its own cell and the two beside it.
        assert_eq!(present(&w), ["Prop 0 0", "Prop 0 1", "Prop 1 0"]);
        assert_eq!(w.loaded_scenes().len(), 4);
        let p = w.find("Prop 0 1").unwrap();
        assert_ne!(w.get(p).unwrap().scene(), SceneHandle::MAIN);
        assert_eq!(w.position(p), Vector3::new(50.0, 0.0, 150.0));

        // Drive to the far corner: the old cells unload, the new ones load.
        let cam = w.find("Main Camera").unwrap();
        w.set_position(cam, Vector3::new(250.0, 1.0, 250.0));
        settle(&mut w);
        assert_eq!(present(&w), ["Prop 1 2", "Prop 2 1", "Prop 2 2"]);
        assert_eq!(w.loaded_scenes().len(), 4);

        // Between the two thresholds nothing changes (no load/unload flicker).
        w.set_position(cam, Vector3::new(250.0, 1.0, 140.0));
        settle(&mut w);
        assert!(present(&w).contains(&"Prop 1 2".to_owned()));

        // Disabling the streamer unloads everything it loaded.
        let cid = w.component_id::<WorldStreamer>(streamer).unwrap();
        w.set_component_enabled(streamer, cid, false);
        settle(&mut w);
        assert!(present(&w).is_empty());
        assert_eq!(w.loaded_scenes().len(), 1);
        assert!(w.find("Car").is_some());
    }
}
