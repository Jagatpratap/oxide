//! Unity's Script Execution Order (Project Settings) and `[DefaultExecutionOrder]`: scripts
//! with a lower order get Awake, OnEnable, Start, FixedUpdate, Update and LateUpdate before
//! those with a higher one (default 0); equal orders keep scene order.
//! <https://docs.unity3d.com/Manual/class-MonoManager.html>

/// One script's order in the Project Settings list.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScriptOrder {
    /// Component type name.
    pub script: String,
    pub order: i32,
}

/// The Project Settings list (a World resource; `ProjectSettings/ScriptExecutionOrder.yaml`).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ScriptExecutionOrder {
    pub scripts: Vec<ScriptOrder>,
}

impl ScriptExecutionOrder {
    /// The order of `type_name`: the list's entry, else the script's own default
    /// (`[DefaultExecutionOrder]`).
    pub fn order_of(&self, type_name: &str, default: i32) -> i32 {
        self.scripts
            .iter()
            .find(|s| s.script == type_name)
            .map_or(default, |s| s.order)
    }

    /// Sets (or with 0 and no default, removes) a script's entry.
    pub fn set(&mut self, type_name: &str, order: i32) {
        match self.scripts.iter_mut().find(|s| s.script == type_name) {
            Some(s) => s.order = order,
            None => self.scripts.push(ScriptOrder {
                script: type_name.to_owned(),
                order,
            }),
        }
        self.scripts.sort_by_key(|s| s.order);
    }

    pub fn remove(&mut self, type_name: &str) {
        self.scripts.retain(|s| s.script != type_name);
    }
}
