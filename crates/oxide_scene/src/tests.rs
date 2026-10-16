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

#[test]
fn script_reload_keeps_fields_order_and_lifecycle() {
    let mut w = world();
    let a = w.create("A");
    w.add_component(a, Light::default());
    let pid = w.add_component(a, probe("reload-p")).unwrap();
    w.add_component(a, Camera::default());
    w.component_mut::<Probe>(a, pid).unwrap().speed = 7.0;
    w.run_start();
    let mine = || {
        LOG.lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("reload-p "))
            .cloned()
            .collect::<Vec<_>>()
    };
    let before = mine().len();

    // Unload: the script gets OnDisable only.
    let taken = w.take_components_for_reload(|t| t == "Probe");
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].index, 1);
    assert!(w.get_component::<Probe>(a).is_none());
    w.registry.unregister_where(|r| r.info.name == "Probe");

    // Script type gone: data is kept in a MissingScript, which saves as the script.
    let missing = w.restore_components_after_reload(taken);
    assert_eq!(missing, vec!["Probe".to_string()]);
    let saved = w.serialize(&[a], "x");
    assert_eq!(saved.objects[0].components[1].type_name, "Probe");
    assert_eq!(
        saved.objects[0].components[1].fields.get("speed"),
        Some(&oxide_core::reflect::Value::Float(7.0))
    );

    // Script back: it returns in the same slot with its value, enabled again, not re-Awoken.
    w.registry.register::<Probe>("");
    let taken = w.take_components_for_reload(|t| t == "Probe");
    assert!(w.restore_components_after_reload(taken).is_empty());
    let list = w.get(a).unwrap().component_list();
    assert_eq!(
        list.iter().map(|c| c.1).collect::<Vec<_>>(),
        ["Light", "Probe", "Camera"]
    );
    assert_eq!(w.get_component::<Probe>(a).unwrap().speed, 7.0);
    // Each reload: OnDisable going out; OnEnable coming back only once it is a real script
    // again (the MissingScript in between has no callbacks). Never Awake/Start/OnDestroy.
    assert_eq!(
        mine()[before..],
        ["reload-p disable", "reload-p enable"],
        "only OnDisable/OnEnable across reloads"
    );
}

#[derive(Clone, Default, Reflect)]
struct Panicker;

impl Component for Panicker {
    fn update(&mut self, _: &mut Ctx) {
        panic!("boom");
    }
}

#[test]
fn panicking_script_does_not_take_the_engine_down() {
    let mut w = world();
    let a = w.create("A");
    w.add_component(a, Panicker);
    let b = w.create("B");
    w.add_component(b, probe("b"));
    w.run_start();
    w.run_phase(Phase::Update);
    // The other script still ran, and the panicking one is still attached.
    assert!(LOG.lock().unwrap().iter().any(|l| l == "b update"));
    assert!(w.get_component::<Panicker>(a).is_some());
}

#[test]
fn static_flags_round_trip_and_read_old_bool() {
    let mut w = world();
    let a = w.create("Wall");
    let mut f = StaticEditorFlags::NOTHING;
    f.set(StaticEditorFlags::OCCLUDER_STATIC, true);
    f.set(StaticEditorFlags::OCCLUDEE_STATIC, true);
    w.get_mut(a).unwrap().static_flags = f;
    w.create("Dynamic");
    let yaml = w.serialize(&[], "S").to_yaml().unwrap();
    let back = SceneFile::from_yaml(&yaml).unwrap();
    let mut w2 = world();
    w2.spawn(&back, None);
    let wall = w2.find("Wall").unwrap();
    assert_eq!(w2.get(wall).unwrap().static_flags, f);
    assert_eq!(
        w2.get(wall).unwrap().static_flags.names(),
        ["Occluder Static", "Occludee Static"]
    );
    assert!(!w2.get(w2.find("Dynamic").unwrap()).unwrap().is_static());

    // Scenes saved before the flags existed wrote `static: true`.
    let old = yaml.replace(&format!("static: {}", f.0), "static: true");
    assert_ne!(old, yaml);
    let mut w3 = world();
    w3.spawn(&SceneFile::from_yaml(&old).unwrap(), None);
    assert_eq!(
        w3.get(w3.find("Wall").unwrap()).unwrap().static_flags,
        StaticEditorFlags::EVERYTHING
    );
    assert_eq!(
        StaticEditorFlags::from_name("occluder_static"),
        Some(StaticEditorFlags::OCCLUDER_STATIC)
    );
}
