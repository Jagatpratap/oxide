//! Unity's AI Navigation components: NavMeshSurface, NavMeshModifier, NavMeshModifierVolume,
//! NavMeshAgent, NavMeshObstacle and NavMeshLink.
//! <https://docs.unity3d.com/Packages/com.unity.ai.navigation@2.0/manual/index.html>

use crate::data::NavMeshData;
use crate::navmesh::PolyRef;
use crate::query::{Corner, NavMeshPath, NavMeshPathStatus, PathNode};
use oxide_assets::Handle;
use oxide_core::Reflect;
use oxide_math::Vector3;
use oxide_scene::Component;
use std::sync::Arc;

/// Which geometry a NavMeshSurface bakes from (Unity `NavMeshCollectGeometry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum NavMeshCollectGeometry {
    #[default]
    RenderMeshes,
    PhysicsColliders,
}

/// Which objects a NavMeshSurface bakes (Unity `CollectObjects`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum CollectObjects {
    #[default]
    All,
    /// Objects overlapping the surface's box (`center`, `size`).
    Volume,
    /// The surface's object and its children.
    Children,
    /// Only objects with a NavMeshModifier.
    MarkedWithModifier,
}

/// Unity `NavMeshSurface`: bakes and owns a NavMesh for one agent type.
#[derive(Clone, Debug, Reflect)]
pub struct NavMeshSurface {
    /// Agent type (Navigation window → Agents); 0 = Humanoid.
    pub agent_type_id: i32,
    pub default_area: i32,
    pub collect_objects: CollectObjects,
    /// Box for Collect Objects = Volume (local space).
    pub size: Vector3,
    pub center: Vector3,
    /// Layers included in the bake.
    pub layer_mask: u32,
    pub use_geometry: NavMeshCollectGeometry,
    pub override_voxel_size: bool,
    pub voxel_size: f32,
    pub override_tile_size: bool,
    pub tile_size: u32,
    /// Islands smaller than this (m²) are dropped.
    pub min_region_area: f32,
    /// The baked NavMesh (`.navmesh` file).
    pub navmesh_data: Option<Handle<NavMeshData>>,
    #[reflect(skip)]
    #[doc(hidden)]
    pub runtime: SurfaceRuntime,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct SurfaceRuntime {
    /// Data built at runtime (not saved), used instead of `navmesh_data`.
    pub data: Option<Arc<NavMeshData>>,
}

impl Default for NavMeshSurface {
    fn default() -> Self {
        Self {
            agent_type_id: 0,
            default_area: 0,
            collect_objects: CollectObjects::All,
            size: Vector3::splat(10.0),
            center: Vector3::new(0.0, 2.0, 0.0),
            layer_mask: u32::MAX,
            use_geometry: NavMeshCollectGeometry::RenderMeshes,
            override_voxel_size: false,
            voxel_size: 0.1666667,
            override_tile_size: false,
            tile_size: 256,
            min_region_area: 2.0,
            navmesh_data: None,
            runtime: SurfaceRuntime::default(),
        }
    }
}

impl Component for NavMeshSurface {}

/// Unity `NavMeshModifier`: changes the area of (or excludes) an object and, optionally, its
/// children when baking.
#[derive(Clone, Debug, Reflect)]
pub struct NavMeshModifier {
    pub override_area: bool,
    pub area: i32,
    pub ignore_from_build: bool,
    pub apply_to_children: bool,
}

impl Default for NavMeshModifier {
    fn default() -> Self {
        Self {
            override_area: false,
            area: 0,
            ignore_from_build: false,
            apply_to_children: true,
        }
    }
}

impl Component for NavMeshModifier {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity `NavMeshModifierVolume`: sets the area inside a box when baking.
#[derive(Clone, Debug, Reflect)]
pub struct NavMeshModifierVolume {
    pub size: Vector3,
    pub center: Vector3,
    pub area: i32,
}

impl Default for NavMeshModifierVolume {
    fn default() -> Self {
        Self {
            size: Vector3::new(4.0, 3.0, 4.0),
            center: Vector3::new(0.0, 1.0, 0.0),
            area: crate::settings::NOT_WALKABLE,
        }
    }
}

impl Component for NavMeshModifierVolume {}

/// Unity `ObstacleAvoidanceType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum ObstacleAvoidanceType {
    NoObstacleAvoidance,
    LowQualityObstacleAvoidance,
    MedQualityObstacleAvoidance,
    GoodQualityObstacleAvoidance,
    #[default]
    HighQualityObstacleAvoidance,
}

/// Unity `NavMeshAgent`: moves its object along NavMesh paths, avoiding other agents.
#[derive(Clone, Debug, Reflect)]
pub struct NavMeshAgent {
    pub agent_type_id: i32,
    /// Height of the object's pivot above the NavMesh.
    pub base_offset: f32,
    /// Maximum speed (m/s).
    pub speed: f32,
    /// Maximum turning speed (degrees/s).
    pub angular_speed: f32,
    /// Maximum acceleration (m/s²).
    pub acceleration: f32,
    /// Stop this close to the destination.
    pub stopping_distance: f32,
    /// Slow down before reaching the destination.
    pub auto_braking: bool,
    /// Avoidance radius.
    pub radius: f32,
    /// Avoidance height.
    pub height: f32,
    pub obstacle_avoidance_type: ObstacleAvoidanceType,
    /// 0 (most important) to 99; agents ignore agents of lower importance (higher number).
    #[reflect(range(0, 99))]
    pub avoidance_priority: i32,
    pub auto_traverse_off_mesh_link: bool,
    pub auto_repath: bool,
    /// Areas this agent may walk on.
    pub area_mask: u32,
    /// Write the simulated position to the Transform.
    pub update_position: bool,
    /// Turn the Transform towards the movement direction.
    pub update_rotation: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub state: AgentState,
}

impl Default for NavMeshAgent {
    fn default() -> Self {
        Self {
            agent_type_id: 0,
            base_offset: 0.0,
            speed: 3.5,
            angular_speed: 120.0,
            acceleration: 8.0,
            stopping_distance: 0.0,
            auto_braking: true,
            radius: 0.5,
            height: 2.0,
            obstacle_avoidance_type: ObstacleAvoidanceType::HighQualityObstacleAvoidance,
            avoidance_priority: 50,
            auto_traverse_off_mesh_link: true,
            auto_repath: true,
            area_mask: crate::ALL_AREAS,
            update_position: true,
            update_rotation: true,
            state: AgentState::default(),
        }
    }
}

impl Component for NavMeshAgent {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity `OffMeshLinkData`: the link an agent is on or about to cross.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffMeshLinkData {
    pub link_id: u32,
    pub start_pos: Vector3,
    pub end_pos: Vector3,
    pub activated: bool,
    pub valid: bool,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct AgentState {
    /// Position on the NavMesh (the Transform is this plus `base_offset`).
    pub pos: Vector3,
    pub poly: Option<PolyRef>,
    pub on_navmesh: bool,
    pub velocity: Vector3,
    pub desired_velocity: Vector3,
    pub destination: Option<Vector3>,
    /// The destination mapped onto the NavMesh.
    pub target: Vector3,
    pub target_poly: Option<PolyRef>,
    pub corridor: Vec<PathNode>,
    pub corners: Vec<Corner>,
    pub status: NavMeshPathStatus,
    pub path_pending: bool,
    pub repath: bool,
    pub stopped: bool,
    pub remaining: f32,
    pub link: Option<LinkTraversal>,
    pub warp: Option<Vector3>,
    pub moves: Vector3,
    pub set_path: Option<NavMeshPath>,
    pub area_costs: Option<[f32; 32]>,
    /// Transform position written last frame, to notice scripts moving the object.
    pub written: Option<Vector3>,
    pub navmesh_version: u64,
}

#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct LinkTraversal {
    pub id: u32,
    pub forward: bool,
    pub start: Vector3,
    pub end: Vector3,
    pub t: f32,
    /// Moving across (auto traverse) or waiting for `complete_off_mesh_link`.
    pub moving: bool,
}

impl NavMeshAgent {
    /// Unity `SetDestination`: plans a path to `target` (ready next frame: `path_pending`).
    pub fn set_destination(&mut self, target: Vector3) -> bool {
        self.state.destination = Some(target);
        self.state.path_pending = true;
        self.state.repath = true;
        true
    }
    /// Unity `destination` (the requested point, or the agent's position without a path).
    pub fn destination(&self) -> Vector3 {
        self.state.destination.unwrap_or(self.state.pos)
    }
    /// Unity `ResetPath`.
    pub fn reset_path(&mut self) {
        self.state.destination = None;
        self.state.corridor.clear();
        self.state.corners.clear();
        self.state.path_pending = false;
        self.state.status = NavMeshPathStatus::PathInvalid;
        self.state.remaining = 0.0;
    }
    /// Unity `isStopped`.
    pub fn is_stopped(&self) -> bool {
        self.state.stopped
    }
    pub fn set_is_stopped(&mut self, stopped: bool) {
        self.state.stopped = stopped;
    }
    /// Unity `velocity` (m/s, world space).
    pub fn velocity(&self) -> Vector3 {
        self.state.velocity
    }
    /// Unity `velocity` setter: overrides the current velocity (steering continues from it).
    pub fn set_velocity(&mut self, v: Vector3) {
        self.state.velocity = v;
    }
    /// Unity `desiredVelocity`: where steering wants to go, before avoidance.
    pub fn desired_velocity(&self) -> Vector3 {
        self.state.desired_velocity
    }
    /// Unity `remainingDistance` along the path.
    pub fn remaining_distance(&self) -> f32 {
        self.state.remaining
    }
    /// Unity `hasPath`.
    pub fn has_path(&self) -> bool {
        !self.state.corridor.is_empty() && self.state.destination.is_some()
    }
    /// Unity `pathPending`.
    pub fn path_pending(&self) -> bool {
        self.state.path_pending
    }
    /// Unity `pathStatus`.
    pub fn path_status(&self) -> NavMeshPathStatus {
        self.state.status
    }
    /// Unity `path.corners`: the corners from the agent to the destination.
    pub fn path_corners(&self) -> Vec<Vector3> {
        self.state.corners.iter().map(|c| c.pos).collect()
    }
    /// Unity `steeringTarget`: the next corner.
    pub fn steering_target(&self) -> Vector3 {
        self.state.corners.get(1).map_or(self.state.pos, |c| c.pos)
    }
    /// Unity `nextPosition` (the simulated position on the NavMesh).
    pub fn next_position(&self) -> Vector3 {
        self.state.pos
    }
    /// Unity `nextPosition` setter: moves the simulated position (constrained to the NavMesh
    /// on the next update). Used with `update_position = false` and root motion.
    pub fn set_next_position(&mut self, p: Vector3) {
        self.state.moves += p - self.state.pos;
    }
    /// Unity `Warp`: teleports to `p` (mapped onto the NavMesh next update).
    pub fn warp(&mut self, p: Vector3) -> bool {
        self.state.warp = Some(p);
        true
    }
    /// Unity `Move`: moves by `offset`, constrained to the NavMesh.
    pub fn move_by(&mut self, offset: Vector3) {
        self.state.moves += offset;
    }
    /// Unity `isOnNavMesh`.
    pub fn is_on_nav_mesh(&self) -> bool {
        self.state.on_navmesh
    }
    /// Unity `isOnOffMeshLink`.
    pub fn is_on_off_mesh_link(&self) -> bool {
        self.state.link.is_some()
    }
    /// Unity `currentOffMeshLinkData`.
    pub fn current_off_mesh_link_data(&self) -> Option<OffMeshLinkData> {
        self.state.link.map(|l| OffMeshLinkData {
            link_id: l.id,
            start_pos: l.start,
            end_pos: l.end,
            activated: true,
            valid: true,
        })
    }
    /// Unity `CompleteOffMeshLink`: finishes a link crossed manually (agent moves to its end).
    pub fn complete_off_mesh_link(&mut self) {
        if let Some(l) = &mut self.state.link {
            l.t = 1.0;
            l.moving = true;
        }
    }
    /// Unity `SetPath`.
    pub fn set_path(&mut self, path: NavMeshPath) -> bool {
        if path.status == NavMeshPathStatus::PathInvalid || path.nodes.is_empty() {
            return false;
        }
        self.state.destination = path.corners.last().copied();
        self.state.set_path = Some(path);
        self.state.path_pending = true;
        true
    }
    /// Unity `SetAreaCost` (for this agent only).
    pub fn set_area_cost(&mut self, area: usize, cost: f32) {
        if area < 32 {
            let mut c = self.state.area_costs.unwrap_or([f32::NAN; 32]);
            c[area] = cost.max(1.0);
            self.state.area_costs = Some(c);
            self.state.repath = true;
        }
    }
}

/// Unity `NavMeshObstacleShape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum NavMeshObstacleShape {
    Capsule,
    #[default]
    Box,
}

/// Unity `NavMeshObstacle`: agents avoid it; with `carve` it cuts a hole in the NavMesh.
#[derive(Clone, Debug, Reflect)]
pub struct NavMeshObstacle {
    pub shape: NavMeshObstacleShape,
    pub center: Vector3,
    /// Box size.
    pub size: Vector3,
    /// Capsule radius and height.
    pub radius: f32,
    pub height: f32,
    pub carve: bool,
    /// Moving further than this re-carves (m).
    pub move_threshold: f32,
    /// Seconds still before it counts as stationary.
    pub time_to_stationary: f32,
    /// Only carve while stationary (moving obstacles are avoided instead).
    pub carve_only_stationary: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub runtime: ObstacleRuntime,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct ObstacleRuntime {
    pub velocity: Vector3,
    /// Where it last carved (position, rotation, size key) if carving.
    pub carved_at: Option<(Vector3, oxide_math::Quaternion)>,
}

impl Default for NavMeshObstacle {
    fn default() -> Self {
        Self {
            shape: NavMeshObstacleShape::Box,
            center: Vector3::ZERO,
            size: Vector3::ONE,
            radius: 0.5,
            height: 2.0,
            carve: false,
            move_threshold: 0.1,
            time_to_stationary: 0.5,
            carve_only_stationary: true,
            runtime: ObstacleRuntime::default(),
        }
    }
}

impl Component for NavMeshObstacle {}

impl NavMeshObstacle {
    /// Unity `velocity` (estimated from movement).
    pub fn velocity(&self) -> Vector3 {
        self.runtime.velocity
    }
}

/// Unity `NavMeshLink`: lets agents cross between two points (gaps, jumps, ladders).
#[derive(Clone, Debug, Reflect)]
pub struct NavMeshLink {
    pub agent_type_id: i32,
    /// Local-space start and end.
    pub start_point: Vector3,
    pub end_point: Vector3,
    /// 0 = a single point; otherwise a link this wide.
    pub width: f32,
    /// Multiplies the area cost when ≥ 0.
    pub cost_modifier: f32,
    pub bidirectional: bool,
    /// Follow the object when it moves.
    pub auto_update_position: bool,
    pub area: i32,
    pub activated: bool,
}

impl Default for NavMeshLink {
    fn default() -> Self {
        Self {
            agent_type_id: 0,
            start_point: Vector3::new(0.0, 0.0, -2.5),
            end_point: Vector3::new(0.0, 0.0, 2.5),
            width: 0.0,
            cost_modifier: -1.0,
            bidirectional: true,
            auto_update_position: false,
            area: 0,
            activated: true,
        }
    }
}

impl Component for NavMeshLink {}
