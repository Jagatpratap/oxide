//! GPU time per render pass for the Profiler (Unity's GPU Usage module), read back a couple
//! of frames later. Graph passes whose render passes take [`GpuTimer::pass_writes`] are timed
//! from their first render pass's start to their last's end; others by the difference of
//! marker timestamps (tiny compute passes) before and after them.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const CAPACITY: u32 = 128;

/// A graph pass's marker timestamp index and its own render pass (begin, end) indices.
type PassSlots = (u32, Option<(u32, u32)>);
const SLOTS: usize = 4;

struct Slot {
    buffer: wgpu::Buffer,
    /// 0 free, 1 mapping, 2 ready.
    state: Arc<AtomicU8>,
    frame: u64,
    names: Vec<String>,
    own: Vec<(u32, u32, u32)>,
    count: u32,
}

/// Times render graph passes on the GPU (when the GPU supports timestamp queries).
pub struct GpuTimer {
    queries: wgpu::QuerySet,
    /// A one-thread no-op, so drivers don't drop the (otherwise empty) marker passes.
    noop: wgpu::ComputePipeline,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// Nanoseconds per timestamp tick.
    period: f32,
    next: Cell<u32>,
    /// `names[k]`: the pass that starts at timestamp `k` (empty: a gap, not reported).
    names: RefCell<Vec<String>>,
    /// Passes timed by their own render passes: (marker index, begin, end).
    own: RefCell<Vec<(u32, u32, u32)>>,
    /// The current graph pass: its marker index and own (begin, end), once allocated.
    current: Cell<Option<PassSlots>>,
    /// The current pass already wrote its begin timestamp.
    begun: Cell<bool>,
    frame: Cell<u64>,
    active: Cell<bool>,
    pending: Option<usize>,
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let bytes = CAPACITY as u64 * 8;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu timer noop"),
            source: wgpu::ShaderSource::Wgsl("@compute @workgroup_size(1) fn main() {}".into()),
        });
        let noop = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpu timer noop"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Some(Self {
            noop,
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gpu timer"),
                ty: wgpu::QueryType::Timestamp,
                count: CAPACITY,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu timer resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            slots: (0..SLOTS)
                .map(|_| Slot {
                    buffer: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("gpu timer readback"),
                        size: bytes,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    state: Arc::new(AtomicU8::new(0)),
                    frame: 0,
                    names: Vec::new(),
                    own: Vec::new(),
                    count: 0,
                })
                .collect(),
            period: queue.get_timestamp_period(),
            next: Cell::new(0),
            names: RefCell::new(Vec::new()),
            own: RefCell::new(Vec::new()),
            current: Cell::new(None),
            begun: Cell::new(false),
            frame: Cell::new(0),
            active: Cell::new(false),
            pending: None,
        })
    }

    /// Starts timing a view of profiler frame `frame`.
    pub fn begin(&self, frame: u64) {
        self.next.set(0);
        self.names.borrow_mut().clear();
        self.own.borrow_mut().clear();
        self.current.set(None);
        self.frame.set(frame);
        self.active.set(true);
    }

    /// A timestamp where pass `name` starts (empty: the end of a run of passes).
    pub fn mark(&self, encoder: &mut wgpu::CommandEncoder, name: &str) {
        let k = self.next.get();
        if !self.active.get() || k >= CAPACITY {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu timer"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries,
                beginning_of_pass_write_index: Some(k),
                end_of_pass_write_index: None,
            }),
        });
        pass.set_pipeline(&self.noop);
        pass.dispatch_workgroups(1, 1, 1);
        drop(pass);
        self.names.borrow_mut().push(name.to_owned());
        self.next.set(k + 1);
    }

    /// A graph pass starts (after its marker).
    pub fn begin_pass(&self, _name: &str) {
        let marker = self.next.get().saturating_sub(1);
        self.current.set(Some((marker, None)));
        self.begun.set(false);
    }

    /// Timestamp writes for the current pass's render passes.
    pub fn pass_writes(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if !self.active.get() {
            return None;
        }
        let (marker, own) = self.current.get()?;
        let (b, e) = match own {
            Some(be) => be,
            None => {
                let k = self.next.get();
                if k + 2 > CAPACITY {
                    return None;
                }
                // Two more slots; the names list keeps one entry per slot ("" = not a gap
                // start of its own).
                self.next.set(k + 2);
                self.names
                    .borrow_mut()
                    .extend([String::new(), String::new()]);
                self.own.borrow_mut().push((marker, k, k + 1));
                self.current.set(Some((marker, Some((k, k + 1)))));
                (k, k + 1)
            }
        };
        let first = !self.begun.replace(true);
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: first.then_some(b),
            end_of_pass_write_index: Some(e),
        })
    }

    /// Resolves the timestamps into a free readback buffer (before the encoder is submitted).
    pub fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let n = self.next.get();
        self.active.set(false);
        if n < 2 {
            return;
        }
        let Some(i) = self
            .slots
            .iter()
            .position(|s| s.state.load(Ordering::Acquire) == 0)
        else {
            return;
        };
        encoder.resolve_query_set(&self.queries, 0..n, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.slots[i].buffer, 0, n as u64 * 8);
        let s = &mut self.slots[i];
        s.frame = self.frame.get();
        s.names = self.names.borrow().clone();
        s.own = self.own.borrow().clone();
        s.count = n;
        s.state.store(1, Ordering::Release);
        self.pending = Some(i);
    }

    /// Starts reading the resolved timestamps back (after the submit).
    pub fn after_submit(&mut self) {
        let Some(i) = self.pending.take() else {
            return;
        };
        let s = &self.slots[i];
        let state = s.state.clone();
        s.buffer
            .slice(..s.count as u64 * 8)
            .map_async(wgpu::MapMode::Read, move |r| {
                state.store(if r.is_ok() { 2 } else { 0 }, Ordering::Release);
            });
    }

    /// Finished readbacks: (profiler frame, pass times in ms, merged by name).
    pub fn collect(&mut self, device: &wgpu::Device) -> Vec<(u64, Vec<(String, f64)>)> {
        let _ = device.poll(wgpu::PollType::Poll);
        let mut out = Vec::new();
        for s in &mut self.slots {
            if s.state.load(Ordering::Acquire) != 2 {
                continue;
            }
            let ticks: Vec<u64> = match s.buffer.slice(..s.count as u64 * 8).get_mapped_range() {
                Ok(view) => view
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| u64::from_le_bytes(*b))
                    .collect(),
                Err(_) => Vec::new(),
            };
            s.buffer.unmap();
            s.state.store(0, Ordering::Release);
            let mut passes: Vec<(String, f64)> = Vec::new();
            let markers: Vec<usize> = (0..s.names.len())
                .filter(|&k| {
                    !s.names[k].is_empty()
                        || !s.own.iter().any(|o| o.1 as usize == k || o.2 as usize == k)
                })
                .collect();
            for (mi, &k) in markers.iter().enumerate() {
                let name = &s.names[k];
                if name.is_empty() || k >= ticks.len() {
                    continue;
                }
                // Its own render pass timestamps, else the next marker's.
                let span = match s.own.iter().find(|o| o.0 as usize == k) {
                    Some(&(_, b, e)) => ticks
                        .get(e as usize)
                        .zip(ticks.get(b as usize))
                        .map(|(e, b)| e.saturating_sub(*b)),
                    None => markers
                        .get(mi + 1)
                        .and_then(|&n| ticks.get(n))
                        .map(|n| n.saturating_sub(ticks[k])),
                };
                let Some(span) = span else { continue };
                let ms = span as f64 * self.period as f64 / 1e6;
                match passes.iter_mut().find(|(n, _)| n == name) {
                    Some(p) => p.1 += ms,
                    None => passes.push((name.clone(), ms)),
                }
            }
            out.push((s.frame, passes));
        }
        out
    }
}
