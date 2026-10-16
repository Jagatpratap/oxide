//! Project Settings (Unity's `ProjectSettings/` folder): Tags & Layers, Time, Physics,
//! Quality, Input Manager and Player, one YAML file each. Loaded and applied when the engine
//! starts (editor and built players alike); the editor's Project Settings window saves them.
//! Reference: <https://docs.unity3d.com/Manual/comp-ManagerGroup.html>

use oxide_core::Time;
use oxide_input::{Input, InputAxis, InputButton};
use oxide_physics::Physics;
use oxide_scene::{Application, QualityLevel, QualitySettings, World};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::Path;

/// Tags & Layers. <https://docs.unity3d.com/Manual/class-TagManager.html>
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TagsAndLayers {
    /// Custom tags (after Unity's built-in ones).
    pub tags: Vec<String>,
    /// Names of layers 0..31 (empty = unused).
    pub layers: Vec<String>,
}

impl Default for TagsAndLayers {
    fn default() -> Self {
        Self {
            tags: Vec::new(),
            layers: oxide_scene::LayerNames::default().0.to_vec(),
        }
    }
}

/// Time. <https://docs.unity3d.com/Manual/class-TimeManager.html>
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeSettings {
    pub fixed_timestep: f32,
    pub maximum_allowed_timestep: f32,
    pub time_scale: f32,
}

impl Default for TimeSettings {
    fn default() -> Self {
        let t = Time::new();
        Self {
            fixed_timestep: t.fixed_delta_time(),
            maximum_allowed_timestep: t.maximum_delta_time(),
            time_scale: t.time_scale(),
        }
    }
}

/// Physics. <https://docs.unity3d.com/Manual/class-PhysicsManager.html>
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhysicsSettings {
    pub gravity: [f32; 3],
    pub queries_hit_triggers: bool,
    /// Bit j of row i: layers i and j collide.
    pub layer_collision_matrix: Vec<u32>,
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            gravity: [0.0, -9.81, 0.0],
            queries_hit_triggers: true,
            layer_collision_matrix: vec![u32::MAX; 32],
        }
    }
}

impl PhysicsSettings {
    pub fn collides(&self, a: usize, b: usize) -> bool {
        self.layer_collision_matrix
            .get(a)
            .is_none_or(|row| row & (1 << b) != 0)
    }

    /// Sets both halves of the symmetric matrix.
    pub fn set_collides(&mut self, a: usize, b: usize, on: bool) {
        self.layer_collision_matrix.resize(32, u32::MAX);
        for (i, j) in [(a, b), (b, a)] {
            if on {
                self.layer_collision_matrix[i] |= 1 << j;
            } else {
                self.layer_collision_matrix[i] &= !(1 << j);
            }
        }
    }
}

/// Quality. <https://docs.unity3d.com/Manual/class-QualitySettings.html>
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QualityFile {
    pub current: usize,
    pub levels: Vec<QualityLevel>,
}

impl Default for QualityFile {
    fn default() -> Self {
        let q = QualitySettings::default();
        Self {
            current: q.quality_level(),
            levels: q.levels,
        }
    }
}

/// Input Manager. <https://docs.unity3d.com/Manual/class-InputManager.html>
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputSettings {
    pub axes: Vec<InputAxis>,
    pub buttons: Vec<InputButton>,
}

impl Default for InputSettings {
    fn default() -> Self {
        let i = Input::new();
        Self {
            axes: i.axes().to_vec(),
            buttons: i.buttons().to_vec(),
        }
    }
}

/// Player (the parts the running game reads; window size and scenes are in Build Settings).
/// <https://docs.unity3d.com/Manual/class-PlayerSettings.html>
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerSettings {
    pub company_name: String,
    pub product_name: String,
    pub version: String,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            company_name: "DefaultCompany".into(),
            product_name: String::new(),
            version: "0.1.0".into(),
        }
    }
}

/// Every Project Settings page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub tags_and_layers: TagsAndLayers,
    pub time: TimeSettings,
    pub physics: PhysicsSettings,
    pub quality: QualityFile,
    pub input: InputSettings,
    pub player: PlayerSettings,
}

/// The pages and their files, for MCP and error messages.
pub const SECTIONS: [(&str, &str); 6] = [
    ("tags_and_layers", "TagManager.yaml"),
    ("time", "TimeManager.yaml"),
    ("physics", "DynamicsManager.yaml"),
    ("quality", "QualitySettings.yaml"),
    ("input", "InputManager.yaml"),
    ("player", "PlayerSettings.yaml"),
];

fn read<T: DeserializeOwned + Default>(dir: &Path, file: &str, warnings: &mut Vec<String>) -> T {
    let p = dir.join(file);
    match std::fs::read_to_string(&p) {
        Ok(text) => serde_yaml_ng::from_str(&text).unwrap_or_else(|e| {
            warnings.push(format!("{}: {e}", p.display()));
            T::default()
        }),
        Err(_) => T::default(),
    }
}

impl ProjectSettings {
    /// Reads `<project>/ProjectSettings/`. Missing files give defaults; broken ones give
    /// defaults plus a warning.
    pub fn load(project: &Path) -> (Self, Vec<String>) {
        let dir = project.join("ProjectSettings");
        let mut w = Vec::new();
        let s = Self {
            tags_and_layers: read(&dir, SECTIONS[0].1, &mut w),
            time: read(&dir, SECTIONS[1].1, &mut w),
            physics: read(&dir, SECTIONS[2].1, &mut w),
            quality: read(&dir, SECTIONS[3].1, &mut w),
            input: read(&dir, SECTIONS[4].1, &mut w),
            player: read(&dir, SECTIONS[5].1, &mut w),
        };
        (s, w)
    }

    /// Writes every page (files whose content is unchanged are left alone).
    pub fn save(&self, project: &Path) -> Result<(), String> {
        let dir = project.join("ProjectSettings");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let texts = [
            serde_yaml_ng::to_string(&self.tags_and_layers),
            serde_yaml_ng::to_string(&self.time),
            serde_yaml_ng::to_string(&self.physics),
            serde_yaml_ng::to_string(&self.quality),
            serde_yaml_ng::to_string(&self.input),
            serde_yaml_ng::to_string(&self.player),
        ];
        for ((_, file), text) in SECTIONS.iter().zip(texts) {
            let text = text.map_err(|e| e.to_string())?;
            let p = dir.join(file);
            if std::fs::read_to_string(&p).ok().as_deref() != Some(text.as_str()) {
                std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
            }
        }
        Ok(())
    }

    /// Reads the settings the World is running with.
    pub fn capture(world: &World) -> Self {
        let time = world.resource::<Time>();
        let input = world.resource::<Input>();
        let app = world.resource::<Application>();
        let q = world
            .try_resource::<QualitySettings>()
            .cloned()
            .unwrap_or_default();
        Self {
            tags_and_layers: TagsAndLayers {
                tags: world.tags.clone(),
                layers: world.layers.0.to_vec(),
            },
            time: TimeSettings {
                fixed_timestep: time.fixed_delta_time(),
                maximum_allowed_timestep: time.maximum_delta_time(),
                time_scale: time.time_scale(),
            },
            physics: world
                .try_resource::<Physics>()
                .map_or_else(PhysicsSettings::default, |p| PhysicsSettings {
                    gravity: p.gravity().to_array(),
                    queries_hit_triggers: p.queries_hit_triggers,
                    layer_collision_matrix: p.layer_collision_matrix.to_vec(),
                }),
            quality: QualityFile {
                current: q.quality_level(),
                levels: q.levels,
            },
            input: InputSettings {
                axes: input.axes().to_vec(),
                buttons: input.buttons().to_vec(),
            },
            player: PlayerSettings {
                company_name: app.company_name.clone(),
                product_name: app.product_name.clone(),
                version: app.version.clone(),
            },
        }
    }

    /// Makes the World run with these settings.
    pub fn apply(&self, world: &mut World) {
        let tl = &self.tags_and_layers;
        world.tags = tl
            .tags
            .iter()
            .filter(|t| !t.is_empty() && !oxide_scene::BUILTIN_TAGS.contains(&t.as_str()))
            .cloned()
            .collect();
        for (i, name) in world.layers.0.iter_mut().enumerate() {
            *name = tl.layers.get(i).cloned().unwrap_or_default();
        }
        {
            let t = world.resource_mut::<Time>();
            t.set_fixed_delta_time(self.time.fixed_timestep.max(0.0001));
            t.set_maximum_delta_time(self.time.maximum_allowed_timestep.max(0.0001));
            t.set_time_scale(self.time.time_scale.max(0.0));
        }
        if let Some(p) = world.try_resource_mut::<Physics>() {
            p.set_gravity(oxide_math::Vector3::from_array(self.physics.gravity));
            p.queries_hit_triggers = self.physics.queries_hit_triggers;
            for (i, row) in p.layer_collision_matrix.iter_mut().enumerate() {
                *row = self
                    .physics
                    .layer_collision_matrix
                    .get(i)
                    .copied()
                    .unwrap_or(u32::MAX);
            }
        }
        world.resources.insert(QualitySettings::new(
            self.quality.levels.clone(),
            self.quality.current,
        ));
        world
            .resource_mut::<Input>()
            .set_mappings(self.input.axes.clone(), self.input.buttons.clone());
        let app = world.resource_mut::<Application>();
        app.company_name = self.player.company_name.clone();
        app.product_name = self.player.product_name.clone();
        app.version = self.player.version.clone();
    }

    /// One page as JSON (MCP).
    pub fn section_json(&self, name: &str) -> Option<serde_json::Value> {
        Some(
            match name {
                "tags_and_layers" => serde_json::to_value(&self.tags_and_layers),
                "time" => serde_json::to_value(&self.time),
                "physics" => serde_json::to_value(&self.physics),
                "quality" => serde_json::to_value(&self.quality),
                "input" => serde_json::to_value(&self.input),
                "player" => serde_json::to_value(&self.player),
                _ => return None,
            }
            .unwrap_or_default(),
        )
    }

    /// Merges `patch` into one page: keys present in `patch` replace those in the page.
    pub fn patch_section(&mut self, name: &str, patch: &serde_json::Value) -> Result<(), String> {
        let mut current = self
            .section_json(name)
            .ok_or_else(|| format!("unknown section `{name}`"))?;
        let (Some(cur), Some(p)) = (current.as_object_mut(), patch.as_object()) else {
            return Err("`values` must be an object".into());
        };
        for (k, v) in p {
            if !cur.contains_key(k) {
                return Err(format!("`{name}` has no setting `{k}`"));
            }
            cur.insert(k.clone(), v.clone());
        }
        fn parse<T: DeserializeOwned>(v: serde_json::Value) -> Result<T, String> {
            serde_json::from_value(v).map_err(|e| e.to_string())
        }
        match name {
            "tags_and_layers" => self.tags_and_layers = parse(current)?,
            "time" => self.time = parse(current)?,
            "physics" => self.physics = parse(current)?,
            "quality" => self.quality = parse(current)?,
            "input" => self.input = parse(current)?,
            _ => self.player = parse(current)?,
        }
        Ok(())
    }
}

/// Loads `ProjectSettings/` from the AssetServer's project folder and applies it.
pub fn load_and_apply(world: &mut World) {
    let root = world.resource::<oxide_assets::AssetServer>().full_path("");
    let (settings, warnings) = ProjectSettings::load(&root);
    for w in warnings {
        oxide_core::debug_log_warning!("Project Settings: {w}");
    }
    settings.apply(world);
}

/// Saves what the World is running with to `ProjectSettings/`.
pub fn save(world: &World) -> Result<(), String> {
    let root = world.resource::<oxide_assets::AssetServer>().full_path("");
    ProjectSettings::capture(world).save(&root)
}

/// MCP: read and change Project Settings.
pub fn register(s: &mut oxide_mcp::McpServer, bus: &std::sync::Arc<crate::bus::Bus>) {
    use crate::mcp_tools::{schema, tool};
    use oxide_mcp::{ToolDef, ToolResult, json};
    tool(
        s,
        bus,
        ToolDef::new(
            "project_settings",
            "Project Settings",
            "Unity Project Settings, saved in ProjectSettings/*.yaml. Without `section`: every page. With `section` \
             (tags_and_layers, time, physics, quality, input, player): that page. With `values`: merges those keys \
             into the page (e.g. section=time, values={\"fixed_timestep\": 0.01}; section=quality, \
             values={\"current\": 2}), applies it to the running engine and saves it. Physics \
             `layer_collision_matrix` is 32 rows of bits; input axes/buttons are replaced as whole lists.",
            schema(
                json!({
                    "section": { "type": "string", "enum": SECTIONS.iter().map(|(n, _)| *n).collect::<Vec<_>>() },
                    "values": { "type": "object" }
                }),
                &[],
            ),
        ),
        |args, e| {
            let mut settings = ProjectSettings::capture(&e.world);
            let section = args.get("section").and_then(serde_json::Value::as_str);
            if let Some(values) = args.get("values") {
                let Some(name) = section else {
                    return ToolResult::error("`values` needs a `section`");
                };
                if let Err(err) = settings.patch_section(name, values) {
                    return ToolResult::error(err);
                }
                settings.apply(&mut e.world);
                if let Err(err) = save(&e.world) {
                    return ToolResult::error(format!("applied, but saving failed: {err}"));
                }
            }
            match section {
                Some(name) => match settings.section_json(name) {
                    Some(v) => ToolResult::json(json!({ name: v })),
                    None => ToolResult::error(format!("unknown section `{name}`")),
                },
                None => ToolResult::json(serde_json::to_value(&settings).unwrap_or_default()),
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(root: &Path) -> World {
        let mut w = World::new();
        crate::engine::init_world(&mut w, root.to_path_buf());
        w.resources.insert(Physics::default());
        w
    }

    #[test]
    fn round_trip_through_files() {
        let dir = std::env::temp_dir().join(format!("oxide_ps_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut w = world(&dir);
        let mut s = ProjectSettings::capture(&w);
        assert_eq!(s, ProjectSettings::default().with_player_from(&s));
        s.tags_and_layers.tags.push("Enemy".into());
        s.tags_and_layers.layers[9] = "Bullets".into();
        s.time.fixed_timestep = 0.01;
        s.physics.gravity = [0.0, -20.0, 0.0];
        s.physics.set_collides(9, 9, false);
        s.quality.current = 1;
        s.input.axes.push(InputAxis::new("Steer"));
        s.player.company_name = "Acme".into();
        s.apply(&mut w);
        save(&w).unwrap();
        assert!(dir.join("ProjectSettings/TagManager.yaml").is_file());

        let mut w2 = world(&dir);
        load_and_apply(&mut w2);
        assert_eq!(w2.all_tags().last().map(String::as_str), Some("Enemy"));
        assert_eq!(w2.layers.name_to_layer("Bullets"), Some(9));
        assert!((w2.resource::<Time>().fixed_delta_time() - 0.01).abs() < 1e-6);
        let p = w2.resource::<Physics>();
        assert_eq!(p.gravity().y, -20.0);
        assert_eq!(p.layer_collision_matrix[9] & (1 << 9), 0);
        assert_eq!(QualitySettings::of(&w2).name, "Low");
        assert!(
            w2.resource::<Input>()
                .axes()
                .iter()
                .any(|a| a.name == "Steer")
        );
        assert_eq!(w2.resource::<Application>().company_name, "Acme");
        assert_eq!(ProjectSettings::capture(&w2), ProjectSettings::capture(&w));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn patch_checks_keys() {
        let mut s = ProjectSettings::default();
        s.patch_section("time", &serde_json::json!({"time_scale": 0.5}))
            .unwrap();
        assert_eq!(s.time.time_scale, 0.5);
        assert!(
            s.patch_section("time", &serde_json::json!({"nope": 1}))
                .is_err()
        );
        assert!(s.patch_section("bogus", &serde_json::json!({})).is_err());
    }

    impl ProjectSettings {
        /// Defaults, with the player page (filled from Application) taken from `other`.
        fn with_player_from(mut self, other: &Self) -> Self {
            self.player = other.player.clone();
            self
        }
    }
}
