//! Script-facing API: Inspector attributes, `[RequireComponent]`, `[ExecuteAlways]`,
//! `[ContextMenu]`, plural GetComponents, BroadcastMessage / SendMessageUpwards and
//! InvokeRepeating.

use oxide_core::Reflect;
use oxide_core::reflect::{Reflect as _, Value};
use oxide_scene::{Component, Ctx, World};

#[derive(Clone, Default, Reflect)]
struct Attributed {
    #[reflect(header = "Movement", min = 0, delayed)]
    speed: f32,
    #[reflect(space = 12, label = "Display Name", multiline = 4)]
    notes: String,
    #[reflect(hide)]
    secret: i32,
    #[reflect(hdr, formerly = "old_tint")]
    tint: oxide_math::Color,
}
impl Component for Attributed {}

#[test]
fn field_attributes_reach_the_inspector_info() {
    let a = Attributed::default();
    let info = a.type_info();
    let f = |n: &str| *info.fields.iter().find(|f| f.name == n).unwrap();
    let speed = f("speed");
    assert_eq!(speed.ui.header, Some("Movement"));
    assert_eq!(speed.ui.min, Some(0.0));
    assert!(speed.ui.delayed);
    let notes = f("notes");
    assert_eq!(
        (notes.ui.space, notes.ui.label, notes.ui.multiline),
        (12.0, Some("Display Name"), Some(4))
    );
    assert!(f("secret").ui.hide);
    assert!(f("tint").ui.hdr);
    // `formerly`: data saved under the old name still loads.
    let mut b = Attributed::default();
    b.set_field("old_tint", Value::Color([0.5, 0.25, 0.0, 1.0]))
        .unwrap();
    assert_eq!(b.tint.r, 0.5);
    // Plain fields stay plain (and keep their MCP / JSON info small).
    assert!(
        oxide_core::reflect::FieldInfo::plain("x", oxide_core::reflect::ValueKind::Float)
            .ui
            .is_plain()
    );
}

#[derive(Clone, Default, Reflect)]
struct Body {
    mass: f32,
}
impl Component for Body {}

#[derive(Clone, Default, Reflect)]
struct Mover {
    #[reflect(skip)]
    updates: u32,
    #[reflect(skip)]
    awakes: u32,
    resets: u32,
}

impl Component for Mover {
    fn requires(&self) -> &'static [&'static str] {
        &["Body"]
    }
    fn execute_always(&self) -> bool {
        true
    }
    fn context_menu(&self) -> &'static [&'static str] {
        &["Reset Counters"]
    }
    fn on_context_menu(&mut self, _: &mut Ctx, item: &str) {
        if item == "Reset Counters" {
            self.updates = 0;
            self.resets += 1;
        }
    }
    fn awake(&mut self, _: &mut Ctx) {
        self.awakes += 1;
    }
    fn update(&mut self, _: &mut Ctx) {
        self.updates += 1;
    }
}

#[test]
fn require_component_execute_always_and_context_menu() {
    let mut w = World::new();
    w.registry.register::<Body>("Physics/Body");
    w.registry.register::<Mover>("Scripts/Mover");
    w.run_scripts = false;
    let id = w.create("Car");
    w.add_component(id, Mover::default());
    // [RequireComponent]: the Body came along and can't be removed while Mover is there.
    assert!(w.get_component::<Body>(id).is_some());
    assert_eq!(w.required_by(id, "Body"), vec!["Mover"]);
    // [ExecuteAlways]: Awake once, Update every edit-mode frame.
    w.run_execute_always();
    w.run_execute_always();
    let m = w.get_component::<Mover>(id).unwrap();
    assert_eq!((m.awakes, m.updates), (1, 2));
    // [ContextMenu]
    let cid = w.component_id::<Mover>(id).unwrap();
    w.call(id, cid, |c, ctx| c.on_context_menu(ctx, "Reset Counters"));
    let m = w.get_component::<Mover>(id).unwrap();
    assert_eq!((m.updates, m.resets), (0, 1));
}

#[derive(Clone, Default, Reflect)]
struct Listener {
    #[reflect(skip)]
    heard: u32,
}
impl Component for Listener {
    fn on_message(&mut self, _: &mut Ctx, message: &str, _: &Value) -> bool {
        if message == "Ping" {
            self.heard += 1;
            true
        } else {
            false
        }
    }
}

#[test]
fn messages_plural_components_and_repeating_invokes() {
    let mut w = World::new();
    w.resources.insert(oxide_core::Time::new());
    w.resources.insert(oxide_scene::timers::Timers::default());
    let root = w.create("Root");
    let mid = w.create("Mid");
    let leaf = w.create("Leaf");
    w.set_parent(mid, Some(root), false).unwrap();
    w.set_parent(leaf, Some(mid), false).unwrap();
    for id in [root, mid, leaf] {
        w.add_component(id, Listener::default());
    }
    w.add_component(leaf, Listener::default());
    let heard = |w: &World, id| -> u32 {
        w.get_components::<Listener>(id)
            .iter()
            .map(|l| l.heard)
            .sum()
    };
    assert_eq!(w.get_components::<Listener>(leaf).len(), 2);
    assert_eq!(w.get_components_in_children::<Listener>(root).len(), 4);
    assert_eq!(w.get_components_in_parent::<Listener>(leaf).len(), 4);
    // BroadcastMessage goes down, SendMessageUpwards up.
    w.broadcast_message(mid, "Ping", Value::Null);
    assert_eq!(
        (heard(&w, root), heard(&w, mid), heard(&w, leaf)),
        (0, 1, 2)
    );
    w.send_message_upwards(mid, "Ping", Value::Null);
    assert_eq!((heard(&w, root), heard(&w, mid)), (1, 2));
    // InvokeRepeating fires every interval until cancelled.
    {
        let t = w.resource_mut::<oxide_scene::timers::Timers>();
        t.repeat_at(root, "Ping", 1.0, 0.5);
        assert!(t.is_invoking(root, Some("Ping")));
    }
    for now in [0.5, 1.0, 1.5, 2.0] {
        oxide_scene::timers::Timers::run(&mut w, now);
    }
    assert_eq!(heard(&w, root), 1 + 3, "at 1.0, 1.5 and 2.0");
    w.resource_mut::<oxide_scene::timers::Timers>()
        .cancel(root, Some("Ping"));
    oxide_scene::timers::Timers::run(&mut w, 3.0);
    assert_eq!(heard(&w, root), 4);
    assert!(
        !w.resource::<oxide_scene::timers::Timers>()
            .is_invoking(root, None)
    );
}
