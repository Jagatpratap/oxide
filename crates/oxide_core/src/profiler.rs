//! Unity's Profiler API: samples (`Profiler.BeginSample` / `EndSample`, `ProfilerMarker`),
//! counters per frame (rendering, physics, audio, memory...) and a history of recorded
//! frames for the Profiler window. Recording is off until [`Profiler::set_enabled`];
//! disabled samples cost one atomic load.
//!
//! ```ignore
//! let _s = oxide_core::profile_scope!("AI.Plan");
//! static PATH: ProfilerMarker = ProfilerMarker::new("Pathfinding");
//! let _p = PATH.auto();
//! ```
//! Reference: <https://docs.unity3d.com/ScriptReference/Profiling.Profiler.html>

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// One timed region.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub name: Cow<'static, str>,
    /// Milliseconds from the frame's start.
    pub start: f64,
    pub duration: f64,
    /// Nesting depth on its thread (0 = top level).
    pub depth: u16,
    /// 0 = the main thread.
    pub thread: u16,
}

/// A counter value recorded in a frame (Unity's Profiler modules' charts).
#[derive(Clone, Debug, PartialEq)]
pub struct Counter {
    /// The Profiler module: "Rendering", "Physics", "Audio", "Memory", ...
    pub category: &'static str,
    pub name: Cow<'static, str>,
    pub value: f64,
}

/// One recorded frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameData {
    pub index: u64,
    /// Main thread time of the frame (ms).
    pub cpu_ms: f64,
    pub samples: Vec<Sample>,
    pub counters: Vec<Counter>,
    /// GPU time per render pass (ms), when the GPU supports timestamps. Arrives a couple of
    /// frames late.
    pub gpu: Vec<(String, f64)>,
    /// Thread names by index.
    pub threads: Vec<String>,
}

impl FrameData {
    pub fn counter(&self, category: &str, name: &str) -> Option<f64> {
        self.counters
            .iter()
            .find(|c| c.category == category && c.name == name)
            .map(|c| c.value)
    }

    /// Total GPU time (ms).
    pub fn gpu_ms(&self) -> f64 {
        self.gpu.iter().map(|(_, t)| t).fold(0.0, |a, b| a + b)
    }

    /// Unity's Hierarchy view: samples merged by path, with total and self time and calls.
    pub fn hierarchy(&self, thread: u16) -> Vec<HierarchyRow> {
        let mut rows: Vec<HierarchyRow> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut samples: Vec<&Sample> =
            self.samples.iter().filter(|s| s.thread == thread).collect();
        samples.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.depth.cmp(&b.depth)));
        for s in samples {
            // The ancestors are the open samples above this depth.
            while stack.len() > s.depth as usize {
                stack.pop();
            }
            let parent = stack.last().copied();
            let existing = rows
                .iter()
                .position(|r| r.parent == parent && r.name == s.name);
            let row = match existing {
                Some(i) => {
                    rows[i].total += s.duration;
                    rows[i].calls += 1;
                    i
                }
                None => {
                    rows.push(HierarchyRow {
                        name: s.name.to_string(),
                        parent,
                        depth: s.depth,
                        total: s.duration,
                        self_time: 0.0,
                        calls: 1,
                    });
                    rows.len() - 1
                }
            };
            stack.push(row);
        }
        // Self time: total minus the children's totals.
        let child_totals: Vec<f64> = (0..rows.len())
            .map(|i| {
                rows.iter()
                    .filter(|r| r.parent == Some(i))
                    .map(|r| r.total)
                    .sum()
            })
            .collect();
        for (i, r) in rows.iter_mut().enumerate() {
            r.self_time = (r.total - child_totals[i]).max(0.0);
        }
        rows
    }
}

/// A row of the Hierarchy view.
#[derive(Clone, Debug, PartialEq)]
pub struct HierarchyRow {
    pub name: String,
    pub parent: Option<usize>,
    pub depth: u16,
    pub total: f64,
    pub self_time: f64,
    pub calls: u32,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static NEXT_THREAD: AtomicU16 = AtomicU16::new(1);
static FRAME: AtomicU64 = AtomicU64::new(0);

/// Frames kept for the Profiler window.
pub const HISTORY: usize = 300;

struct State {
    frame_start: Instant,
    current: FrameData,
    history: VecDeque<Arc<FrameData>>,
    threads: Vec<String>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(State {
            frame_start: Instant::now(),
            current: FrameData::default(),
            history: VecDeque::new(),
            threads: vec!["Main Thread".into()],
        })
    })
}

/// A thread's index and its open samples (name and start).
type ThreadSamples = (Option<u16>, Vec<(Cow<'static, str>, Instant)>);

thread_local! {
    static LOCAL: RefCell<ThreadSamples> = const { RefCell::new((None, Vec::new())) };
    static MAIN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn thread_index() -> u16 {
    LOCAL.with(|l| {
        let mut l = l.borrow_mut();
        if let Some(i) = l.0 {
            return i;
        }
        let i = if MAIN.with(|m| m.get()) {
            0
        } else {
            let i = NEXT_THREAD.fetch_add(1, Ordering::Relaxed);
            let name = std::thread::current()
                .name()
                .map_or_else(|| format!("Thread {i}"), str::to_owned);
            let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
            if s.threads.len() <= i as usize {
                s.threads.resize(i as usize + 1, String::new());
            }
            s.threads[i as usize] = name;
            i
        };
        l.0 = Some(i);
        i
    })
}

/// Unity `Profiler` (static API).
pub struct Profiler;

impl Profiler {
    /// Unity `Profiler.enabled`: whether samples and counters are recorded.
    pub fn set_enabled(on: bool) {
        ENABLED.store(on, Ordering::Relaxed);
    }

    pub fn enabled() -> bool {
        ENABLED.load(Ordering::Relaxed)
    }

    /// Unity `Profiler.BeginSample`.
    pub fn begin_sample(name: impl Into<Cow<'static, str>>) {
        if !Self::enabled() {
            return;
        }
        let _ = thread_index();
        LOCAL.with(|l| l.borrow_mut().1.push((name.into(), Instant::now())));
    }

    /// Unity `Profiler.EndSample`.
    pub fn end_sample() {
        if !Self::enabled() {
            return;
        }
        let thread = thread_index();
        let Some((name, start, depth)) = LOCAL.with(|l| {
            let mut l = l.borrow_mut();
            let depth = l.1.len().saturating_sub(1) as u16;
            l.1.pop().map(|(n, s)| (n, s, depth))
        }) else {
            return;
        };
        let end = Instant::now();
        let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
        let frame_start = s.frame_start;
        let ms = |t: Instant| {
            t.checked_duration_since(frame_start)
                .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
        };
        s.current.samples.push(Sample {
            name,
            start: ms(start),
            duration: end.duration_since(start).as_secs_f64() * 1000.0,
            depth,
            thread,
        });
    }

    /// A sample that ends when the guard drops.
    pub fn scope(name: impl Into<Cow<'static, str>>) -> SampleGuard {
        let on = Self::enabled();
        if on {
            Self::begin_sample(name);
        }
        SampleGuard { on }
    }

    /// Records a counter for this frame (replacing the same counter's earlier value).
    pub fn counter(category: &'static str, name: impl Into<Cow<'static, str>>, value: f64) {
        if !Self::enabled() {
            return;
        }
        let name = name.into();
        let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
        match s
            .current
            .counters
            .iter_mut()
            .find(|c| c.category == category && c.name == name)
        {
            Some(c) => c.value = value,
            None => s.current.counters.push(Counter {
                category,
                name,
                value,
            }),
        }
    }

    /// Starts a frame (the engine calls this; the calling thread is the main thread).
    pub fn begin_frame() {
        MAIN.with(|m| m.set(true));
        LOCAL.with(|l| l.borrow_mut().0 = Some(0));
        let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
        s.frame_start = Instant::now();
        s.current = FrameData {
            index: FRAME.fetch_add(1, Ordering::Relaxed),
            ..Default::default()
        };
    }

    /// Ends the frame and keeps it in the history (when recording).
    pub fn end_frame() {
        let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
        if !Self::enabled() {
            return;
        }
        let mut f = std::mem::take(&mut s.current);
        f.cpu_ms = s.frame_start.elapsed().as_secs_f64() * 1000.0;
        f.threads = s.threads.clone();
        s.history.push_back(Arc::new(f));
        while s.history.len() > HISTORY {
            s.history.pop_front();
        }
    }

    /// Attaches GPU pass times to the recorded frame `index` (they arrive late).
    pub fn set_gpu_times(index: u64, passes: Vec<(String, f64)>) {
        let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = s.history.iter_mut().rev().find(|f| f.index == index) {
            Arc::make_mut(f).gpu = passes;
        }
    }

    /// The index of the frame being recorded.
    pub fn current_frame() -> u64 {
        state()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .current
            .index
    }

    /// Recorded frames, oldest first.
    pub fn frames() -> Vec<Arc<FrameData>> {
        state()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .history
            .iter()
            .cloned()
            .collect()
    }

    /// Clears the recorded frames.
    pub fn clear() {
        state()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .history
            .clear();
    }
}

/// Ends its sample when dropped.
pub struct SampleGuard {
    on: bool,
}

impl Drop for SampleGuard {
    fn drop(&mut self) {
        if self.on {
            Profiler::end_sample();
        }
    }
}

/// Unity `ProfilerMarker`: a named sample to use in many places.
pub struct ProfilerMarker {
    name: &'static str,
}

impl ProfilerMarker {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }
    /// Unity `ProfilerMarker.Auto()`: a sample until the guard drops.
    pub fn auto(&self) -> SampleGuard {
        Profiler::scope(self.name)
    }
    pub fn begin(&self) {
        Profiler::begin_sample(self.name);
    }
    pub fn end(&self) {
        Profiler::end_sample();
    }
}

/// `let _s = profile_scope!("Name");` — a Profiler sample for the rest of the scope.
#[macro_export]
macro_rules! profile_scope {
    ($name:expr) => {
        $crate::profiler::Profiler::scope($name)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Profiler is global: one test drives it.
    #[test]
    fn records_samples_counters_and_hierarchy() {
        Profiler::set_enabled(true);
        Profiler::clear();
        Profiler::begin_frame();
        {
            let _update = Profiler::scope("Update");
            for _ in 0..3 {
                let _s = Profiler::scope("Script.Update");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        static RENDER: ProfilerMarker = ProfilerMarker::new("Render");
        {
            let _r = RENDER.auto();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // Another thread's samples are kept apart.
        std::thread::Builder::new()
            .name("worker".into())
            .spawn(|| {
                let _s = Profiler::scope("Job");
            })
            .unwrap()
            .join()
            .unwrap();
        Profiler::counter("Rendering", "Draw Calls", 12.0);
        Profiler::counter("Rendering", "Draw Calls", 14.0);
        let index = Profiler::current_frame();
        Profiler::end_frame();
        Profiler::set_gpu_times(index, vec![("Draw Objects".into(), 1.5)]);
        let frames = Profiler::frames();
        let f = frames.last().unwrap();
        assert_eq!(f.counter("Rendering", "Draw Calls"), Some(14.0));
        assert_eq!(f.gpu_ms(), 1.5);
        assert!(f.cpu_ms >= 5.0, "{}", f.cpu_ms);
        let main: Vec<&Sample> = f.samples.iter().filter(|s| s.thread == 0).collect();
        assert_eq!(main.len(), 5);
        let job = f.samples.iter().find(|s| s.name == "Job").unwrap();
        assert_ne!(job.thread, 0);
        assert_eq!(f.threads[job.thread as usize], "worker");
        let h = f.hierarchy(0);
        let update = h.iter().position(|r| r.name == "Update").unwrap();
        let child = h.iter().find(|r| r.name == "Script.Update").unwrap();
        assert_eq!(child.parent, Some(update));
        assert_eq!(child.calls, 3);
        assert!(child.total >= 3.0);
        assert!(h[update].self_time < h[update].total);
        assert!(h.iter().any(|r| r.name == "Render" && r.parent.is_none()));
        // Disabled: nothing is kept.
        Profiler::set_enabled(false);
        Profiler::begin_frame();
        let _s = Profiler::scope("Ignored");
        Profiler::end_frame();
        assert_eq!(Profiler::frames().len(), frames.len());
    }
}
