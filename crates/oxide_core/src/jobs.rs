//! Unity's C# Job System for Rust scripts: work scheduled on a shared pool of worker threads
//! (one per core but the main thread's), with dependencies and handles to wait on.
//!
//! - [`Jobs::schedule`] — `IJob.Schedule(dependsOn)`: one closure.
//! - [`Jobs::schedule_parallel_for`] — `IJobParallelFor.Schedule(length, batch, dependsOn)`.
//! - [`JobHandle::complete`], [`JobHandle::is_completed`], [`JobHandle::combine`] —
//!   `JobHandle.Complete`, `IsCompleted`, `CombineDependencies`.
//! - [`Jobs::run`] — a job with a result ([`JobResult::complete`] returns it).
//! - [`Jobs::parallel_for`] / [`Jobs::par_for_each_mut`] — scheduled and completed at once,
//!   so they can borrow the caller's data (Unity's `job.Schedule(...).Complete()` over a
//!   `NativeArray`).
//!
//! Jobs are plain Rust closures, so the compiler checks what Unity's safety system checks
//! at runtime: shared data must be `Sync`, moved data `Send`. A job that panics logs an
//! error and still counts as completed.
//! <https://docs.unity3d.com/Manual/job-system.html>

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

type Task = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    queue: Mutex<VecDeque<Task>>,
    ready: Condvar,
    workers: usize,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<&'static Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let workers = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .saturating_sub(1)
            .max(1);
        let pool: &'static Pool = Box::leak(Box::new(Pool {
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
            workers,
        }));
        for i in 0..workers {
            std::thread::Builder::new()
                .name(format!("oxide job {i}"))
                .spawn(move || {
                    loop {
                        let task = {
                            let mut q = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
                            loop {
                                if let Some(t) = q.pop_front() {
                                    break t;
                                }
                                q = pool.ready.wait(q).unwrap_or_else(|e| e.into_inner());
                            }
                        };
                        task();
                    }
                })
                .expect("job worker thread");
        }
        pool
    })
}

fn push(task: Task) {
    let p = pool();
    p.queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(task);
    p.ready.notify_one();
}

/// Runs one queued task on the calling thread, if there is one.
fn help() -> bool {
    let task = pool()
        .queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop_front();
    match task {
        Some(t) => {
            t();
            true
        }
        None => false,
    }
}

struct State {
    /// Parts still running (a parallel-for has one per batch).
    remaining: AtomicUsize,
    done: Mutex<bool>,
    signal: Condvar,
}

impl State {
    fn new(parts: usize) -> Arc<Self> {
        Arc::new(Self {
            remaining: AtomicUsize::new(parts),
            done: Mutex::new(parts == 0),
            signal: Condvar::new(),
        })
    }

    fn finish_part(&self) {
        if self.remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
            *self.done.lock().unwrap_or_else(|e| e.into_inner()) = true;
            self.signal.notify_all();
        }
    }
}

/// Unity `JobHandle`: wait for scheduled work, or pass it as a dependency.
#[derive(Clone)]
pub struct JobHandle {
    states: Vec<Arc<State>>,
}

impl Default for JobHandle {
    /// A handle that is already complete (Unity `default(JobHandle)`).
    fn default() -> Self {
        Self { states: Vec::new() }
    }
}

impl JobHandle {
    /// Unity `JobHandle.IsCompleted`.
    pub fn is_completed(&self) -> bool {
        self.states
            .iter()
            .all(|s| *s.done.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Unity `JobHandle.Complete()`: waits for the work (and its dependencies), running
    /// queued jobs on this thread meanwhile.
    pub fn complete(&self) {
        for s in &self.states {
            loop {
                if *s.done.lock().unwrap_or_else(|e| e.into_inner()) {
                    break;
                }
                if help() {
                    continue;
                }
                let done = s.done.lock().unwrap_or_else(|e| e.into_inner());
                if !*done {
                    // Woken when it finishes; the timeout re-checks the queue for work to help
                    // with (jobs scheduled by other jobs).
                    let _ = s
                        .signal
                        .wait_timeout(done, std::time::Duration::from_millis(1));
                }
            }
        }
    }

    /// Unity `JobHandle.CombineDependencies`: complete when all of them are.
    pub fn combine(handles: &[JobHandle]) -> JobHandle {
        JobHandle {
            states: handles
                .iter()
                .flat_map(|h| h.states.iter().cloned())
                .collect(),
        }
    }
}

/// A scheduled job's result ([`Jobs::run`]).
pub struct JobResult<R> {
    handle: JobHandle,
    value: Arc<Mutex<Option<R>>>,
}

impl<R> JobResult<R> {
    pub fn handle(&self) -> &JobHandle {
        &self.handle
    }

    pub fn is_completed(&self) -> bool {
        self.handle.is_completed()
    }

    /// Waits and returns the result (`None` if the job panicked).
    pub fn complete(self) -> Option<R> {
        self.handle.complete();
        self.value.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

fn run_guarded(name: &str, f: impl FnOnce()) {
    if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        crate::debug_log_error!("{name}: {msg}");
    }
}

/// Unity's job scheduling (static API).
pub struct Jobs;

impl Jobs {
    /// Worker threads in the pool.
    pub fn worker_count() -> usize {
        pool().workers
    }

    /// Unity `IJob.Schedule(dependsOn)`: `job` runs on a worker once `depends_on` is done.
    pub fn schedule(job: impl FnOnce() + Send + 'static, depends_on: &JobHandle) -> JobHandle {
        let state = State::new(1);
        let s = state.clone();
        let deps = depends_on.clone();
        push(Box::new(move || {
            deps.complete();
            run_guarded("Job", job);
            s.finish_part();
        }));
        JobHandle {
            states: vec![state],
        }
    }

    /// A job with a result: `Jobs::run(|| heavy()).complete()`.
    pub fn run<R: Send + 'static>(job: impl FnOnce() -> R + Send + 'static) -> JobResult<R> {
        let value = Arc::new(Mutex::new(None));
        let v = value.clone();
        let handle = Self::schedule(
            move || {
                let r = job();
                *v.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            },
            &JobHandle::default(),
        );
        JobResult { handle, value }
    }

    /// Unity `IJobParallelFor.Schedule(length, innerloopBatchCount, dependsOn)`: `job(i)`
    /// for every `i` in `0..length`, `batch` indices per worker task.
    pub fn schedule_parallel_for(
        length: usize,
        batch: usize,
        job: impl Fn(usize) + Send + Sync + 'static,
        depends_on: &JobHandle,
    ) -> JobHandle {
        let batch = batch.max(1);
        let parts = length.div_ceil(batch);
        let state = State::new(parts);
        let job = Arc::new(job);
        for p in 0..parts {
            let (s, job, deps) = (state.clone(), job.clone(), depends_on.clone());
            push(Box::new(move || {
                deps.complete();
                let range = p * batch..((p + 1) * batch).min(length);
                run_guarded("Parallel job", || range.for_each(|i| job(i)));
                s.finish_part();
            }));
        }
        JobHandle {
            states: vec![state],
        }
    }

    /// `job(i)` for every `i` in `0..length` across the workers, returning when all are done
    /// — so `job` may borrow local data.
    pub fn parallel_for(length: usize, batch: usize, job: &(dyn Fn(usize) + Sync)) {
        if length == 0 {
            return;
        }
        // SAFETY: the closure only lives as long as this call: every task using it is finished
        // (`complete` returns only when the count reaches zero, panics included) before we
        // return, so extending its lifetime to 'static is never observed.
        let job: &'static (dyn Fn(usize) + Sync) = unsafe { std::mem::transmute(job) };
        Self::schedule_parallel_for(length, batch, job, &JobHandle::default()).complete();
    }

    /// Runs `job(index, item)` on every element of `items` in parallel and waits (Unity's
    /// parallel-for over a `NativeArray`).
    pub fn par_for_each_mut<T: Send>(
        items: &mut [T],
        batch: usize,
        job: impl Fn(usize, &mut T) + Sync,
    ) {
        struct Ptr<T>(*mut T);
        // SAFETY: each index is visited by exactly one task, so no two tasks touch the same
        // element; the slice outlives the call (`parallel_for` waits).
        unsafe impl<T: Send> Sync for Ptr<T> {}
        let base = Ptr(items.as_mut_ptr());
        let base = &base;
        Self::parallel_for(items.len(), batch, &|i| {
            // SAFETY: see `Ptr`; `i < items.len()`.
            let item = unsafe { &mut *base.0.add(i) };
            job(i, item);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn jobs_run_with_dependencies_and_results() {
        // A chain: b waits for a.
        let log = Arc::new(Mutex::new(Vec::new()));
        let (l1, l2) = (log.clone(), log.clone());
        let a = Jobs::schedule(
            move || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                l1.lock().unwrap().push("a");
            },
            &JobHandle::default(),
        );
        let b = Jobs::schedule(move || l2.lock().unwrap().push("b"), &a);
        b.complete();
        assert!(a.is_completed() && b.is_completed());
        assert_eq!(*log.lock().unwrap(), ["a", "b"]);
        // A result.
        assert_eq!(Jobs::run(|| (1..=100).sum::<u32>()).complete(), Some(5050));
        // A panicking job completes (and reports nothing).
        assert_eq!(Jobs::run(|| -> u32 { panic!("boom") }).complete(), None);
    }

    #[test]
    fn parallel_for_covers_every_index_once() {
        let sum = Arc::new(AtomicU64::new(0));
        let s = sum.clone();
        let h = Jobs::schedule_parallel_for(
            10_000,
            64,
            move |i| {
                s.fetch_add(i as u64, Ordering::Relaxed);
            },
            &JobHandle::default(),
        );
        let other = Jobs::schedule(|| {}, &JobHandle::default());
        JobHandle::combine(&[h, other]).complete();
        assert_eq!(sum.load(Ordering::Relaxed), (0..10_000u64).sum::<u64>());
        // Borrowing local data.
        let mut v: Vec<f32> = (0..5_000).map(|i| i as f32).collect();
        Jobs::par_for_each_mut(&mut v, 128, |i, x| *x = *x * 2.0 + i as f32);
        assert!(v.iter().enumerate().all(|(i, x)| *x == i as f32 * 3.0));
        let input: Vec<u32> = (0..1000).collect();
        let total = AtomicU64::new(0);
        Jobs::parallel_for(input.len(), 50, &|i| {
            total.fetch_add(input[i] as u64, Ordering::Relaxed);
        });
        assert_eq!(total.load(Ordering::Relaxed), 499_500);
        // Jobs that schedule and wait for other jobs don't deadlock the pool.
        let outer: Vec<JobResult<u64>> = (0..Jobs::worker_count() * 3)
            .map(|_| {
                Jobs::run(|| {
                    let inner = Jobs::run(|| 7u64);
                    inner.complete().unwrap()
                })
            })
            .collect();
        assert!(outer.into_iter().all(|r| r.complete() == Some(7)));
    }
}
