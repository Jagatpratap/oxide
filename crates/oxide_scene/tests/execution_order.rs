//! Script Execution Order: the Project Settings list and `default_execution_order` decide
//! which scripts' Awake / Start / Update run first.

use oxide_core::Reflect;
use oxide_scene::{Component, Ctx, Phase, ScriptExecutionOrder, World};
use std::sync::Mutex;

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

macro_rules! script {
    ($name:ident, $order:expr) => {
        #[derive(Clone, Default, Reflect)]
        struct $name;
        impl Component for $name {
            fn default_execution_order(&self) -> i32 {
                $order
            }
            fn awake(&mut self, _: &mut Ctx) {
                LOG.lock()
                    .unwrap()
                    .push(format!("awake {}", stringify!($name)));
            }
            fn update(&mut self, _: &mut Ctx) {
                LOG.lock()
                    .unwrap()
                    .push(format!("update {}", stringify!($name)));
            }
        }
    };
}

script!(Camera2, 0);
script!(Player, 0);
script!(Input2, -100);

#[test]
fn scripts_run_in_execution_order() {
    let mut w = World::new();
    // Edit mode while the scene is built; Play starts them all at once.
    w.run_scripts = false;
    let a = w.create("A");
    w.add_component(a, Camera2);
    let b = w.create("B");
    w.add_component(b, Player);
    let c = w.create("C");
    w.add_component(c, Input2);
    LOG.lock().unwrap().clear();
    // Input2's [DefaultExecutionOrder(-100)] puts it first; the rest keep scene order.
    w.start_scripts();
    w.run_start();
    w.run_phase(Phase::Update);
    let log = std::mem::take(&mut *LOG.lock().unwrap());
    assert_eq!(
        log,
        [
            "awake Input2",
            "awake Camera2",
            "awake Player",
            "update Input2",
            "update Camera2",
            "update Player"
        ]
    );
    // Project Settings: Camera2 after everything, Player before Input2.
    let mut order = ScriptExecutionOrder::default();
    order.set("Camera2", 200);
    order.set("Player", -200);
    w.resources.insert(order);
    w.run_phase(Phase::Update);
    let log = std::mem::take(&mut *LOG.lock().unwrap());
    assert_eq!(log, ["update Player", "update Input2", "update Camera2"]);
}
