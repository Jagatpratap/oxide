//! Coroutines: Unity's waits as async, resumed at the right points of the player loop, and
//! stopped by StopCoroutine or by deactivating the object.

use oxide_core::{Reflect, Time};
use oxide_scene::coroutines::Stage;
use oxide_scene::{Component, Coroutines, World};
use std::sync::{Arc, Mutex};

type Log = Arc<Mutex<Vec<String>>>;

#[derive(Clone, Default, Reflect)]
struct Script {
    #[reflect(skip)]
    ready: bool,
}
impl Component for Script {}

fn world() -> World {
    let mut w = World::new();
    w.resources.insert(Time::new());
    w.run_scripts = true;
    w
}

/// One frame of the player loop as far as coroutines go.
fn frame(w: &mut World, dt: f64) {
    w.resource_mut::<Time>().begin_frame(dt);
    Coroutines::run(w, Stage::AfterFixedUpdate);
    Coroutines::run(w, Stage::AfterUpdate);
    Coroutines::run(w, Stage::EndOfFrame);
}

fn start(w: &mut World, id: oxide_scene::GameObjectId, log: &Log) -> oxide_scene::CoroutineId {
    let cid = w.component_id::<Script>(id).unwrap();
    let mut out = None;
    let log = log.clone();
    w.call(id, cid, |_, ctx| {
        out = Some(ctx.start_coroutine(move |co| async move {
            let say = |s: &str| log.lock().unwrap().push(s.to_owned());
            say("start");
            co.wait_for_seconds(1.0).await;
            say("1s");
            co.next_frame().await;
            say("frame");
            co.wait_until(|ctx| {
                let id = ctx.this;
                ctx.world
                    .get_component::<Script>(id)
                    .is_some_and(|s| s.ready)
            })
            .await;
            say("until");
            co.wait_for_end_of_frame().await;
            say("eof");
            co.wait_for_fixed_update().await;
            say("fixed");
            co.with_component::<Script, _>(|s, _| s.ready = false);
            // Like Unity, it runs on to its next wait, then stops.
            co.with(|ctx| ctx.set_active(false));
            co.next_frame().await;
            say("never: deactivated");
        }));
    });
    out.unwrap()
}

#[test]
fn waits_resume_in_order() {
    let mut w = world();
    let id = w.create("Runner");
    w.add_component(id, Script::default());
    let log: Log = Arc::default();
    start(&mut w, id, &log);
    let seen = |log: &Log| log.lock().unwrap().clone();
    // The body runs at once up to its first wait.
    assert_eq!(seen(&log), ["start"]);
    for _ in 0..3 {
        frame(&mut w, 0.25);
    }
    assert_eq!(seen(&log), ["start"], "0.75 s: still waiting");
    frame(&mut w, 0.25);
    assert_eq!(seen(&log), ["start", "1s"]);
    frame(&mut w, 0.25);
    assert_eq!(seen(&log), ["start", "1s", "frame"], "a frame later");
    frame(&mut w, 0.25);
    frame(&mut w, 0.25);
    assert_eq!(seen(&log).len(), 3, "WaitUntil holds");
    w.get_component_mut::<Script>(id).unwrap().ready = true;
    frame(&mut w, 0.25);
    // Until (after Update), then end of frame in the same frame, then the next fixed step.
    assert_eq!(seen(&log), ["start", "1s", "frame", "until", "eof"]);
    frame(&mut w, 0.25);
    assert_eq!(
        seen(&log),
        ["start", "1s", "frame", "until", "eof", "fixed"]
    );
    // It deactivated its object, so the rest never runs and the coroutine is gone.
    frame(&mut w, 0.25);
    assert_eq!(seen(&log).len(), 6);
    assert!(w.resource::<Coroutines>().is_empty());
}

#[test]
fn stop_coroutine_and_deactivation_stop_it() {
    let mut w = world();
    let id = w.create("Runner");
    w.add_component(id, Script::default());
    let log: Log = Arc::default();
    let c = start(&mut w, id, &log);
    let cid = w.component_id::<Script>(id).unwrap();
    w.call(id, cid, |_, ctx| ctx.stop_coroutine(c));
    for _ in 0..10 {
        frame(&mut w, 0.25);
    }
    assert_eq!(
        log.lock().unwrap().len(),
        1,
        "stopped before its first wait ended"
    );
    // Deactivating the GameObject stops them too (Unity).
    let log2: Log = Arc::default();
    start(&mut w, id, &log2);
    w.set_active(id, false);
    for _ in 0..10 {
        frame(&mut w, 0.25);
    }
    assert_eq!(log2.lock().unwrap().len(), 1);
    assert!(w.resource::<Coroutines>().is_empty());
}
