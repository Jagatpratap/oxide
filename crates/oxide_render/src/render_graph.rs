//! A render graph (Unity's RenderGraph API): passes declare the textures they read and
//! write; the graph orders them, culls passes whose results nothing uses, and allocates
//! transient textures from a pool, reusing one texture for transients whose lifetimes don't
//! overlap. <https://docs.unity3d.com/Manual/urp/render-graph.html>
//!
//! ```ignore
//! let mut g = RenderGraph::new();
//! let color = g.import("camera color", view, desc);
//! let blurred = g.create(desc_half);
//! g.add_pass("blur", |b| { b.read(color); b.write(blurred); }, move |ctx| { /* encode */ });
//! g.add_pass("composite", |b| { b.read(blurred); b.write(color); }, move |ctx| { /* ... */ });
//! g.execute(&device, &queue, &mut encoder, &mut pool);
//! ```

use std::collections::HashMap;

/// A texture in a graph (imported or transient).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureHandle(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    pub usage: wgpu::TextureUsages,
}

impl TextureDesc {
    /// A color target that later passes can sample.
    pub fn color(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        }
    }
}

enum Resource {
    Imported(wgpu::TextureView),
    Transient(TextureDesc),
}

/// Declares what a pass reads and writes.
#[derive(Default)]
pub struct PassBuilder {
    reads: Vec<TextureHandle>,
    writes: Vec<TextureHandle>,
    side_effects: bool,
}

impl PassBuilder {
    pub fn read(&mut self, t: TextureHandle) -> &mut Self {
        self.reads.push(t);
        self
    }

    pub fn write(&mut self, t: TextureHandle) -> &mut Self {
        self.writes.push(t);
        self
    }

    /// Never cull this pass (it does something outside the graph, e.g. a readback).
    pub fn side_effects(&mut self) -> &mut Self {
        self.side_effects = true;
        self
    }
}

/// What a pass gets when it runs.
pub struct PassContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    views: &'a [Option<wgpu::TextureView>],
    timer: Option<&'a crate::gpu_timer::GpuTimer>,
}

impl<'a> PassContext<'a> {
    /// Timestamp writes for this pass's render pass(es), when the Profiler times the GPU:
    /// pass them as `RenderPassDescriptor::timestamp_writes` (several render passes in one
    /// graph pass are timed from the first's start to the last's end).
    pub fn timestamp_writes(&self) -> Option<wgpu::RenderPassTimestampWrites<'a>> {
        self.timer.and_then(|t| t.pass_writes())
    }

    /// The texture behind `t` (only valid for handles the pass declared). Views are cheap
    /// reference-counted handles, so this returns its own.
    pub fn view(&self, t: TextureHandle) -> wgpu::TextureView {
        self.views[t.0 as usize]
            .clone()
            .expect("texture not allocated: declare it with read/write")
    }
}

type Exec<'a> = Box<dyn FnOnce(&mut PassContext) + 'a>;

struct Pass<'a> {
    name: String,
    decl: PassBuilder,
    exec: Exec<'a>,
}

/// The order passes run in and which transient textures share memory.
#[derive(Debug, PartialEq)]
pub struct Compiled {
    /// Indices of passes that run, in order.
    pub order: Vec<usize>,
    /// Names of culled passes.
    pub culled: Vec<String>,
    /// Names of the passes that run, in order (the Frame Debugger's event list).
    pub ran: Vec<String>,
    /// For each transient resource that is used: which physical slot it lives in.
    pub slots: HashMap<u32, usize>,
    /// Number of physical textures needed for transients.
    pub physical: usize,
}

#[derive(Default)]
pub struct RenderGraph<'a> {
    resources: Vec<Resource>,
    passes: Vec<Pass<'a>>,
    timer: Option<&'a crate::gpu_timer::GpuTimer>,
}

/// Transient textures kept between frames, by description.
#[derive(Default)]
pub struct TexturePool {
    free: HashMap<TextureDesc, Vec<wgpu::TextureView>>,
    /// Textures created so far (for stats and tests).
    pub created: usize,
}

impl TexturePool {
    fn take(&mut self, device: &wgpu::Device, d: TextureDesc) -> wgpu::TextureView {
        if let Some(v) = self.free.get_mut(&d).and_then(Vec::pop) {
            return v;
        }
        self.created += 1;
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("render graph transient"),
                size: wgpu::Extent3d {
                    width: d.width,
                    height: d.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: d.format,
                usage: d.usage,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    fn give(&mut self, d: TextureDesc, v: wgpu::TextureView) {
        self.free.entry(d).or_default().push(v);
    }
}

impl<'a> RenderGraph<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Times each pass on the GPU (the Profiler's GPU module).
    pub fn set_timer(&mut self, timer: Option<&'a crate::gpu_timer::GpuTimer>) {
        self.timer = timer;
    }

    /// A texture that lives outside the graph (camera targets, history buffers).
    pub fn import(&mut self, view: &wgpu::TextureView) -> TextureHandle {
        self.resources.push(Resource::Imported(view.clone()));
        TextureHandle(self.resources.len() as u32 - 1)
    }

    /// A texture that only lives during this graph's execution.
    pub fn create(&mut self, desc: TextureDesc) -> TextureHandle {
        self.resources.push(Resource::Transient(desc));
        TextureHandle(self.resources.len() as u32 - 1)
    }

    pub fn add_pass(
        &mut self,
        name: &str,
        declare: impl FnOnce(&mut PassBuilder),
        exec: impl FnOnce(&mut PassContext) + 'a,
    ) {
        let mut decl = PassBuilder::default();
        declare(&mut decl);
        self.passes.push(Pass {
            name: name.to_owned(),
            decl,
            exec: Box::new(exec),
        });
    }

    fn imported(&self, t: TextureHandle) -> bool {
        matches!(self.resources[t.0 as usize], Resource::Imported(_))
    }

    /// Culls passes whose writes nobody needs (imported textures and side effects are
    /// always needed), then assigns transients to physical textures by lifetime.
    pub fn compile(&self) -> Compiled {
        let n = self.passes.len();
        let mut needed = vec![false; n];
        let mut wanted: std::collections::HashSet<TextureHandle> = Default::default();
        for i in (0..n).rev() {
            let d = &self.passes[i].decl;
            let useful = d.side_effects
                || d.writes
                    .iter()
                    .any(|w| self.imported(*w) || wanted.contains(w));
            if useful {
                needed[i] = true;
                wanted.extend(d.reads.iter().copied());
            }
        }
        let order: Vec<usize> = (0..n).filter(|&i| needed[i]).collect();
        let culled = (0..n)
            .filter(|&i| !needed[i])
            .map(|i| self.passes[i].name.clone())
            .collect();

        // Lifetime (first and last position in `order`) of each used transient.
        let mut life: HashMap<u32, (usize, usize)> = HashMap::new();
        for (pos, &i) in order.iter().enumerate() {
            let d = &self.passes[i].decl;
            for t in d.reads.iter().chain(&d.writes) {
                if let Resource::Transient(_) = self.resources[t.0 as usize] {
                    let e = life.entry(t.0).or_insert((pos, pos));
                    e.1 = pos;
                }
            }
        }
        // Greedy interval assignment per description: a slot frees after its last use.
        let mut ids: Vec<u32> = life.keys().copied().collect();
        ids.sort_by_key(|id| (life[id].0, *id));
        let mut slots = HashMap::new();
        let mut slot_desc: Vec<(TextureDesc, usize)> = Vec::new(); // (desc, free from pos)
        for id in ids {
            let Resource::Transient(desc) = self.resources[id as usize] else {
                unreachable!()
            };
            let (first, last) = life[&id];
            let slot = slot_desc
                .iter()
                .position(|(d, free_at)| *d == desc && *free_at <= first)
                .unwrap_or_else(|| {
                    slot_desc.push((desc, 0));
                    slot_desc.len() - 1
                });
            slot_desc[slot].1 = last + 1;
            slots.insert(id, slot);
        }
        let ran = order.iter().map(|&i| self.passes[i].name.clone()).collect();
        Compiled {
            order,
            ran,
            culled,
            slots,
            physical: slot_desc.len(),
        }
    }

    /// Compiles and records every surviving pass into `encoder`.
    pub fn execute(
        self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TexturePool,
    ) -> Compiled {
        let compiled = self.compile();
        // Physical textures for the slots.
        let mut slot_descs: Vec<Option<TextureDesc>> = vec![None; compiled.physical];
        for (id, slot) in &compiled.slots {
            if let Resource::Transient(d) = self.resources[*id as usize] {
                slot_descs[*slot] = Some(d);
            }
        }
        let physical: Vec<(TextureDesc, wgpu::TextureView)> = slot_descs
            .into_iter()
            .map(|d| {
                let d = d.expect("every slot has a resource");
                (d, pool.take(device, d))
            })
            .collect();
        let views: Vec<Option<wgpu::TextureView>> = self
            .resources
            .iter()
            .enumerate()
            .map(|(i, r)| match r {
                Resource::Imported(v) => Some(v.clone()),
                Resource::Transient(_) => compiled
                    .slots
                    .get(&(i as u32))
                    .map(|s| physical[*s].1.clone()),
            })
            .collect();
        let mut passes: Vec<Option<Pass>> = self.passes.into_iter().map(Some).collect();
        let timer = self.timer;
        for &i in &compiled.order {
            let pass = passes[i].take().expect("each pass runs once");
            if let Some(t) = timer {
                t.mark(encoder, &pass.name);
            }
            if let Some(t) = timer {
                t.begin_pass(&pass.name);
            }
            let mut ctx = PassContext {
                device,
                queue,
                encoder,
                views: &views,
                timer,
            };
            (pass.exec)(&mut ctx);
        }
        if let Some(t) = timer {
            t.mark(encoder, "");
        }
        for (d, v) in physical {
            pool.give(d, v);
        }
        compiled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> TextureDesc {
        TextureDesc::color(64, 64, wgpu::TextureFormat::Rgba16Float)
    }

    #[test]
    fn unused_passes_are_culled() {
        let mut g = RenderGraph::new();
        let a = g.create(desc());
        let b = g.create(desc());
        g.add_pass(
            "make a",
            |p| {
                p.write(a);
            },
            |_| {},
        );
        g.add_pass(
            "make b (unused)",
            |p| {
                p.write(b);
            },
            |_| {},
        );
        g.add_pass(
            "present a",
            |p| {
                p.read(a).side_effects();
            },
            |_| {},
        );
        let c = g.compile();
        assert_eq!(c.order, vec![0, 2]);
        assert_eq!(c.culled, vec!["make b (unused)".to_owned()]);
    }

    #[test]
    fn transients_with_disjoint_lifetimes_share_a_texture() {
        let mut g = RenderGraph::new();
        let a = g.create(desc());
        let b = g.create(desc());
        let c = g.create(desc());
        // a → b → c: a is dead once b is written, so c can reuse a's texture.
        g.add_pass(
            "1",
            |p| {
                p.write(a);
            },
            |_| {},
        );
        g.add_pass(
            "2",
            |p| {
                p.read(a).write(b);
            },
            |_| {},
        );
        g.add_pass(
            "3",
            |p| {
                p.read(b).write(c);
            },
            |_| {},
        );
        g.add_pass(
            "out",
            |p| {
                p.read(c).side_effects();
            },
            |_| {},
        );
        let compiled = g.compile();
        assert_eq!(compiled.physical, 2, "{:?}", compiled.slots);
        assert_eq!(compiled.slots[&a.0], compiled.slots[&c.0]);
        assert_ne!(compiled.slots[&a.0], compiled.slots[&b.0]);
    }

    #[test]
    fn different_descriptions_never_alias() {
        let mut g = RenderGraph::new();
        let a = g.create(desc());
        let half = g.create(TextureDesc::color(32, 32, wgpu::TextureFormat::Rgba16Float));
        g.add_pass(
            "1",
            |p| {
                p.write(a);
            },
            |_| {},
        );
        g.add_pass(
            "2",
            |p| {
                p.read(a).side_effects();
            },
            |_| {},
        );
        g.add_pass(
            "3",
            |p| {
                p.write(half);
            },
            |_| {},
        );
        g.add_pass(
            "4",
            |p| {
                p.read(half).side_effects();
            },
            |_| {},
        );
        assert_eq!(g.compile().physical, 2);
    }
}
