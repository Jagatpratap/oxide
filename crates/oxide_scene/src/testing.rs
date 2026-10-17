//! Unity Test Framework's play-mode tests (`[UnityTest]`): async functions that run in Play
//! mode as coroutines, across frames, and fail by panicking (`assert!`, `assert_eq!`).
//!
//! ```ignore
//! async fn car_falls_onto_the_ground(co: Co) {
//!     let car = co.with(|ctx| ctx.world.create("Car"));
//!     co.wait_for_seconds(1.0).await;
//!     co.with(|ctx| assert!(ctx.world.position(car).y < 1.0));
//! }
//! oxide::scripts! { ...; tests: car_falls_onto_the_ground }
//! ```
//!
//! Edit-mode tests (`[Test]`) are plain `#[test]` functions in the Scripts crate; the editor's
//! Test Runner runs both kinds. <https://docs.unity3d.com/Packages/com.unity.test-framework@1.4/manual/index.html>

use crate::component::Component;
use crate::coroutines::Co;
use crate::id::GameObjectId;
use crate::world::World;
use oxide_core::Reflect;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

/// A play-mode test's body.
pub type PlayModeTestFn = fn(Co) -> Pin<Box<dyn Future<Output = ()>>>;

/// One registered play-mode test.
#[derive(Clone)]
pub struct PlayModeTest {
    pub name: &'static str,
    pub run: PlayModeTestFn,
}

/// The project's play-mode tests (a World resource the Scripts library fills).
#[derive(Clone, Default)]
pub struct PlayModeTests {
    pub tests: Vec<PlayModeTest>,
}

impl PlayModeTests {
    pub fn register(&mut self, name: &'static str, run: PlayModeTestFn) {
        self.tests.retain(|t| t.name != name);
        self.tests.push(PlayModeTest { name, run });
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tests.iter().map(|t| t.name).collect()
    }
}

/// The object a play-mode test runs on (Unity's "Code-based tests runner").
#[derive(Clone, Default, Reflect)]
pub struct TestRunnerHost;
impl Component for TestRunnerHost {}

/// A running test's outcome: `None` while it runs, then `Ok` or the failure message.
pub type TestOutcome = Arc<Mutex<Option<Result<(), String>>>>;

/// Wraps a future so a panic inside becomes the test's failure instead of unwinding.
struct CatchUnwind {
    inner: Pin<Box<dyn Future<Output = ()>>>,
    outcome: TestOutcome,
}

impl Future for CatchUnwind {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            this.inner.as_mut().poll(cx)
        }));
        match result {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(())) => {
                *this.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(Ok(()));
                Poll::Ready(())
            }
            Err(p) => {
                let msg = p
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| p.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "panic".into());
                *this.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(Err(msg));
                Poll::Ready(())
            }
        }
    }
}

/// Starts play-mode test `test` on a fresh runner object (the World should be in Play mode).
/// Returns the runner and the outcome to watch.
pub fn start_play_mode_test(world: &mut World, test: &PlayModeTest) -> (GameObjectId, TestOutcome) {
    let host = world.create("Code-based tests runner");
    let cid = world
        .add_component(host, TestRunnerHost)
        .expect("runner object exists");
    let outcome: TestOutcome = Arc::new(Mutex::new(None));
    let o = outcome.clone();
    let run = test.run;
    world.call(host, cid, move |_, ctx| {
        ctx.start_coroutine(move |co| CatchUnwind {
            inner: run(co),
            outcome: o,
        });
    });
    (host, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coroutines::{Coroutines, Stage};

    async fn passes(co: Co) {
        co.next_frame().await;
        let n = co.with(|ctx| ctx.world.len());
        assert!(n >= 1);
    }

    async fn fails(co: Co) {
        co.wait_for_seconds(0.5).await;
        assert_eq!(1 + 1, 3, "maths is broken");
    }

    #[test]
    fn play_mode_tests_report_pass_and_fail() {
        let mut w = World::new();
        w.resources.insert(oxide_core::Time::new());
        let mut tests = PlayModeTests::default();
        tests.register("passes", |co| Box::pin(passes(co)));
        tests.register("fails", |co| Box::pin(fails(co)));
        assert_eq!(tests.names(), ["passes", "fails"]);
        let outcomes: Vec<TestOutcome> = tests
            .tests
            .iter()
            .map(|t| start_play_mode_test(&mut w, t).1)
            .collect();
        for _ in 0..4 {
            w.resource_mut::<oxide_core::Time>().begin_frame(0.25);
            Coroutines::run(&mut w, Stage::AfterUpdate);
        }
        assert_eq!(*outcomes[0].lock().unwrap(), Some(Ok(())));
        let fail = outcomes[1].lock().unwrap().clone();
        assert!(
            matches!(&fail, Some(Err(m)) if m.contains("maths is broken")),
            "{fail:?}"
        );
    }
}
