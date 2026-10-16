//! Reflection probes and image-based lighting (Unity's Environment Reflections and
//! `ReflectionProbe`).
//!
//! Every probe owns a slot in one cube-map array (slot 0 is the sky). A capture renders the
//! scene into the six faces of a scratch cube, builds its mip chain, then GGX-prefilters it
//! into the slot so each mip matches a roughness (Unity's `perceptualRoughnessToMipmapLevel`).
//! The sky slot is also projected onto L2 spherical harmonics for diffuse ambient light.
//!
//! Renderers pick probes per object, like URP's probe blending: the two most important
//! probes whose boxes contain the object, blended by the blend distance (or the sky).
//!
//! References: <https://docs.unity3d.com/Manual/class-ReflectionProbe.html>,
//! <https://docs.unity3d.com/Manual/urp/lighting/reflection-probes.html>

use crate::frame::{ProbeRefresh, ReflectionProbeView};
use crate::renderer::HDR_FORMAT;
use oxide_math::Vector3;
use std::collections::HashMap;

/// Face size of every probe cubemap (Unity's default probe resolution).
pub const PROBE_SIZE: u32 = 128;
/// Mips 0..=6 map to perceptual roughness 0..=1 (Unity's `UNITY_SPECCUBE_LOD_STEPS` = 6).
pub const PROBE_MIPS: u32 = 7;
/// Cubemap slots including the sky in slot 0.
pub const MAX_PROBE_SLOTS: usize = 16;
/// Probe captures with `EveryFrame` refresh per rendered frame.
const EVERY_FRAME_BUDGET: usize = 2;
/// Captures of new or changed probes per frame (spreads the cost when a scene loads).
const CHANGED_BUDGET: usize = 4;
/// Mip of the sky slot projected onto spherical harmonics (32×32 faces).
const SH_MIP: u32 = 2;
const UNIFORM_SLOT: u64 = 256;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FilterParams {
    face: u32,
    _pad: u32,
    alpha: f32,
    src_size: f32,
}

/// One probe slot as the shaders see it.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ProbeGpu {
    /// Capture position, w = 1 for box projection.
    pub position: [f32; 4],
    /// Box min, w = intensity.
    pub box_min: [f32; 4],
    pub box_max: [f32; 4],
}

struct Slot {
    index: u32,
    /// Hash of the settings that require a new capture when they change.
    key: u64,
    requests: u32,
    captured: bool,
    seen: bool,
}

/// A captured probe usable by renderers this frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ActiveProbe {
    pub slot: u32,
    pub min: Vector3,
    pub max: Vector3,
    pub importance: i32,
    pub blend: f32,
    pub volume: f32,
}

/// A capture the renderer must perform this frame.
pub(crate) struct CaptureJob {
    pub slot: u32,
    /// `None` for the sky.
    pub probe: Option<ReflectionProbeView>,
}

pub struct ReflectionProbes {
    _array: wgpu::Texture,
    /// All slots as a cube array (bound to every lit shader).
    pub(crate) cube_array: wgpu::TextureView,
    /// Render-target views: `(slot * 6 + face) * PROBE_MIPS + mip`.
    slot_targets: Vec<wgpu::TextureView>,
    pub(crate) capture: wgpu::Texture,
    /// Render-target views of the scratch cube: `face * PROBE_MIPS + mip`.
    capture_targets: Vec<wgpu::TextureView>,
    /// Per mip: all six faces as a 2D array, for downsampling into the next mip.
    down_groups: Vec<wgpu::BindGroup>,
    prefilter_group: wgpu::BindGroup,
    down_pipeline: wgpu::RenderPipeline,
    prefilter_pipeline: wgpu::RenderPipeline,
    sh_pipeline: wgpu::ComputePipeline,
    sh_group: wgpu::BindGroup,
    pub(crate) sh_buffer: wgpu::Buffer,
    pub(crate) probe_buffer: wgpu::Buffer,
    pub(crate) sampler: wgpu::Sampler,
    slots: HashMap<u64, Slot>,
    sky_key: Option<u64>,
    round_robin: usize,
    last_time: f32,
    warned_full: bool,
    pub(crate) active: Vec<ActiveProbe>,
}

fn uniform_slot(i: u32) -> u32 {
    i * UNIFORM_SLOT as u32
}

/// Uniform slots: downsample `face` at `[0, 6)`, prefilter `(mip, face)` at
/// `6 + mip * 6 + face`.
const DOWN_BASE: u32 = 0;
const FILTER_BASE: u32 = 6;

/// Perceptual roughness stored in `mip` (inverse of Unity's
/// `mip = r * (1.7 - 0.7 r) * 6`).
pub fn mip_roughness(mip: u32) -> f32 {
    let m = mip as f32 / (PROBE_MIPS - 1) as f32;
    (1.7 - (2.89 - 2.8 * m).max(0.0).sqrt()) / 1.4
}

fn hash_f32s(h: &mut impl std::hash::Hasher, v: &[f32]) {
    for f in v {
        std::hash::Hash::hash(&f.to_bits(), h);
    }
}

impl ReflectionProbes {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let size = |layers| wgpu::Extent3d {
            width: PROBE_SIZE,
            height: PROBE_SIZE,
            depth_or_array_layers: layers,
        };
        let array = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reflection probes"),
            size: size(6 * MAX_PROBE_SLOTS as u32),
            mip_level_count: PROBE_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let cube_array = array.create_view(&wgpu::TextureViewDescriptor {
            label: Some("reflection probe cubes"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            ..Default::default()
        });
        let face_view = |tex: &wgpu::Texture, layer: u32, mip: u32| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                label: Some("cube face"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                base_mip_level: mip,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut slot_targets = Vec::new();
        for layer in 0..6 * MAX_PROBE_SLOTS as u32 {
            for mip in 0..PROBE_MIPS {
                slot_targets.push(face_view(&array, layer, mip));
            }
        }
        let capture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("probe capture cube"),
            size: size(6),
            mip_level_count: PROBE_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut capture_targets = Vec::new();
        for face in 0..6 {
            for mip in 0..PROBE_MIPS {
                capture_targets.push(face_view(&capture, face, mip));
            }
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("probe sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let fs = wgpu::ShaderStages::FRAGMENT;
        let tex_entry = |binding, dim| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fs,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: dim,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ibl filter layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: fs,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
                tex_entry(1, wgpu::TextureViewDimension::D2Array),
                tex_entry(2, wgpu::TextureViewDimension::Cube),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: fs,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ibl filter pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let cubemap = include_str!("shaders/cubemap.wgsl");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ibl.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{cubemap}\n{}", include_str!("shaders/ibl.wgsl")).into(),
            ),
        });
        let make = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let down_pipeline = make("fs_downsample");
        let prefilter_pipeline = make("fs_prefilter");

        // Filter parameters never change, so they are written once.
        let slots = FILTER_BASE + PROBE_MIPS * 6;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ibl params"),
            size: UNIFORM_SLOT * slots as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut bytes = vec![0u8; (UNIFORM_SLOT * slots as u64) as usize];
        let mut put = |slot: u32, p: FilterParams| {
            let o = uniform_slot(slot) as usize;
            bytes[o..o + 16].copy_from_slice(bytemuck::bytes_of(&p));
        };
        for face in 0..6 {
            put(
                DOWN_BASE + face,
                FilterParams {
                    face,
                    _pad: 0,
                    alpha: 0.0,
                    src_size: PROBE_SIZE as f32,
                },
            );
        }
        for mip in 0..PROBE_MIPS {
            let r = mip_roughness(mip);
            for face in 0..6 {
                put(
                    FILTER_BASE + mip * 6 + face,
                    FilterParams {
                        face,
                        _pad: 0,
                        alpha: r * r,
                        src_size: PROBE_SIZE as f32,
                    },
                );
            }
        }
        queue.write_buffer(&uniforms, 0, &bytes);

        let capture_cube = capture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("probe capture (cube)"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let capture_mip = |mip: u32| {
            capture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("probe capture mip"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                base_mip_level: mip,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let group = |faces: &wgpu::TextureView, cube: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ibl filter group"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &uniforms,
                            offset: 0,
                            size: wgpu::BufferSize::new(16),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(faces),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(cube),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        // A placeholder cube for the downsample groups (they only read `faces`), which must
        // not alias the mip being written.
        let placeholder = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("placeholder cube"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let placeholder_cube = placeholder.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let down_groups = (0..PROBE_MIPS - 1)
            .map(|m| group(&capture_mip(m), &placeholder_cube))
            .collect();
        let prefilter_group = group(&capture_mip(0), &capture_cube);

        // Spherical harmonics.
        let sh_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sh.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{cubemap}\n{}", include_str!("shaders/sh.wgsl")).into(),
            ),
        });
        let sh_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sh layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sh_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("sky sh"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("sh pipeline layout"),
                    bind_group_layouts: &[Some(&sh_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &sh_module,
            entry_point: Some("cs_sh"),
            compilation_options: Default::default(),
            cache: None,
        });
        let sh_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ambient sh"),
            size: 9 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Until the sky is captured: a neutral gray ambient.
        let mut init = [[0.0f32; 4]; 9];
        init[0] = [0.3 / 0.282_095; 4];
        queue.write_buffer(&sh_buffer, 0, bytemuck::cast_slice(&init));
        let sky_faces = array.create_view(&wgpu::TextureViewDescriptor {
            label: Some("sky sh source"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            base_array_layer: 0,
            array_layer_count: Some(6),
            base_mip_level: SH_MIP,
            mip_level_count: Some(1),
            ..Default::default()
        });
        let sh_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sh group"),
            layout: &sh_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&sky_faces),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sh_buffer.as_entire_binding(),
                },
            ],
        });
        let probe_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reflection probe data"),
            size: (MAX_PROBE_SLOTS * std::mem::size_of::<ProbeGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut sky = [ProbeGpu::default(); MAX_PROBE_SLOTS];
        sky[0].box_min[3] = 1.0;
        queue.write_buffer(&probe_buffer, 0, bytemuck::cast_slice(&sky));

        Self {
            _array: array,
            cube_array,
            slot_targets,
            capture,
            capture_targets,
            down_groups,
            prefilter_group,
            down_pipeline,
            prefilter_pipeline,
            sh_pipeline,
            sh_group,
            sh_buffer,
            probe_buffer,
            sampler,
            slots: HashMap::new(),
            sky_key: None,
            round_robin: 0,
            last_time: f32::NAN,
            warned_full: false,
            active: Vec::new(),
        }
    }

    /// Decides what to capture this frame: the sky when the environment changed (`sky_key`),
    /// then probes that are new, changed, requested, or refreshed every frame.
    pub(crate) fn plan(
        &mut self,
        queue: &wgpu::Queue,
        sky_key: u64,
        probes: &[ReflectionProbeView],
        time: f32,
    ) -> Vec<CaptureJob> {
        let mut jobs = Vec::new();
        if self.sky_key != Some(sky_key) {
            self.sky_key = Some(sky_key);
            jobs.push(CaptureJob {
                slot: 0,
                probe: None,
            });
        }
        // Several render calls can share one frame (Scene and Game views): refresh
        // EveryFrame probes once per frame time.
        let new_frame = time.to_bits() != self.last_time.to_bits();
        self.last_time = time;

        for s in self.slots.values_mut() {
            s.seen = false;
        }
        let mut changed = 0;
        let mut every_frame = Vec::new();
        for p in probes {
            if !self.slots.contains_key(&p.id) {
                let used: Vec<u32> = self.slots.values().map(|s| s.index).collect();
                let Some(free) = (1..MAX_PROBE_SLOTS as u32).find(|i| !used.contains(i)) else {
                    if !self.warned_full {
                        self.warned_full = true;
                        oxide_core::debug_log_warning!(
                            "More than {} reflection probes are active; extra probes are ignored",
                            MAX_PROBE_SLOTS - 1
                        );
                    }
                    continue;
                };
                self.slots.insert(
                    p.id,
                    Slot {
                        index: free,
                        key: 0,
                        requests: p.render_requests,
                        captured: false,
                        seen: true,
                    },
                );
            }
            let slot = self.slots.get_mut(&p.id).expect("inserted");
            slot.seen = true;
            let key = probe_key(p);
            // `RenderProbe()` captures in every refresh mode.
            let requested = slot.requests != p.render_requests;
            let wanted = requested
                || match p.refresh {
                    ProbeRefresh::OnAwake => !slot.captured || slot.key != key,
                    ProbeRefresh::ViaScripting => false,
                    ProbeRefresh::EveryFrame => {
                        if !slot.captured {
                            true
                        } else {
                            if new_frame {
                                every_frame.push(p.clone());
                            }
                            false
                        }
                    }
                };
            if wanted && changed < CHANGED_BUDGET {
                changed += 1;
                slot.key = key;
                slot.requests = p.render_requests;
                slot.captured = true;
                jobs.push(CaptureJob {
                    slot: slot.index,
                    probe: Some(p.clone()),
                });
            }
        }
        if !every_frame.is_empty() {
            for k in 0..EVERY_FRAME_BUDGET.min(every_frame.len()) {
                let p = &every_frame[(self.round_robin + k) % every_frame.len()];
                let slot = &self.slots[&p.id];
                jobs.push(CaptureJob {
                    slot: slot.index,
                    probe: Some(p.clone()),
                });
            }
            self.round_robin = self.round_robin.wrapping_add(EVERY_FRAME_BUDGET);
        }
        self.slots.retain(|_, s| s.seen);

        // Upload slot data and the list renderers choose from.
        let mut data = [ProbeGpu::default(); MAX_PROBE_SLOTS];
        data[0].box_min[3] = 1.0;
        self.active.clear();
        for p in probes {
            let Some(s) = self.slots.get(&p.id).filter(|s| s.captured) else {
                continue;
            };
            data[s.index as usize] = ProbeGpu {
                position: p.position.extend(p.box_projection as u32 as f32).to_array(),
                box_min: p.box_min.extend(p.intensity.max(0.0)).to_array(),
                box_max: p.box_max.extend(0.0).to_array(),
            };
            let size = (p.box_max - p.box_min).max(Vector3::ZERO);
            self.active.push(ActiveProbe {
                slot: s.index,
                min: p.box_min,
                max: p.box_max,
                importance: p.importance,
                blend: p.blend_distance.max(0.0),
                volume: size.x * size.y * size.z,
            });
        }
        queue.write_buffer(&self.probe_buffer, 0, bytemuck::cast_slice(&data));
        jobs
    }

    pub(crate) fn active_count(&self) -> u32 {
        self.active.len() as u32
    }

    /// Probe selection for an object at `p`, packed for the instance data:
    /// `(slot_a | slot_b << 16, weight_a bits)`. Slot 0 is the sky.
    pub(crate) fn select(&self, p: Vector3, exclude: Option<u32>) -> [u32; 2] {
        let mut best: [Option<(&ActiveProbe, f32)>; 2] = [None, None];
        let better = |a: &ActiveProbe, b: &ActiveProbe| {
            a.importance > b.importance || (a.importance == b.importance && a.volume < b.volume)
        };
        for probe in &self.active {
            if Some(probe.slot) == exclude || p.cmplt(probe.min).any() || p.cmpgt(probe.max).any() {
                continue;
            }
            // Weight fades from 1 inside to 0 at the box faces over the blend distance.
            let inside = (p - probe.min).min(probe.max - p).min_element();
            let w = if probe.blend > 0.0 {
                (inside / probe.blend).clamp(0.0, 1.0)
            } else {
                1.0
            };
            match best {
                [None, _] => best[0] = Some((probe, w)),
                [Some((a, _)), _] if better(probe, a) => {
                    best[1] = best[0];
                    best[0] = Some((probe, w));
                }
                [_, None] => best[1] = Some((probe, w)),
                [_, Some((b, _))] if better(probe, b) => best[1] = Some((probe, w)),
                _ => {}
            }
        }
        match best {
            [None, _] => [0, 0.0f32.to_bits()],
            [Some((a, w)), b] => {
                let b_slot = b.map_or(0, |(b, _)| b.slot);
                [a.slot | (b_slot << 16), w.to_bits()]
            }
        }
    }

    /// The scratch cube's mip-0 face `face` (copy destination for a capture).
    pub(crate) fn capture_texture(&self) -> &wgpu::Texture {
        &self.capture
    }

    /// Builds the scratch cube's mips, then prefilters it into `slot` (and refreshes the
    /// ambient SH for the sky slot).
    pub(crate) fn filter(&self, encoder: &mut wgpu::CommandEncoder, slot: u32) {
        let pass = |encoder: &mut wgpu::CommandEncoder,
                    target: &wgpu::TextureView,
                    pipeline: &wgpu::RenderPipeline,
                    group: &wgpu::BindGroup,
                    offset: u32| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ibl filter"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[offset]);
            pass.draw(0..3, 0..1);
        };
        for mip in 1..PROBE_MIPS {
            for face in 0..6 {
                pass(
                    encoder,
                    &self.capture_targets[(face * PROBE_MIPS + mip) as usize],
                    &self.down_pipeline,
                    &self.down_groups[(mip - 1) as usize],
                    uniform_slot(DOWN_BASE + face),
                );
            }
        }
        for mip in 0..PROBE_MIPS {
            for face in 0..6 {
                pass(
                    encoder,
                    &self.slot_targets[((slot * 6 + face) * PROBE_MIPS + mip) as usize],
                    &self.prefilter_pipeline,
                    &self.prefilter_group,
                    uniform_slot(FILTER_BASE + mip * 6 + face),
                );
            }
        }
        if slot == 0 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sky sh"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.sh_pipeline);
            pass.set_bind_group(0, &self.sh_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }
}

fn probe_key(p: &ReflectionProbeView) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    hash_f32s(
        &mut h,
        &[p.position.x, p.position.y, p.position.z, p.near, p.far],
    );
    p.culling_mask.hash(&mut h);
    p.baked.hash(&mut h);
    match p.clear {
        crate::ClearMode::Color(c) => hash_f32s(&mut h, &[c.r, c.g, c.b]),
        other => std::mem::discriminant(&other).hash(&mut h),
    }
    h.finish()
}

/// Rotations of the six cube-face cameras (+X, -X, +Y, -Y, +Z, -Z), each looking along its
/// axis with the up vector of the cube-map convention.
pub fn face_rotations() -> [oxide_math::Quaternion; 6] {
    use oxide_math::QuaternionExt;
    let q = |f: Vector3, u: Vector3| oxide_math::Quaternion::look_rotation(f, u);
    [
        q(Vector3::X, Vector3::Y),
        q(Vector3::NEG_X, Vector3::Y),
        q(Vector3::Y, Vector3::NEG_Z),
        q(Vector3::NEG_Y, Vector3::Z),
        q(Vector3::Z, Vector3::Y),
        q(Vector3::NEG_Z, Vector3::Y),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_roughness_inverts_unitys_mapping() {
        for mip in 0..PROBE_MIPS {
            let r = mip_roughness(mip);
            let back = r * (1.7 - 0.7 * r) * 6.0;
            assert!(
                (back - mip as f32).abs() < 1e-4,
                "mip {mip}: r {r} -> {back}"
            );
        }
        assert_eq!(mip_roughness(0), 0.0);
        assert!((mip_roughness(PROBE_MIPS - 1) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn face_cameras_match_cube_convention() {
        // Camera right = cross(up, forward) in Unity's left-handed space; the cube map's
        // texel u axis must point the same way.
        let right = [
            Vector3::NEG_Z,
            Vector3::Z,
            Vector3::X,
            Vector3::X,
            Vector3::X,
            Vector3::NEG_X,
        ];
        for (i, r) in face_rotations().iter().enumerate() {
            let cam_right = *r * Vector3::X;
            assert!(
                cam_right.distance(right[i]) < 1e-4,
                "face {i}: {cam_right:?} vs {:?}",
                right[i]
            );
        }
    }
}
