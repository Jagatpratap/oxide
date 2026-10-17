//! Terrain Toolbox grids from a heightmap, and terrain streaming.

use oxide_assets::AssetServer;
use oxide_math::Vector3;
use oxide_scene::World;
use oxide_terrain::toolbox::*;
use oxide_terrain::{Terrain, TerrainMeshes};

fn world(tag: &str) -> (World, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("oxide-terrain-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Assets")).unwrap();
    let mut w = World::new();
    oxide_scene::register_builtin(&mut w.registry);
    w.resources.insert(oxide_core::Time::new());
    w.resources.insert(AssetServer::new(&dir));
    (w, dir)
}

#[test]
fn grids_from_a_heightmap_and_streaming() {
    let (mut w, dir) = world("grid");
    let mut sys = oxide_terrain::install(&mut w);
    // A 9×9 ramp, west (0) to east (1).
    let hm = Heightmap {
        resolution: 9,
        heights: (0..81).map(|i| (i % 9) as f32 / 8.0).collect(),
    };
    let s = NewTerrainSettings {
        name: "Island".into(),
        tiles_x: 2,
        tiles_z: 1,
        tile_size: Vector3::new(100.0, 50.0, 100.0),
        heightmap_resolution: 33,
        grouping_id: 7,
        ..Default::default()
    };
    let tiles = create_terrain_grid(&mut w, &s, Some(&hm)).unwrap();
    assert_eq!(tiles.len(), 2);
    assert_eq!(w.position(tiles[1]), Vector3::new(100.0, 0.0, 0.0));
    assert!(dir.join("Assets/Terrains/Island_1_0.terraindata").exists());
    let data = |w: &World, i: usize| {
        w.get_component::<Terrain>(tiles[i])
            .unwrap()
            .data
            .clone()
            .unwrap()
    };
    {
        let a = w.resource::<AssetServer>();
        let (l, r) = (
            a.terrains.get(&data(&w, 0)).unwrap(),
            a.terrains.get(&data(&w, 1)).unwrap(),
        );
        // The seam: the west tile's east column is the east tile's west column.
        for z in 0..33 {
            assert!((l.sample(32, z) - r.sample(0, z)).abs() < 1e-5);
        }
        assert!((l.sample(32, 0) - 0.5).abs() < 1e-4, "{}", l.sample(32, 0));
        assert!((r.sample(32, 16) - 1.0).abs() < 1e-4);
    }
    assert_eq!(w.get_component::<Terrain>(tiles[1]).unwrap().grouping_id, 7);
    // Batch settings: turn streaming on everywhere.
    let n = apply_settings(
        &mut w,
        &tiles,
        &BatchSettings {
            template: Terrain {
                streaming: true,
                stream_distance: 50.0,
                ..Default::default()
            },
            fields: vec!["streaming", "stream_distance"],
        },
    );
    assert_eq!(n, 2);
    sys.always(&mut w);
    assert!(w.resource::<TerrainMeshes>().is_built(data(&w, 1).id()));
    // Far from the east tile: it unloads (data and meshes); the west one stays.
    let (loaded, unloaded) = stream(&mut w, Vector3::new(10.0, 0.0, 50.0));
    assert!(loaded.is_empty());
    assert_eq!(unloaded, ["Assets/Terrains/Island_1_0.terraindata"]);
    assert!(
        w.resource::<AssetServer>()
            .terrains
            .get(&data(&w, 1))
            .is_none()
    );
    assert!(!w.resource::<TerrainMeshes>().is_built(data(&w, 1).id()));
    assert!(
        w.resource::<AssetServer>()
            .terrains
            .get(&data(&w, 0))
            .is_some()
    );
    sys.always(&mut w);
    // Back near: reloaded from the file with its heights.
    let (loaded, _) = stream(&mut w, Vector3::new(120.0, 0.0, 50.0));
    assert_eq!(loaded, ["Assets/Terrains/Island_1_0.terraindata"]);
    let a = w.resource::<AssetServer>();
    let r = a.terrains.get(&data(&w, 1)).unwrap();
    assert!((r.sample(32, 16) - 1.0).abs() < 1e-4);
    let _ = std::fs::remove_dir_all(dir);
}
