use crate::build::{AreaVolume, BuildProgress, BuildSettings, BuildSource, TileGrid, build_tiles};
use crate::*;
use oxide_assets::AssetServer;
use oxide_core::Time;
use oxide_math::{Quaternion, Vector3};
use oxide_physics::BoxCollider;
use oxide_scene::{GameObjectId, Phase, System, Timers, World};
use std::sync::Arc;

fn v(x: f32, y: f32, z: f32) -> Vector3 {
    Vector3::new(x, y, z)
}

/// Adds an axis-aligned solid box (all six faces).
fn add_box(src: &mut BuildSource, min: Vector3, max: Vector3, area: Option<u8>) {
    let c = |i: u32| {
        v(
            if i & 1 != 0 { max.x } else { min.x },
            if i & 2 != 0 { max.y } else { min.y },
            if i & 4 != 0 { max.z } else { min.z },
        )
    };
    let verts: Vec<Vector3> = (0..8).map(c).collect();
    let center = (min + max) * 0.5;
    let mut tris = vec![
        [0, 1, 3],
        [0, 3, 2],
        [4, 6, 7],
        [4, 7, 5],
        [0, 4, 5],
        [0, 5, 1],
        [2, 3, 7],
        [2, 7, 6],
        [0, 2, 6],
        [0, 6, 4],
        [1, 5, 7],
        [1, 7, 3],
    ];
    for t in &mut tris {
        let [a, b, cc] = t.map(|i: u32| verts[i as usize]);
        if (b - a).cross(cc - a).dot((a + b + cc) / 3.0 - center) < 0.0 {
            t.swap(1, 2);
        }
    }
    src.add_triangles(&verts, &tris, area);
}

/// A flat floor from (-h, 0, -h) to (h, 0, h), as a thin slab.
fn floor(src: &mut BuildSource, h: f32) {
    add_box(src, v(-h, -0.2, -h), v(h, 0.0, h), Some(0));
}

fn settings() -> BuildSettings {
    BuildSettings {
        tile_size: 64,
        ..Default::default()
    }
}

fn bake(src: &BuildSource, vols: &[AreaVolume], s: &BuildSettings) -> NavMesh {
    let (bmin, bmax) = src.bounds().unwrap();
    let grid = TileGrid::new(bmin, bmax, s);
    let tiles = build_tiles(src, vols, s, &grid, &BuildProgress::default()).unwrap();
    let data = NavMeshData {
        agent_type_id: 0,
        settings: s.clone(),
        grid,
        position: Vector3::ZERO,
        rotation: Quaternion::IDENTITY,
        tiles,
    };
    let mut nav = NavMesh::new();
    nav.agent_types = vec![(0, s.clone())];
    nav.add_data(Arc::new(data), Vector3::ZERO, Quaternion::IDENTITY);
    nav
}

fn xz_dist(a: Vector3, b: Vector3) -> f32 {
    Vector3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

fn filter() -> QueryFilter {
    QueryFilter::new(0)
}

#[test]
fn flat_floor_is_walkable_and_eroded_by_radius() {
    let mut src = BuildSource::default();
    floor(&mut src, 10.0);
    let nav = bake(&src, &[], &settings());
    assert!(nav.polygon_count() > 0);
    // 20 m floor with 64-voxel (10.7 m) tiles: 2×2 tiles.
    assert_eq!(nav.tile_count(), 4);
    let hit = nav
        .sample_position(v(0.0, 0.0, 0.0), 1.0, &filter())
        .unwrap();
    assert!(hit.position.y.abs() < 0.1, "{hit:?}");
    // Near the edge the NavMesh stops one agent radius (0.5 m) from the drop.
    let edge = nav
        .sample_position(v(9.9, 0.0, 0.0), 2.0, &filter())
        .unwrap();
    assert!(
        (edge.position.x - 9.5).abs() < 0.25,
        "edge at {:?}",
        edge.position
    );
    let e = nav.find_closest_edge(v(8.0, 0.0, 0.0), &filter()).unwrap();
    assert!((e.distance - 1.5).abs() < 0.3, "{e:?}");
    assert!(e.normal.x < -0.9, "normal should point inward: {e:?}");
}

#[test]
fn path_goes_around_a_wall_with_corners_at_its_ends() {
    let mut src = BuildSource::default();
    floor(&mut src, 10.0);
    add_box(&mut src, v(-6.0, 0.0, -0.25), v(6.0, 2.0, 0.25), Some(0));
    let nav = bake(&src, &[], &settings());
    let p = nav.calculate_path(v(0.0, 0.0, -5.0), v(0.0, 0.0, 5.0), &filter());
    assert_eq!(p.status, NavMeshPathStatus::PathComplete);
    assert!(p.corners.len() >= 4, "{:?}", p.corners);
    // Around a wall end: corners about one radius past x = ±6.
    let mid: Vec<_> = p.corners[1..p.corners.len() - 1].to_vec();
    assert!(
        mid.iter().all(|c| c.x.abs() > 5.9 && c.x.abs() < 7.2),
        "{mid:?}"
    );
    let len = p.length();
    let ideal = 2.0 * (6.5f32 * 6.5 + 5.0 * 5.0).sqrt();
    assert!(
        len > ideal - 0.5 && len < ideal + 1.5,
        "length {len} vs {ideal}"
    );
    // A straight NavMesh raycast through the wall is blocked, facing the start.
    let hit = nav.raycast(v(0.0, 0.0, -5.0), v(0.0, 0.0, 5.0), &filter());
    assert!(hit.hit);
    assert!((hit.position.z + 0.75).abs() < 0.25, "{hit:?}");
    assert!(hit.normal.z < -0.9, "{hit:?}");
    let free = nav.raycast(v(-3.0, 0.0, -5.0), v(3.0, 0.0, -3.0), &filter());
    assert!(!free.hit, "{free:?}");
}

#[test]
fn unobstructed_path_across_tiles_is_one_straight_segment() {
    let mut src = BuildSource::default();
    floor(&mut src, 40.0);
    let nav = bake(&src, &[], &settings());
    assert!(nav.tile_count() >= 49);
    let p = nav.calculate_path(v(-35.0, 0.0, -30.0), v(33.0, 0.0, 31.0), &filter());
    assert_eq!(p.status, NavMeshPathStatus::PathComplete);
    assert_eq!(p.corners.len(), 2, "{:?}", p.corners);
}

#[test]
fn steps_and_slopes_follow_the_agent_settings() {
    let mut src = BuildSource::default();
    floor(&mut src, 10.0);
    // A 0.5 m step (climbable) and a 1.2 m block (not), both 3×3 m.
    add_box(&mut src, v(-7.0, 0.0, -1.5), v(-4.0, 0.5, 1.5), Some(0));
    add_box(&mut src, v(4.0, 0.0, -1.5), v(7.0, 1.2, 1.5), Some(0));
    let nav = bake(&src, &[], &settings());
    let step = nav.calculate_path(v(0.0, 0.0, 0.0), v(-5.5, 0.5, 0.0), &filter());
    assert_eq!(step.status, NavMeshPathStatus::PathComplete, "{step:?}");
    assert!((step.corners.last().unwrap().y - 0.5).abs() < 0.15);
    // The top of the high block is an island (smaller than min region area after erosion
    // or unreachable): no complete path onto it.
    let high = nav.calculate_path(v(0.0, 0.0, 0.0), v(5.5, 1.2, 0.0), &filter());
    assert_ne!(high.status, NavMeshPathStatus::PathComplete, "{high:?}");
}

#[test]
fn steep_ramp_is_not_walkable_but_gentle_one_is() {
    let ramp = |deg: f32| {
        let mut src = BuildSource::default();
        let len = 6.0;
        let h = len * deg.to_radians().tan();
        let verts = [
            v(-2.0, 0.0, 0.0),
            v(-2.0, h, len),
            v(2.0, h, len),
            v(2.0, 0.0, 0.0),
        ];
        src.add_triangles(&verts, &[[0, 1, 2], [0, 2, 3]], Some(0));
        let nav = bake(&src, &[], &settings());
        nav.sample_position(v(0.0, h * 0.5, len * 0.5), 0.3, &filter())
    };
    let gentle = ramp(30.0).expect("30° ramp walkable");
    assert!(
        (gentle.position.y - 6.0 * 30f32.to_radians().tan() * 0.5).abs() < 0.2,
        "{gentle:?}"
    );
    assert!(
        ramp(50.0).is_none(),
        "50° is steeper than the 45° max slope"
    );
}

#[test]
fn area_costs_and_masks_steer_paths() {
    let mut src = BuildSource::default();
    floor(&mut src, 10.0);
    // A 4 m wide band of area 3 across the middle, leaving 1.5 m gaps at both ends.
    let mud = AreaVolume {
        center: v(-1.5, 0.0, 0.0),
        half: v(7.0, 1.0, 2.0),
        rotation: Quaternion::IDENTITY,
        cylinder: false,
        area: Some(3),
    };
    let mut nav = bake(&src, &[mud], &settings());
    let (a, b) = (v(-1.5, 0.0, -6.0), v(-1.5, 0.0, 6.0));
    let straight = nav.calculate_path(a, b, &filter());
    assert_eq!(straight.status, NavMeshPathStatus::PathComplete);
    assert!(straight.length() < 12.5, "cost 1: straight through");
    nav.area_costs[3] = 10.0;
    let around = nav.calculate_path(a, b, &filter());
    // Through one of the gaps: left of x = -8.5 or right of x = 5.5.
    let through_gap = |p: &NavMeshPath| p.corners.iter().any(|c| c.x < -8.4 || c.x > 5.4);
    assert!(through_gap(&around), "{:?}", around.corners);
    nav.area_costs[3] = 1.0;
    let masked = nav.calculate_path(a, b, &filter().with_mask(ALL_AREAS & !(1 << 3)));
    assert_eq!(masked.status, NavMeshPathStatus::PathComplete);
    assert!(through_gap(&masked), "{:?}", masked.corners);
    let at = nav
        .sample_position(v(-1.5, 0.0, 0.0), 0.5, &filter())
        .unwrap();
    assert_eq!(at.mask, 1 << 3);
}

#[test]
fn off_mesh_link_bridges_a_gap() {
    let mut src = BuildSource::default();
    add_box(&mut src, v(-5.0, -0.2, -10.0), v(5.0, 0.0, -1.0), Some(0));
    add_box(&mut src, v(-5.0, -0.2, 1.0), v(5.0, 0.0, 10.0), Some(0));
    let mut nav = bake(&src, &[], &settings());
    let (a, b) = (v(0.0, 0.0, -6.0), v(0.0, 0.0, 6.0));
    assert_eq!(
        nav.calculate_path(a, b, &filter()).status,
        NavMeshPathStatus::PathPartial
    );
    let id = nav.add_link(OffMeshLink {
        start: v(0.0, 0.0, -1.6),
        end: v(0.0, 0.0, 1.6),
        width: 0.0,
        bidirectional: true,
        area: JUMP as u8,
        cost_modifier: -1.0,
        agent_type: 0,
        activated: true,
        owner: 0,
        start_poly: None,
        end_poly: None,
    });
    assert!(nav.link(id).unwrap().is_connected());
    let p = nav.calculate_path(a, b, &filter());
    assert_eq!(p.status, NavMeshPathStatus::PathComplete);
    assert!(
        p.corners.iter().any(|c| (c.z + 1.6).abs() < 0.05),
        "{:?}",
        p.corners
    );
    assert!(
        p.corners.iter().any(|c| (c.z - 1.6).abs() < 0.05),
        "{:?}",
        p.corners
    );
    // Back the other way too (bidirectional), and not once deactivated.
    assert_eq!(
        nav.calculate_path(b, a, &filter()).status,
        NavMeshPathStatus::PathComplete
    );
    nav.set_link_activated(id, false);
    assert_eq!(
        nav.calculate_path(a, b, &filter()).status,
        NavMeshPathStatus::PathPartial
    );
}

#[test]
fn navmesh_file_round_trips() {
    let mut src = BuildSource::default();
    floor(&mut src, 10.0);
    add_box(&mut src, v(-2.0, 0.0, -2.0), v(2.0, 1.0, 2.0), Some(0));
    let s = settings();
    let (bmin, bmax) = src.bounds().unwrap();
    let grid = TileGrid::new(bmin, bmax, &s);
    let tiles = build_tiles(&src, &[], &s, &grid, &BuildProgress::default()).unwrap();
    let d = NavMeshData {
        agent_type_id: 3,
        settings: s,
        grid,
        position: v(1.0, 2.0, 3.0),
        rotation: Quaternion::from_rotation_y(0.5),
        tiles,
    };
    let back = NavMeshData::from_bytes(&d.to_bytes()).unwrap();
    assert_eq!(back.tiles, d.tiles);
    assert_eq!(back.agent_type_id, 3);
    assert_eq!(back.grid, d.grid);
    assert!(NavMeshData::from_bytes(&d.to_bytes()[..100]).is_err());
}

// ---- Components in a World ----

struct Sim {
    world: World,
    sys: Box<dyn System>,
}

impl Sim {
    fn new() -> Self {
        let mut world = World::new();
        oxide_scene::register_builtin(&mut world.registry);
        world.resources.insert(Time::new());
        world.resources.insert(AssetServer::new("."));
        world.resources.insert(Timers::default());
        let sys = install(&mut world);
        Self { world, sys }
    }

    fn step(&mut self, seconds: f32) {
        let frames = (seconds * 60.0).round() as u32;
        for _ in 0..frames {
            let w = &mut self.world;
            w.resource_mut::<Time>().begin_frame(1.0 / 60.0);
            w.run_start();
            w.run_phase(Phase::Update);
            self.sys.update(w);
            self.sys.always(w);
        }
    }

    fn solid(&mut self, name: &str, center: Vector3, size: Vector3) -> GameObjectId {
        let g = self.world.create_at(name, center, Quaternion::IDENTITY);
        self.world.add_component(
            g,
            BoxCollider {
                size,
                ..Default::default()
            },
        );
        g
    }

    /// A 30×30 m floor with a NavMeshSurface baked from colliders.
    fn level(&mut self) -> GameObjectId {
        self.solid("Floor", v(0.0, -0.25, 0.0), v(30.0, 0.5, 30.0));
        let s = self.world.create("NavMesh");
        self.world.add_component(
            s,
            NavMeshSurface {
                use_geometry: NavMeshCollectGeometry::PhysicsColliders,
                ..Default::default()
            },
        );
        s
    }

    fn bake(&mut self, surface: GameObjectId) {
        bake::build_now(&mut self.world, surface).unwrap();
        self.sys.always(&mut self.world);
    }

    fn agent(&mut self, name: &str, at: Vector3) -> GameObjectId {
        let a = self.world.create_at(name, at, Quaternion::IDENTITY);
        self.world.add_component(a, NavMeshAgent::default());
        a
    }

    fn agent_mut(&mut self, id: GameObjectId) -> &mut NavMeshAgent {
        self.world.get_component_mut::<NavMeshAgent>(id).unwrap()
    }
}

#[test]
fn surface_bakes_from_colliders_and_agent_walks_to_destination() {
    let mut s = Sim::new();
    let surf = s.level();
    // A wall the agent must walk around; it is part of the bake.
    s.solid("Wall", v(0.0, 1.0, 0.0), v(16.0, 2.0, 0.5));
    s.bake(surf);
    assert!(s.world.resource::<NavMesh>().polygon_count() > 0);
    let a = s.agent("Agent", v(0.0, 0.0, -8.0));
    s.step(0.1);
    assert!(s.agent_mut(a).is_on_nav_mesh());
    s.agent_mut(a).set_destination(v(0.0, 0.0, 8.0));
    assert!(s.agent_mut(a).path_pending());
    s.step(0.05);
    let ag = s.agent_mut(a);
    assert!(!ag.path_pending());
    assert_eq!(ag.path_status(), NavMeshPathStatus::PathComplete);
    let len = ag.remaining_distance();
    assert!(len > 20.0, "must go around the wall: {len}");
    s.step(12.0);
    let p = s.world.position(a);
    assert!(xz_dist(p, v(0.0, 0.0, 8.0)) < 0.2, "arrived at {p}");
    let ag = s.agent_mut(a);
    assert!(ag.remaining_distance() < 0.2);
    assert!(ag.velocity().length() < 0.2, "braked: {}", ag.velocity());
    // It came round the wall's left end (x = -8.5), so it ends facing +x, +z.
    let f = s.world.rotation(a) * Vector3::Z;
    assert!(f.x > 0.4 && f.z > 0.3, "facing {f}");
    // Standing on the floor, not floating above it.
    assert!(p.y.abs() < 0.03, "height {}", p.y);
}

#[test]
fn agents_avoid_each_other_head_on() {
    let mut s = Sim::new();
    let surf = s.level();
    s.bake(surf);
    let a = s.agent("A", v(0.0, 0.0, -8.0));
    let b = s.agent("B", v(0.0, 0.0, 8.0));
    s.step(0.05);
    s.agent_mut(a).set_destination(v(0.0, 0.0, 8.0));
    s.agent_mut(b).set_destination(v(0.0, 0.0, -8.0));
    let mut closest = f32::MAX;
    for _ in 0..(8 * 60) {
        s.step(1.0 / 60.0);
        let d = s.world.position(a).distance(s.world.position(b));
        closest = closest.min(d);
    }
    assert!(closest > 0.8, "agents overlapped: closest {closest}");
    assert!(
        s.world.position(a).distance(v(0.0, 0.0, 8.0)) < 0.5,
        "A got there"
    );
    assert!(
        s.world.position(b).distance(v(0.0, 0.0, -8.0)) < 0.5,
        "B got there"
    );
}

#[test]
fn carving_obstacle_blocks_a_corridor_and_agent_replans() {
    let mut s = Sim::new();
    let surf = s.level();
    // Two walls leave a 3 m gap at x = 0.
    s.solid("WallL", v(-8.5, 1.0, 0.0), v(14.0, 2.0, 0.5));
    s.solid("WallR", v(8.5, 1.0, 0.0), v(14.0, 2.0, 0.5));
    s.bake(surf);
    let (from, to) = (v(0.0, 0.0, -6.0), v(0.0, 0.0, 6.0));
    let f = QueryFilter::new(0);
    assert_eq!(
        s.world
            .resource::<NavMesh>()
            .calculate_path(from, to, &f)
            .status,
        NavMeshPathStatus::PathComplete
    );
    // A crate dropped in the gap carves it shut once still.
    let c = s
        .world
        .create_at("Crate", v(0.0, 0.5, 0.0), Quaternion::IDENTITY);
    s.world.add_component(
        c,
        NavMeshObstacle {
            carve: true,
            size: v(3.0, 1.0, 1.0),
            ..Default::default()
        },
    );
    s.step(1.0);
    let p = s.world.resource::<NavMesh>().calculate_path(from, to, &f);
    assert_eq!(p.status, NavMeshPathStatus::PathPartial, "{:?}", p.corners);
    // Moving it away reopens the gap.
    s.world.set_position(c, v(10.0, 0.5, 10.0));
    s.step(1.0);
    assert_eq!(
        s.world
            .resource::<NavMesh>()
            .calculate_path(from, to, &f)
            .status,
        NavMeshPathStatus::PathComplete
    );
}

#[test]
fn agent_crosses_a_navmesh_link() {
    let mut s = Sim::new();
    s.solid("A", v(0.0, -0.25, -6.0), v(10.0, 0.5, 10.0));
    s.solid("B", v(0.0, -0.25, 6.0), v(10.0, 0.5, 10.0));
    let surf = s.world.create("NavMesh");
    s.world.add_component(
        surf,
        NavMeshSurface {
            use_geometry: NavMeshCollectGeometry::PhysicsColliders,
            ..Default::default()
        },
    );
    s.bake(surf);
    let l = s.world.create("Link");
    s.world.add_component(
        l,
        NavMeshLink {
            start_point: v(0.0, 0.0, -1.6),
            end_point: v(0.0, 0.0, 1.6),
            ..Default::default()
        },
    );
    let a = s.agent("Agent", v(0.0, 0.0, -6.0));
    s.step(0.05);
    s.agent_mut(a).set_destination(v(0.0, 0.0, 6.0));
    let mut was_on_link = false;
    for _ in 0..(8 * 60) {
        s.step(1.0 / 60.0);
        was_on_link |= s.agent_mut(a).is_on_off_mesh_link();
    }
    assert!(was_on_link);
    let p = s.world.position(a);
    assert!(p.distance(v(0.0, 0.0, 6.0)) < 0.3, "ended at {p}");
}

#[test]
fn heights_follow_the_ground_on_large_polygons_over_hills() {
    // A gentle 3 m mound in the middle of open ground. With default (42 m) tiles the region
    // around it becomes a few big polygons whose corners are all on the flat ground, so the
    // polygons alone miss the mound by metres; heights must come from the height grid.
    let h = |x: f32, z: f32| 3.0 * (-(x * x + z * z) / 120.0).exp();
    let n = 121;
    let size = 60.0;
    let mut verts = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let x = -size / 2.0 + i as f32 / (n - 1) as f32 * size;
            let z = -size / 2.0 + j as f32 / (n - 1) as f32 * size;
            verts.push(v(x, h(x, z), z));
        }
    }
    let mut tris = Vec::new();
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = (j * n + i) as u32;
            let b = a + n as u32;
            tris.push([a, b, b + 1]);
            tris.push([a, b + 1, a + 1]);
        }
    }
    let mut src = BuildSource::default();
    src.add_triangles(&verts, &tris, Some(0));
    let nav = bake(&src, &[], &BuildSettings::default());
    let mut worst = 0.0f32;
    for k in 0..400 {
        let x = ((k * 37) % 41) as f32 - 20.0;
        let z = ((k * 53) % 37) as f32 - 18.0;
        let hit = nav
            .sample_position(v(x, h(x, z) + 0.5, z), 1.0, &filter())
            .unwrap_or_else(|| panic!("no NavMesh found standing on the ground at ({x}, {z})"));
        worst = worst.max((hit.position.y - h(x, z)).abs());
    }
    assert!(worst < 0.1, "worst height error {worst} m");
}
