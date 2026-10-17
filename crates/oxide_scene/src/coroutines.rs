//! Unity coroutines as Rust `async` blocks: [`Ctx::start_coroutine`] runs a future that can
//! wait for seconds, frames, the end of the frame, the next fixed update or a condition,
//! and reach its object between waits with [`Co::with`].
//!
//! ```ignore
//! ctx.start_coroutine(|co| async move {
//!     co.wait_for_seconds(1.0).await;                 // yield return new WaitForSeconds(1)
//!     co.with(|ctx| ctx.set_active(false));
//!     co.next_frame().await;                          // yield return null
//!     co.wait_until(|ctx| ctx.input().key_down(KeyCode::Space)).await;
//!     co.with_component::<Gun, _>(|gun, _| gun.ammo += 10);
//! });
//! ```
//!
//! The engine polls them where Unity resumes coroutines: after `Update` (frames, seconds,
//! conditions), after each `FixedUpdate` (`WaitForFixedUpdate`) and after rendering
//! (`WaitForEndOfFrame`). They stop when their GameObject is deactivated or destroyed, and
//! when scripts reload. <https://docs.unity3d.com/Manual/coroutines.html>

use crate::component::Component;
use crate::ctx::Ctx;
use crate::id::{ComponentId, GameObjectId};
use crate::world::World;
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

/// Where the player loop is when coroutines are resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// After `Update`: `yield return null`, `WaitForSeconds`, `WaitUntil`/`WaitWhile`.
    AfterUpdate,
    /// After each `FixedUpdate` step: `WaitForFixedUpdate`.
    AfterFixedUpdate,
    /// After the frame is rendered: `WaitForEndOfFrame`.
    EndOfFrame,
}

/// What the running coroutine may reach during a poll.
#[derive(Clone, Copy)]
struct Current {
    world: *mut World,
    stage: Stage,
    frame: u64,
}

thread_local! {
    static CURRENT: Cell<Option<Current>> = const { Cell::new(None) };
}

fn current() -> Current {
    CURRENT
        .with(Cell::get)
        .expect("coroutine waits and `Co::with` only work inside a running coroutine")
}

/// A started coroutine (Unity's `Coroutine`), for [`Ctx::stop_coroutine`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CoroutineId(u64);

/// The coroutine's handle on its object: its waits, and access between them.
#[derive(Clone, Copy, Debug)]
pub struct Co {
    pub object: GameObjectId,
    pub component: ComponentId,
}

impl Co {
    /// Runs `f` with the object's [`Ctx`] (like code in the coroutine's own method).
    pub fn with<R>(&self, f: impl FnOnce(&mut Ctx) -> R) -> R {
        let c = current();
        // SAFETY: the world pointer is set only while the executor polls this coroutine,
        // with the world exclusively borrowed by the executor and not otherwise used.
        let world = unsafe { &mut *c.world };
        let mut ctx = Ctx {
            world,
            this: self.object,
            component: self.component,
        };
        f(&mut ctx)
    }

    /// Runs `f` on the script that started the coroutine (or another `T` on its object).
    /// `None` when there is no such component.
    pub fn with_component<T: Component, R>(
        &self,
        f: impl FnOnce(&mut T, &mut Ctx) -> R,
    ) -> Option<R> {
        let object = self.object;
        self.with(|ctx| ctx.with_component::<T, R>(object, f))
    }

    /// Unity `yield return new WaitForSeconds(s)` (scaled time).
    pub fn wait_for_seconds(&self, seconds: f32) -> WaitFor {
        WaitFor::Seconds {
            seconds,
            realtime: false,
            deadline: None,
        }
    }

    /// Unity `WaitForSecondsRealtime` (ignores `Time.timeScale`).
    pub fn wait_for_seconds_realtime(&self, seconds: f32) -> WaitFor {
        WaitFor::Seconds {
            seconds,
            realtime: true,
            deadline: None,
        }
    }

    /// Unity `yield return null`: the next frame, after `Update`.
    pub fn next_frame(&self) -> WaitFor {
        WaitFor::Frame { after: None }
    }

    /// Unity `WaitForEndOfFrame`: after this frame is rendered.
    pub fn wait_for_end_of_frame(&self) -> WaitFor {
        WaitFor::Stage(Stage::EndOfFrame)
    }

    /// Unity `WaitForFixedUpdate`: after the next fixed-update step.
    pub fn wait_for_fixed_update(&self) -> WaitFor {
        WaitFor::Stage(Stage::AfterFixedUpdate)
    }

    /// Unity `WaitUntil(predicate)`: checked once per frame after `Update`.
    pub fn wait_until<F: FnMut(&mut Ctx) -> bool>(&self, predicate: F) -> WaitUntil<F> {
        WaitUntil {
            co: *self,
            predicate,
            negate: false,
        }
    }

    /// Unity `WaitWhile(predicate)`.
    pub fn wait_while<F: FnMut(&mut Ctx) -> bool>(&self, predicate: F) -> WaitUntil<F> {
        WaitUntil {
            co: *self,
            predicate,
            negate: true,
        }
    }
}

/// Unity's built-in yield instructions.
pub enum WaitFor {
    Seconds {
        seconds: f32,
        realtime: bool,
        deadline: Option<f32>,
    },
    Frame {
        after: Option<u64>,
    },
    Stage(Stage),
}

fn now(world: &World, realtime: bool) -> f32 {
    let t = world.resource::<oxide_core::Time>();
    if realtime {
        t.unscaled_time()
    } else {
        t.time()
    }
}

impl Future for WaitFor {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        let c = current();
        // SAFETY: as in `Co::with`.
        let world = unsafe { &*c.world };
        match &mut *self {
            WaitFor::Seconds {
                seconds,
                realtime,
                deadline,
            } => {
                let t = now(world, *realtime);
                let d = *deadline.get_or_insert(t + *seconds);
                if c.stage == Stage::AfterUpdate && t >= d {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }
            WaitFor::Frame { after } => {
                let start = *after.get_or_insert(c.frame);
                if c.stage == Stage::AfterUpdate && c.frame > start {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }
            WaitFor::Stage(s) => {
                if c.stage == *s {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

/// [`Co::wait_until`] / [`Co::wait_while`].
pub struct WaitUntil<F> {
    co: Co,
    predicate: F,
    negate: bool,
}

impl<F: FnMut(&mut Ctx) -> bool + Unpin> Future for WaitUntil<F> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if current().stage != Stage::AfterUpdate {
            return Poll::Pending;
        }
        let this = &mut *self;
        let co = this.co;
        let v = co.with(|ctx| (this.predicate)(ctx));
        if v != this.negate {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

struct Task {
    id: CoroutineId,
    co: Co,
    future: Pin<Box<dyn Future<Output = ()>>>,
}

/// The running coroutines (a World resource; made on first use).
#[derive(Default)]
pub struct Coroutines {
    tasks: Vec<Task>,
    next: u64,
    /// Stopped while their stage ran (removed after it).
    stopped: Vec<CoroutineId>,
}

fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );
    // SAFETY: the vtable's functions do nothing and never touch the data pointer.
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

impl Coroutines {
    pub fn start(&mut self, co: Co, future: Pin<Box<dyn Future<Output = ()>>>) -> CoroutineId {
        self.next += 1;
        let id = CoroutineId(self.next);
        self.tasks.push(Task { id, co, future });
        id
    }

    pub fn stop(&mut self, id: CoroutineId) {
        self.tasks.retain(|t| t.id != id);
        self.stopped.push(id);
    }

    /// Unity `StopAllCoroutines` for one component.
    pub fn stop_all(&mut self, object: GameObjectId, component: ComponentId) {
        let ids: Vec<CoroutineId> = self
            .tasks
            .iter()
            .filter(|t| t.co.object == object && t.co.component == component)
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.stop(id);
        }
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Drops every coroutine (script reload: their code is about to go away).
    pub fn clear(&mut self) {
        self.tasks.clear();
    }

    /// Resumes the coroutines waiting for `stage`. Called by the engine.
    pub fn run(world: &mut World, stage: Stage) {
        let Some(mut tasks) = world
            .try_resource_mut::<Coroutines>()
            .map(|c| std::mem::take(&mut c.tasks))
        else {
            return;
        };
        if tasks.is_empty() {
            return;
        }
        let frame = world.resource::<oxide_core::Time>().frame_count();
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut keep = Vec::with_capacity(tasks.len());
        for mut task in tasks.drain(..) {
            // Deactivated or destroyed objects stop their coroutines (Unity).
            if !world
                .get(task.co.object)
                .is_some_and(|o| o.active_in_hierarchy())
            {
                continue;
            }
            // Stopped by an earlier coroutine this stage.
            if world
                .try_resource::<Coroutines>()
                .is_some_and(|c| c.stopped.contains(&task.id))
            {
                continue;
            }
            CURRENT.with(|c| {
                c.set(Some(Current {
                    world: world as *mut World,
                    stage,
                    frame,
                }))
            });
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                task.future.as_mut().poll(&mut cx)
            }));
            CURRENT.with(|c| c.set(None));
            match result {
                Ok(Poll::Pending) => keep.push(task),
                Ok(Poll::Ready(())) => {}
                Err(p) => {
                    let msg = p
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| p.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "panic".into());
                    let name = world.get(task.co.object).map_or("?", |o| o.name.as_str());
                    oxide_core::debug_log_error!("Coroutine on \"{name}\": {msg}");
                }
            }
        }
        let c = world.resource_mut::<Coroutines>();
        let stopped = std::mem::take(&mut c.stopped);
        keep.retain(|t| !stopped.contains(&t.id));
        // Coroutines started during this stage were added to the (emptied) list.
        keep.append(&mut c.tasks);
        c.tasks = keep;
        world.flush_destroyed();
    }
}

impl Ctx<'_> {
    /// Unity `StartCoroutine`: runs `body(co)` as a coroutine of this component. The first
    /// part runs at once (until its first wait), like Unity.
    pub fn start_coroutine<F, Fut>(&mut self, body: F) -> CoroutineId
    where
        F: FnOnce(Co) -> Fut,
        Fut: Future<Output = ()> + 'static,
    {
        let co = Co {
            object: self.this,
            component: self.component,
        };
        if self.world.try_resource::<Coroutines>().is_none() {
            self.world.resources.insert(Coroutines::default());
        }
        let mut future: Pin<Box<dyn Future<Output = ()>>> = Box::pin(body(co));
        // Unity runs a coroutine up to its first yield inside StartCoroutine.
        let frame = self
            .world
            .try_resource::<oxide_core::Time>()
            .map_or(0, |t| t.frame_count());
        let outer = CURRENT.with(Cell::get);
        CURRENT.with(|c| {
            c.set(Some(Current {
                world: self.world as *mut World,
                stage: Stage::AfterUpdate,
                frame,
            }))
        });
        let waker = noop_waker();
        let done = future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_ready();
        CURRENT.with(|c| c.set(outer));
        let coroutines = self.world.resource_mut::<Coroutines>();
        if done {
            coroutines.next += 1;
            CoroutineId(coroutines.next)
        } else {
            coroutines.start(co, future)
        }
    }

    /// Unity `StopCoroutine`.
    pub fn stop_coroutine(&mut self, id: CoroutineId) {
        if let Some(c) = self.world.try_resource_mut::<Coroutines>() {
            c.stop(id);
        }
    }

    /// Unity `StopAllCoroutines` (this component's).
    pub fn stop_all_coroutines(&mut self) {
        let (o, c) = (self.this, self.component);
        if let Some(cs) = self.world.try_resource_mut::<Coroutines>() {
            cs.stop_all(o, c);
        }
    }
}
