//! Project-wide navigation settings (Unity's Navigation window → Agents and Areas tabs,
//! `ProjectSettings/NavMeshAreas.asset`).

use crate::build::BuildSettings;
use serde::{Deserialize, Serialize};

/// Unity `NavMeshBuildSettings` as an agent type in the Agents tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavMeshAgentType {
    /// Unity `agentTypeID` (0 = Humanoid).
    pub id: i32,
    pub name: String,
    pub radius: f32,
    pub height: f32,
    pub step_height: f32,
    /// Degrees.
    pub max_slope: f32,
}

impl Default for NavMeshAgentType {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Humanoid".into(),
            radius: 0.5,
            height: 2.0,
            step_height: 0.75,
            max_slope: 45.0,
        }
    }
}

/// A navigation area (Areas tab): name and pathfinding cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavArea {
    pub name: String,
    pub cost: f32,
}

impl Default for NavArea {
    fn default() -> Self {
        Self {
            name: String::new(),
            cost: 1.0,
        }
    }
}

/// Unity's built-in area indices.
pub const WALKABLE: i32 = 0;
pub const NOT_WALKABLE: i32 = 1;
pub const JUMP: i32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavigationSettings {
    pub agents: Vec<NavMeshAgentType>,
    /// 32 areas; 0 Walkable, 1 Not Walkable, 2 Jump are built in.
    pub areas: Vec<NavArea>,
}

impl Default for NavigationSettings {
    fn default() -> Self {
        let mut areas = vec![NavArea::default(); 32];
        areas[0].name = "Walkable".into();
        areas[1].name = "Not Walkable".into();
        areas[2] = NavArea {
            name: "Jump".into(),
            cost: 2.0,
        };
        Self {
            agents: vec![NavMeshAgentType::default()],
            areas,
        }
    }
}

impl NavigationSettings {
    /// Unity `NavMesh.GetSettingsByID`.
    pub fn agent(&self, id: i32) -> Option<&NavMeshAgentType> {
        self.agents.iter().find(|a| a.id == id)
    }

    /// Unity `NavMesh.GetAreaFromName` (-1 if none).
    pub fn area_from_name(&self, name: &str) -> i32 {
        self.areas
            .iter()
            .position(|a| !a.name.is_empty() && a.name.eq_ignore_ascii_case(name))
            .map_or(-1, |i| i as i32)
    }

    /// Area mask with the named areas set (Unity `NavMesh.GetAreaMask`-style helper).
    pub fn area_mask(&self, names: &[&str]) -> u32 {
        names
            .iter()
            .map(|n| self.area_from_name(n))
            .filter(|&i| i >= 0)
            .fold(0, |m, i| m | 1 << i)
    }

    pub fn costs(&self) -> [f32; 32] {
        let mut c = [1.0; 32];
        for (i, a) in self.areas.iter().take(32).enumerate() {
            c[i] = a.cost.max(1.0);
        }
        c
    }

    /// Build settings for an agent type (unknown ids fall back to Humanoid).
    pub fn build_settings(&self, agent_type: i32) -> BuildSettings {
        let a = self.agent(agent_type).cloned().unwrap_or_default();
        BuildSettings {
            agent_radius: a.radius.max(0.05),
            agent_height: a.height.max(0.1),
            agent_climb: a.step_height.max(0.0),
            agent_slope: a.max_slope.clamp(0.0, 60.0),
            voxel_size: a.radius.max(0.05) / 3.0,
            ..Default::default()
        }
    }

    /// Adds an agent type with a fresh id (Agents tab "+").
    pub fn add_agent(&mut self, name: &str) -> i32 {
        let id = self.agents.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        self.agents.push(NavMeshAgentType {
            id,
            name: name.into(),
            ..Default::default()
        });
        id
    }
}
