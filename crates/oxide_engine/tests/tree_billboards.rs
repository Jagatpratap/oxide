//! Terrain tree billboards: baking a prototype's views (GPU), the mesh / billboard split by
//! distance, and trees bending in Wind Zones.

use oxide_assets::terrain::{TreeInstance, TreePrototype};
use oxide_assets::{AssetId, AssetServer, Handle, TerrainData};
use oxide_core::Time;
use oxide_math::{Color, Quaternion, Vector3};
use oxide_render::*;
use oxide_scene::World;
use oxide_terrain::{Terrain, TreeBillboard, TreeBillboards};

fn tree_assets() -> AssetServer {
    let mut a = AssetServer::new(std::env::temp_dir());
    oxide_terrain::register_builtin_tree(&mut a);
    a
}

fn proto() -> TreePrototype {
    TreePrototype::new(
        Handle::from_path("builtin:tree/simple"),
        vec![
            Handle::from_path("builtin:tree/bark.mat"),
            Handle::from_path("builtin:tree/leaves.mat"),
        ],
    )
}

#[test]
fn bakes_eight_views_with_transparent_backgrounds() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; skipped");
        return;
    };
    let assets = tree_assets();
    let mut r = SceneRenderer::new(&gpu);
    let p = proto();
    let (tex, b) =
        oxide_engine::tree_billboards::bake(&gpu, &mut r, &assets, &p.mesh, &p.materials)
            .expect("baked");
    assert_eq!(tex.width, 128 * oxide_engine::tree_billboards::VIEWS);
    assert!(
        b.height > b.width * 0.8,
        "a tall tree: {} x {}",
        b.width,
        b.height
    );
    assert_eq!(b.images.len(), 8);
    let px = |x: u32, y: u32| {
        let o = ((y * tex.width + x) * 4) as usize;
        [
            tex.mips[0][o],
            tex.mips[0][o + 1],
            tex.mips[0][o + 2],
            tex.mips[0][o + 3],
        ]
    };
    let h = tex.height;
    for v in 0..8 {
        let cx = v * 128 + 64;
        // The trunk near the bottom and leaves above are opaque; the corners are clear.
        assert!(
            px(cx, h - 4)[3] > 200,
            "view {v}: trunk {:?}",
            px(cx, h - 4)
        );
        assert!(
            px(cx, h / 3)[3] > 200,
            "view {v}: canopy {:?}",
            px(cx, h / 3)
        );
        assert_eq!(px(v * 128 + 2, 2)[3], 0, "view {v}: clear corner");
        // Leaves are green, not tinted by the background.
        let c = px(cx, h / 3);
        assert!(c[1] > c[0] && c[1] > c[2], "{c:?}");
    }
}

fn forest(max_mesh_trees: u32) -> (World, GameObjectId) {
    let mut w = World::new();
    oxide_scene::register_builtin(&mut w.registry);
    w.resources.insert(Time::new());
    let mut assets = tree_assets();
    let mut data = TerrainData::flat(Vector3::new(1000.0, 10.0, 1000.0), 33);
    data.tree_prototypes.push(proto());
    for (x, z) in [(0.01, 0.01), (0.5, 0.5), (0.9, 0.9)] {
        data.trees.push(TreeInstance {
            position: Vector3::new(x, 0.0, z),
            prototype: 0,
            width_scale: 1.0,
            height_scale: 1.0,
            rotation: 0.0,
            color: Color::WHITE,
        });
    }
    let h = assets.terrains.add(data);
    w.resources.insert(assets);
    let mut bb = TreeBillboards::default();
    bb.by_mesh.insert(
        AssetId::from_path("builtin:tree/simple"),
        TreeBillboard {
            texture: AssetId::from_path("test:billboard"),
            width: 4.0,
            height: 9.0,
            bottom: 0.0,
            images: (0..8).map(|v| [v as f32 / 8.0, 0.0, 0.125, 1.0]).collect(),
        },
    );
    w.resources.insert(bb);
    let id = w.create("Terrain");
    w.add_component(
        id,
        Terrain {
            data: Some(h),
            max_mesh_trees,
            ..Default::default()
        },
    );
    // Build the chunk meshes (what the Terrain system does every frame).
    let mut sys = oxide_terrain::install(&mut w);
    sys.always(&mut w);
    (w, id)
}

use oxide_scene::GameObjectId;

#[test]
fn near_trees_are_meshes_and_far_trees_billboards() {
    let (w, _) = forest(50);
    let camera = Vector3::new(5.0, 2.0, 5.0);
    let (mut terrains, mut draws) = (Vec::new(), Vec::new());
    oxide_terrain::extract(&w, camera, &mut terrains, &mut draws);
    let tree_mesh = AssetId::from_path("builtin:tree/simple");
    let mesh_trees: Vec<_> = draws.iter().filter(|d| d.mesh == tree_mesh).collect();
    assert_eq!(mesh_trees.len(), 2, "the near tree's two submeshes");
    let mut bbs = Vec::new();
    oxide_terrain::extract_tree_billboards(&w, camera, &mut bbs);
    assert_eq!(bbs.len(), 1);
    assert_eq!(bbs[0].particles.len(), 2, "the two far trees");
    let p = &bbs[0].particles[0];
    assert_eq!(p.mode, ParticleMode::Vertical as u8 as f32);
    assert!((p.aspect - 9.0 / 4.0).abs() < 1e-4);
    // With Max Mesh Trees 0 every tree is a billboard.
    let (w, _) = forest(0);
    let mut draws = Vec::new();
    oxide_terrain::extract(&w, camera, &mut Vec::new(), &mut draws);
    assert!(draws.iter().all(|d| d.mesh != tree_mesh));
    let mut bbs = Vec::new();
    oxide_terrain::extract_tree_billboards(&w, camera, &mut bbs);
    assert_eq!(bbs[0].particles.len(), 3);
}

#[test]
fn trees_bend_in_wind_zones() {
    let (mut w, terrain) = forest(50);
    let camera = Vector3::new(5.0, 2.0, 5.0);
    let tree_mesh = AssetId::from_path("builtin:tree/simple");
    let up = |w: &World| {
        let mut draws = Vec::new();
        oxide_terrain::extract(w, camera, &mut Vec::new(), &mut draws);
        let d = draws.iter().find(|d| d.mesh == tree_mesh).unwrap();
        d.model.transform_vector3(Vector3::Y).normalize()
    };
    assert!((up(&w) - Vector3::Y).length() < 1e-4, "no wind: upright");
    // Bend factor 0 ignores the wind (Unity's default).
    let zone = w.create_at("Wind", Vector3::ZERO, Quaternion::IDENTITY);
    w.add_component(
        zone,
        oxide_scene::wind::WindZone {
            wind_main: 3.0,
            wind_turbulence: 0.0,
            wind_pulse_magnitude: 0.0,
            ..Default::default()
        },
    );
    assert!((up(&w) - Vector3::Y).length() < 1e-4);
    let h = w
        .get_component::<Terrain>(terrain)
        .unwrap()
        .data
        .clone()
        .unwrap();
    w.resource_mut::<AssetServer>()
        .terrains
        .get_mut(&h)
        .unwrap()
        .tree_prototypes[0]
        .bend_factor = 1.0;
    let bent = up(&w);
    assert!(bent.z > 0.02, "leans with the wind (+Z): {bent}");
}
