use crate::*;
use oxide_core::Reflect;
use oxide_math::Vector3;
use std::sync::Mutex;

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log(s: impl Into<String>) {
    LOG.lock().unwrap().push(s.into());
}

#[derive(Clone, Default, Reflect)]
struct Probe {
    label: String,
    target: Option<GameObjectId>,
    speed: f32,
}

impl Component for Probe {
    fn awake(&mut self, _: &mut Ctx) {
        log(format!("{} awake", self.label));
    }
    fn on_enable(&mut self, _: &mut Ctx) {
        log(format!("{} enable", self.label));
    }
    fn start(&mut self, _: &mut Ctx) {
        log(format!("{} start", self.label));
    }
    fn update(&mut self, ctx: &mut Ctx) {
        log(format!("{} update", self.label));
        let s = self.speed;
        ctx.transform().translate(Vector3::Z * s);
    }
    fn on_disable(&mut self, _: &mut Ctx) {
        log(format!("{} disable", self.label));
    }
    fn on_destroy(&mut self, _: &mut Ctx) {
        log(format!("{} destroy", self.label));
    }
}

fn world() -> World {
    let mut w = World::new();
    register_builtin(&mut w.registry);
    w.registry.register::<Probe>("");
    w
}

fn probe(label: &str) -> Probe {
    Probe {
        label: label.into(),
        speed: 1.0,
        ..Default::default()
    }
}

#[test]
fn lifecycle_order_matches_unity() {
    let mut w = world();
    let a = w.create("A");
    w.add_component(a, probe("a"));
    w.run_start();
    w.run_phase(Phase::Update);
    w.set_active(a, false);
    w.set_active(a, true);
    w.destroy(a);
    w.flush_destroyed();
    // Tests run in parallel; keep only this test's entries.
    let got: Vec<String> = LOG
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.starts_with("a "))
        .cloned()
        .collect();
    assert_eq!(
        got,
        [
            "a awake",
            "a enable",
            "a start",
            "a update",
            "a disable",
            "a enable",
            "a disable",
            "a destroy"
        ]
    );
    assert_eq!(w.len(), 0);
}

#[test]
fn update_moves_and_scene_round_trips() {
    let mut w = world();
    let car = w.create("Car");
    let wheel = w.create("Wheel");
    w.set_parent(wheel, Some(car), false).unwrap();
    w.add_component(
        car,
        Probe {
            label: "car".into(),
            target: Some(wheel),
            speed: 2.0,
        },
    );
    w.add_component(
        wheel,
        MeshFilter {
            mesh: Some(oxide_assets::Handle::from_path("builtin:Cylinder")),
        },
    );
    w.add_component(wheel, MeshRenderer::default());
    w.run_start();
    w.run_phase(Phase::Update);
    assert_eq!(w.position(car), Vector3::new(0.0, 0.0, 2.0));
    assert_eq!(w.position(wheel), Vector3::new(0.0, 0.0, 2.0));

    let file = w.serialize(&[], "Test");
    let yaml = file.to_yaml().unwrap();
    assert!(yaml.contains("MeshRenderer"), "{yaml}");
    let back = SceneFile::from_yaml(&yaml).unwrap();
    assert_eq!(back, file);

    let mut w2 = world();
    w2.run_scripts = false;
    let (roots, warnings) = w2.spawn(&back, None);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(roots.len(), 1);
    let car2 = w2.find("Car").unwrap();
    let wheel2 = w2.find("Car/Wheel").unwrap();
    // The object reference was remapped to the new wheel.
    assert_eq!(
        w2.get_component::<Probe>(car2).unwrap().target,
        Some(wheel2)
    );
    assert_eq!(w2.serialize(&[], "Test"), file);
}

#[test]
fn instantiate_remaps_internal_refs_and_keeps_external() {
    let mut w = world();
    let target = w.create("Target");
    let root = w.create("Enemy");
    let gun = w.create("Gun");
    w.set_parent(gun, Some(root), false).unwrap();
    w.add_component(
        root,
        Probe {
            label: "enemy".into(),
            target: Some(gun),
            speed: 0.0,
        },
    );
    w.add_component(
        gun,
        Probe {
            label: "gun".into(),
            target: Some(target),
            speed: 0.0,
        },
    );
    let clone = w.instantiate(root).unwrap();
    assert_eq!(w.get(clone).unwrap().name, "Enemy(Clone)");
    let gun2 = w.find_child(clone, "Gun").unwrap();
    assert_ne!(gun2, gun);
    assert_eq!(w.get_component::<Probe>(clone).unwrap().target, Some(gun2));
    assert_eq!(w.get_component::<Probe>(gun2).unwrap().target, Some(target));
}
