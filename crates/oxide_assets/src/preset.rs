//! Unity's Preset asset (`.preset`): a component's saved settings that can be applied to
//! other components of the same type.
//! Reference: <https://docs.unity3d.com/Manual/Presets.html>

use crate::handle::Asset;
use oxide_core::reflect::{FieldInfo, Reflect, ReflectError, TypeInfo, Value, ValueKind};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preset {
    /// Component type the values belong to (`Light`, `Rigidbody`, ...).
    pub target_type: String,
    /// Saved field values by name.
    pub values: BTreeMap<String, Value>,
}

impl Asset for Preset {
    const TYPE_NAME: &'static str = "Preset";
}

impl crate::AssetServer {
    /// The component type a preset file is for, without loading it as an asset.
    pub fn preset_target(&self, path: &str) -> Option<String> {
        if let Some(p) = self.data.get_dyn(path) {
            return match p.as_reflect().get_field("target_type") {
                Some(Value::String(s)) => Some(s),
                _ => None,
            };
        }
        let text = std::fs::read_to_string(self.full_path(path)).ok()?;
        let line = text
            .lines()
            .find(|l| l.trim_start().starts_with("target_type:"))?;
        let v = line.split_once(':')?.1.trim();
        Some(
            v.trim_start_matches("!String")
                .trim()
                .trim_matches('\'')
                .trim_matches('"')
                .to_owned(),
        )
    }

    /// Preset files for a component type (Unity's Select Preset list).
    pub fn presets_for(&self, type_name: &str) -> Vec<String> {
        self.paths_of_type("Preset")
            .into_iter()
            .filter(|p| self.preset_target(p).as_deref() == Some(type_name))
            .collect()
    }
}

impl Preset {
    /// A preset of every field of `component`.
    pub fn of(component: &dyn Reflect) -> Self {
        Self {
            target_type: component.type_info().name.to_owned(),
            values: component
                .fields()
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
        }
    }

    /// Writes the saved values into `component` (fields it no longer has are skipped).
    /// Returns the names that could not be applied.
    pub fn apply(&self, component: &mut dyn Reflect) -> Vec<String> {
        self.values
            .iter()
            .filter_map(|(k, v)| component.set_field(k, v.clone()).err().map(|_| k.clone()))
            .collect()
    }

    pub fn can_apply_to(&self, component: &dyn Reflect) -> bool {
        self.target_type == component.type_info().name
    }
}

/// Saved as `{target_type, values}` like any data asset.
impl Reflect for Preset {
    fn type_info(&self) -> TypeInfo {
        TypeInfo {
            name: "Preset",
            fields: vec![
                FieldInfo::plain("target_type", ValueKind::String),
                FieldInfo::plain("values", ValueKind::Object),
            ],
        }
    }

    fn get_field(&self, name: &str) -> Option<Value> {
        match name {
            "target_type" => Some(Value::String(self.target_type.clone())),
            "values" => Some(Value::Object(self.values.clone())),
            _ => None,
        }
    }

    fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError> {
        match (name, value) {
            ("target_type", Value::String(s)) => self.target_type = s,
            ("values", Value::Object(m)) => self.values = m,
            ("target_type" | "values", _) => {
                return Err(ReflectError::WrongType {
                    field: name.to_owned(),
                    expected: if name == "values" {
                        ValueKind::Object
                    } else {
                        ValueKind::String
                    },
                });
            }
            _ => return Err(ReflectError::NoSuchField(name.to_owned())),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_core::Reflect;

    #[derive(Reflect, Clone, Default, Debug, PartialEq)]
    struct Lamp {
        intensity: f32,
        color: oxide_math::Color,
    }

    #[test]
    fn save_apply_and_round_trip() {
        let lamp = Lamp {
            intensity: 3.0,
            color: oxide_math::Color::RED,
        };
        let p = Preset::of(&lamp);
        assert_eq!(p.target_type, "Lamp");
        let mut other = Lamp::default();
        assert!(p.can_apply_to(&other));
        assert!(p.apply(&mut other).is_empty());
        assert_eq!(other, lamp);

        let dir = std::env::temp_dir().join(format!("oxide-preset-{}", std::process::id()));
        let mut a = crate::AssetServer::new(&dir);
        a.save_data("Assets/Red.preset", Box::new(p.clone()))
            .unwrap();
        a.data.remove("Assets/Red.preset");
        a.ensure_loaded("Assets/Red.preset").unwrap();
        let h: crate::Handle<Preset> = crate::Handle::from_path("Assets/Red.preset");
        assert_eq!(a.data.get(&h), Some(&p));
        let _ = std::fs::remove_dir_all(dir);
    }
}
