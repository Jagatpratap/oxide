//! Oxide navigation, matching Unity's AI Navigation package: NavMeshSurface baking (Recast-style
//! voxelization), NavMeshAgent with path following and local avoidance, NavMeshObstacle
//! (with carving), NavMeshLink, NavMeshModifier / NavMeshModifierVolume, agent types and areas
//! with costs, and the `NavMesh` queries (`CalculatePath`, `SamplePosition`, `Raycast`,
//! `FindClosestEdge`).
//!
//! Reference: <https://docs.unity3d.com/Packages/com.unity.ai.navigation@2.0/manual/index.html>

mod agent;
pub mod bake;
pub mod build;
pub mod collect;
pub mod components;
pub mod data;
pub mod navmesh;
pub mod query;
pub mod settings;
pub mod system;

pub use bake::{BakeStatus, NavMeshBake};
pub use build::{BuildSettings, BuildSource};
pub use components::*;
pub use data::NavMeshData;
pub use navmesh::{ALL_AREAS, NavMesh, NavMeshDataInstance, OffMeshLink, PolyRef};
pub use query::{Corner, NavMeshHit, NavMeshPath, NavMeshPathStatus, QueryFilter};
pub use settings::{JUMP, NOT_WALKABLE, NavArea, NavMeshAgentType, NavigationSettings, WALKABLE};
pub use system::{NavigationRuntime, NavigationSystem};

use oxide_scene::{System, World};

pub(crate) fn build_dx(d: usize) -> i32 {
    [-1, 0, 1, 0][d & 3]
}
pub(crate) fn build_dz(d: usize) -> i32 {
    [0, 1, 0, -1][d & 3]
}

/// Installs the NavMesh resources and components; returns the system to run.
pub fn install(world: &mut World) -> Box<dyn System> {
    world.resources.insert(NavMesh::new());
    world.resources.insert(NavigationRuntime::default());
    if !world.resources.contains::<NavigationSettings>() {
        world.resources.insert(NavigationSettings::default());
    }
    if let Some(a) = world.resources.get_mut::<oxide_assets::AssetServer>() {
        a.types.register(
            oxide_assets::AssetTypeInfo::new("NavMeshData", &["navmesh"]).color(90, 170, 230),
        );
    }
    let r = &mut world.registry;
    r.register::<NavMeshSurface>("Navigation/NavMesh Surface");
    r.register::<NavMeshModifier>("Navigation/NavMesh Modifier");
    r.register::<NavMeshModifierVolume>("Navigation/NavMesh Modifier Volume");
    r.register::<NavMeshAgent>("Navigation/NavMesh Agent");
    r.register::<NavMeshObstacle>("Navigation/NavMesh Obstacle");
    r.register::<NavMeshLink>("Navigation/NavMesh Link");
    Box::new(NavigationSystem)
}

/// Unity's static `NavMesh` queries for scripts: `world.calculate_path(...)` and friends.
pub trait NavMeshExt {
    /// Unity `NavMesh.CalculatePath` (Humanoid agent type unless `filter` says otherwise).
    fn calculate_path(
        &self,
        source: oxide_math::Vector3,
        target: oxide_math::Vector3,
        filter: &QueryFilter,
    ) -> NavMeshPath;
    /// Unity `NavMesh.SamplePosition`.
    fn sample_position(
        &self,
        p: oxide_math::Vector3,
        max_distance: f32,
        filter: &QueryFilter,
    ) -> Option<NavMeshHit>;
    /// Unity `NavMesh.Raycast` (`hit.hit` is true when blocked).
    fn navmesh_raycast(
        &self,
        source: oxide_math::Vector3,
        target: oxide_math::Vector3,
        filter: &QueryFilter,
    ) -> NavMeshHit;
    /// Unity `NavMesh.FindClosestEdge`.
    fn find_closest_edge(&self, p: oxide_math::Vector3, filter: &QueryFilter)
    -> Option<NavMeshHit>;
    /// Unity `NavMesh.GetAreaFromName`.
    fn navmesh_area_from_name(&self, name: &str) -> i32;
}

impl NavMeshExt for World {
    fn calculate_path(
        &self,
        source: oxide_math::Vector3,
        target: oxide_math::Vector3,
        filter: &QueryFilter,
    ) -> NavMeshPath {
        self.resources
            .get::<NavMesh>()
            .map(|n| n.calculate_path(source, target, filter))
            .unwrap_or_default()
    }
    fn sample_position(
        &self,
        p: oxide_math::Vector3,
        max_distance: f32,
        filter: &QueryFilter,
    ) -> Option<NavMeshHit> {
        self.resources
            .get::<NavMesh>()?
            .sample_position(p, max_distance, filter)
    }
    fn navmesh_raycast(
        &self,
        source: oxide_math::Vector3,
        target: oxide_math::Vector3,
        filter: &QueryFilter,
    ) -> NavMeshHit {
        self.resources
            .get::<NavMesh>()
            .map(|n| n.raycast(source, target, filter))
            .unwrap_or_default()
    }
    fn find_closest_edge(
        &self,
        p: oxide_math::Vector3,
        filter: &QueryFilter,
    ) -> Option<NavMeshHit> {
        self.resources
            .get::<NavMesh>()?
            .find_closest_edge(p, filter)
    }
    fn navmesh_area_from_name(&self, name: &str) -> i32 {
        self.resources
            .get::<NavigationSettings>()
            .map_or(-1, |s| s.area_from_name(name))
    }
}

#[cfg(test)]
mod tests;
