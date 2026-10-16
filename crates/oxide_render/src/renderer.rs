//! The built-in forward renderer (Oxide's URP equivalent).
//!
//! Per frame: upload changed assets → cull and batch draws per camera (GPU instancing for
//! repeated mesh+material pairs) → 4 cascaded sun shadow maps → for each camera an MSAA HDR
//! pass (clear/sky, opaque front-to-back, transparent back-to-front, debug lines) →
//! bloom and tonemapping into the target.
//!
//! References: <https://docs.unity3d.com/Manual/urp/rendering-in-universalrp.html>,
//! <https://docs.unity3d.com/Manual/shadow-cascades.html>

use crate::Gpu;
use crate::frame::*;
use crate::gpu_assets::{GpuAssets, SkinVertex, Vertex};
use crate::post::PostProcessor;
use oxide_assets::{AssetId, AssetServer};
use oxide_math::{Matrix4x4, Matrix4x4Ext, Vector3, Vector4};
use std::collections::HashMap;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const CASCADES: usize = 4;
const MAX_LIGHTS: usize = 256;
/// Occluder triangles rasterized per camera per frame at most.
const OCCLUDER_TRIANGLE_BUDGET: u32 = 60_000;
/// Meshes with more triangles than this don't occlude (use a simpler LOD as the occluder).
const MAX_OCCLUDER_TRIANGLES: u32 = 6_000;
/// Occluders smaller than this (bounding radius / distance) are skipped.
const MIN_OCCLUDER_SIZE: f32 = 0.02;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    view_proj: [f32; 16],
    view: [f32; 16],
    inv_view_proj: [f32; 16],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    shadow_params: [f32; 4],
    cascade_splits: [f32; 4],
    cascade_vp: [[f32; 16]; CASCADES],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    ground_color: [f32; 4],
    fog_color: [f32; 4],
    fog_params: [f32; 4],
    misc: [f32; 4],
    clear_color: [f32; 4],
    cascade_texel: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightUniform {
    pos_range: [f32; 4],
    color: [f32; 4],
    dir: [f32; 4],
    spot: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    model: [f32; 16],
    normal: [[f32; 4]; 3],
    extra: [u32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
struct MainKey {
    transparent: bool,
    double_sided: bool,
    skinned: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct ShadowKey {
    cutout: bool,
    double_sided: bool,
    skinned: bool,
}

struct Batch {
    key: MainKey,
    material: AssetId,
    mesh: AssetId,
    submesh: u32,
    instances: std::ops::Range<u32>,
}

struct ShadowBatch {
    key: ShadowKey,
    material: AssetId,
    mesh: AssetId,
    submesh: u32,
    instances: std::ops::Range<u32>,
}

struct Targets {
    last_used: u64,
    bloom: Vec<(wgpu::TextureView, u32, u32)>,
    msaa: Option<wgpu::TextureView>,
    resolve: wgpu::TextureView,
    depth: wgpu::TextureView,
}

fn grow_buffer(
    device: &wgpu::Device,
    buf: &mut wgpu::Buffer,
    needed: u64,
    usage: wgpu::BufferUsages,
    label: &str,
) -> bool {
    if buf.size() >= needed {
        return false;
    }
    let size = needed.next_power_of_two().max(1024);
    *buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    });
    true
}

pub struct SceneRenderer {
    pub gpu_assets: GpuAssets,
    terrain: crate::terrain::TerrainGpu,
    particles: crate::particles::ParticleGpu,
    sample_count: u32,
    frame_layout: wgpu::BindGroupLayout,
    shadow_layout: wgpu::BindGroupLayout,
    main_layout: wgpu::PipelineLayout,
    shadow_pipeline_layout: wgpu::PipelineLayout,
    lit_module: wgpu::ShaderModule,
    shadow_module: wgpu::ShaderModule,
    main_pipelines: HashMap<MainKey, wgpu::RenderPipeline>,
    shadow_pipelines: HashMap<ShadowKey, wgpu::RenderPipeline>,
    sky_pipeline: wgpu::RenderPipeline,
    solid_pipeline: wgpu::RenderPipeline,
    depth_clear_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    frame_buffers: Vec<wgpu::Buffer>,
    frame_groups: Vec<wgpu::BindGroup>,
    light_buffer: wgpu::Buffer,
    joint_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    line_buffer: wgpu::Buffer,
    shadow_size: u32,
    shadow_layers: Vec<wgpu::TextureView>,
    shadow_array: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    shadow_buffers: Vec<wgpu::Buffer>,
    shadow_groups: Vec<wgpu::BindGroup>,
    groups_dirty: bool,
    /// Render targets per output size (Scene and Game views render at different sizes).
    targets: HashMap<(u32, u32), Targets>,
    frame_index: u64,
    post: PostProcessor,
    /// One occlusion buffer per camera of the last frame.
    occlusion: Vec<crate::occlusion::OcclusionBuffer>,
    pub last_stats: RenderStats,
}

fn vertex_layouts(skinned: bool) -> Vec<Option<wgpu::VertexBufferLayout<'static>>> {
    const VERTEX: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2, 4 => Float32x2, 5 => Float32x4
    ];
    const INSTANCE: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
        6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4,
        10 => Float32x4, 11 => Float32x4, 12 => Float32x4, 13 => Uint32x4
    ];
    const SKIN: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![14 => Uint16x4, 15 => Float32x4];
    let mut v = vec![
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE,
        }),
    ];
    if skinned {
        v.push(Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SkinVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &SKIN,
        }));
    }
    v
}

fn shader(device: &wgpu::Device, label: &str, src: &str) -> wgpu::ShaderModule {
    let full = format!("{}\n{}", include_str!("shaders/common.wgsl"), src);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(full.into()),
    })
}

fn storage_entry(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl SceneRenderer {
    pub fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;
        let flags = gpu.adapter.get_texture_format_features(HDR_FORMAT).flags;
        let sample_count = if flags.sample_count_supported(4) {
            4
        } else {
            1
        };
        let gpu_assets = GpuAssets::new(device, &gpu.queue);
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame layout"),
            entries: &[
                uniform_entry(0, vf),
                storage_entry(1, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                storage_entry(4, wgpu::ShaderStages::VERTEX),
            ],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow view layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX),
                storage_entry(1, wgpu::ShaderStages::VERTEX),
            ],
        });
        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("main pipeline layout"),
            bind_group_layouts: &[Some(&frame_layout), Some(&gpu_assets.material_layout)],
            immediate_size: 0,
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow pipeline layout"),
                bind_group_layouts: &[Some(&shadow_layout), Some(&gpu_assets.material_layout)],
                immediate_size: 0,
            });
        let frame_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("frame-only layout"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let lit_src = format!(
            "{}\n{}",
            include_str!("shaders/lit.wgsl"),
            include_str!("shaders/lighting.wgsl")
        );
        let lit_module = shader(device, "lit.wgsl", &lit_src);

        let shadow_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
        });
        let sky_module = shader(device, "sky.wgsl", include_str!("shaders/sky.wgsl"));
        let lines_module = shader(device, "lines.wgsl", include_str!("shaders/lines.wgsl"));

        let ms = wgpu::MultisampleState {
            count: sample_count,
            ..Default::default()
        };
        let fullscreen = |entry: &str, write_color: bool, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&frame_only),
                vertex: wgpu::VertexState {
                    module: &sky_module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                // Writes depth 0 (reversed-Z far plane): this also clears depth in the viewport.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: ms,
                fragment: Some(wgpu::FragmentState {
                    module: &sky_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: None,
                        write_mask: if write_color {
                            wgpu::ColorWrites::ALL
                        } else {
                            wgpu::ColorWrites::empty()
                        },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sky_pipeline = fullscreen("fs_main", true, "sky pipeline");
        let solid_pipeline = fullscreen("fs_solid", true, "solid clear pipeline");
        let depth_clear_pipeline = fullscreen("fs_solid", false, "depth clear pipeline");

        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("line pipeline"),
            layout: Some(&frame_only),
            vertex: wgpu::VertexState {
                module: &lines_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: ms,
            fragment: Some(wgpu::FragmentState {
                module: &lines_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let shadow_size = 2048;
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun shadow cascades"),
            size: wgpu::Extent3d {
                width: shadow_size,
                height: shadow_size,
                depth_or_array_layers: CASCADES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_layers = (0..CASCADES as u32)
            .map(|i| {
                shadow_tex.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("shadow cascade"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let shadow_array = shadow_tex.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let shadow_buffers = (0..CASCADES)
            .map(|_| buf("shadow view", 64, wgpu::BufferUsages::UNIFORM))
            .collect();
        let post = PostProcessor::new(device);
        let mut terrain = crate::terrain::TerrainGpu::new(device);
        {
            let terrain_src = format!(
                "{}\n{}",
                include_str!("shaders/terrain.wgsl"),
                include_str!("shaders/lighting.wgsl")
            );
            let module = shader(device, "terrain.wgsl", &terrain_src);
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("terrain pipeline layout"),
                bind_group_layouts: &[Some(&frame_layout), Some(&terrain.layout)],
                immediate_size: 0,
            });
            let buffers = vertex_layouts(false);
            terrain.pipeline = Some(device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("terrain pipeline"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &buffers,
                    },
                    primitive: wgpu::PrimitiveState {
                        front_face: wgpu::FrontFace::Cw,
                        cull_mode: Some(wgpu::Face::Back),
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Greater),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: ms,
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: HDR_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                },
            ));
        }
        let particle_module = shader(
            device,
            "particles.wgsl",
            include_str!("shaders/particles.wgsl"),
        );
        let particles = crate::particles::ParticleGpu::new(
            device,
            &gpu.queue,
            &frame_layout,
            &particle_module,
            sample_count,
        );
        Self {
            terrain,
            particles,
            gpu_assets,
            sample_count,
            frame_layout,
            shadow_layout,
            main_layout,
            shadow_pipeline_layout,
            lit_module,
            shadow_module,
            main_pipelines: HashMap::new(),
            shadow_pipelines: HashMap::new(),
            sky_pipeline,
            solid_pipeline,
            depth_clear_pipeline,
            line_pipeline,
            frame_buffers: Vec::new(),
            frame_groups: Vec::new(),
            light_buffer: buf(
                "lights",
                (MAX_LIGHTS * 64) as u64,
                wgpu::BufferUsages::STORAGE,
            ),
            joint_buffer: buf("joints", 64 * 256, wgpu::BufferUsages::STORAGE),
            instance_buffer: buf("instances", 128 * 1024, wgpu::BufferUsages::VERTEX),
            line_buffer: buf("lines", 28 * 1024, wgpu::BufferUsages::VERTEX),
            shadow_size,
            shadow_layers,
            shadow_array,
            shadow_sampler,
            shadow_buffers,
            shadow_groups: Vec::new(),
            groups_dirty: true,
            targets: HashMap::new(),
            frame_index: 0,
            post,
            occlusion: Vec::new(),
            last_stats: RenderStats::default(),
        }
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    fn main_pipeline(&mut self, device: &wgpu::Device, key: MainKey) -> &wgpu::RenderPipeline {
        let (module, layout, ms) = (&self.lit_module, &self.main_layout, self.sample_count);
        self.main_pipelines.entry(key).or_insert_with(|| {
            let buffers = vertex_layouts(key.skinned);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("lit {key:?}")),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(if key.skinned { "vs_skinned" } else { "vs_main" }),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: if key.double_sided {
                        None
                    } else {
                        Some(wgpu::Face::Back)
                    },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(!key.transparent),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: ms,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: Some(if key.transparent {
                            wgpu::BlendState::ALPHA_BLENDING
                        } else {
                            wgpu::BlendState::REPLACE
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    fn shadow_pipeline(&mut self, device: &wgpu::Device, key: ShadowKey) -> &wgpu::RenderPipeline {
        let (module, layout) = (&self.shadow_module, &self.shadow_pipeline_layout);
        self.shadow_pipelines.entry(key).or_insert_with(|| {
            let buffers = vertex_layouts(key.skinned);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("shadow {key:?}")),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(if key.skinned { "vs_skinned" } else { "vs_main" }),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    front_face: wgpu::FrontFace::Cw,
                    // Two-sided casting avoids light leaks through open meshes.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                }),
                multisample: Default::default(),
                fragment: key.cutout.then(|| wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_cutout"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        self.frame_index += 1;
        let now = self.frame_index;
        if let Some(t) = self.targets.get_mut(&(w, h)) {
            t.last_used = now;
            return;
        }
        // Keep a few sizes alive; drop the least recently used beyond that.
        while self.targets.len() >= 4 {
            let oldest = *self
                .targets
                .iter()
                .min_by_key(|(_, t)| t.last_used)
                .expect("non-empty")
                .0;
            self.targets.remove(&oldest);
        }
        let tex = |label: &str, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa =
            (self.sample_count > 1).then(|| tex("hdr msaa", HDR_FORMAT, self.sample_count, ra));
        let resolve = tex(
            "hdr",
            HDR_FORMAT,
            1,
            ra | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth = tex("depth", DEPTH_FORMAT, self.sample_count, ra);
        let bloom = PostProcessor::bloom_chain(device, w, h);
        self.targets.insert(
            (w, h),
            Targets {
                last_used: now,
                bloom,
                msaa,
                resolve,
                depth,
            },
        );
    }

    fn rebuild_groups(&mut self, device: &wgpu::Device, cameras: usize) {
        while self.frame_buffers.len() < cameras {
            self.frame_buffers
                .push(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("frame uniform"),
                    size: std::mem::size_of::<FrameUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            self.groups_dirty = true;
        }
        if !self.groups_dirty {
            return;
        }
        self.frame_groups = self
            .frame_buffers
            .iter()
            .map(|b| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("frame bind group"),
                    layout: &self.frame_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: b.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.light_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&self.shadow_array),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: self.joint_buffer.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        self.shadow_groups = self
            .shadow_buffers
            .iter()
            .map(|b| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shadow view bind group"),
                    layout: &self.shadow_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: b.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.joint_buffer.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        self.groups_dirty = false;
    }

    /// Renders `frame` into `target` (`width`×`height`, `format`) and submits the work.
    /// The occlusion buffer of camera `index` from the last frame (empty if that camera
    /// had occlusion culling off).
    pub fn occlusion_buffer(&self, index: usize) -> Option<&crate::occlusion::OcclusionBuffer> {
        self.occlusion.get(index).filter(|b| b.width() > 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gpu: &Gpu,
        assets: &AssetServer,
        frame: &RenderFrame,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> RenderStats {
        let device = &gpu.device;
        let queue = &gpu.queue;
        let (w, h) = (width.max(1), height.max(1));
        self.ensure_targets(device, w, h);
        let mut stats = RenderStats {
            cameras: frame.cameras.len() as u32,
            ..Default::default()
        };

        // ---- upload assets and resolve draws ------------------------------------------
        struct Resolved<'a> {
            item: &'a DrawItem,
            material: AssetId,
            center: Vector3,
            radius: f32,
            /// Mesh bounds (local).
            bounds: oxide_math::Bounds,
            transparent: bool,
            cutout: bool,
            double_sided: bool,
            cast: bool,
            priority: i32,
        }
        let mut resolved = Vec::with_capacity(frame.draws.len());
        for d in &frame.draws {
            if !self.gpu_assets.ensure_mesh(device, assets, d.mesh) {
                continue;
            }
            let mat = self
                .gpu_assets
                .ensure_material(device, queue, assets, d.material);
            let gm = &self.gpu_assets.materials[&mat];
            let mesh = &self.gpu_assets.meshes[&d.mesh];
            if d.submesh as usize >= mesh.submeshes.len() {
                continue;
            }
            let (center, radius) = if d.joint_offset.is_some() {
                (Vector3::ZERO, f32::INFINITY)
            } else {
                let c = d.model.transform_point3(mesh.bounds.center);
                let s = d
                    .model
                    .x_axis
                    .truncate()
                    .length()
                    .max(d.model.y_axis.truncate().length())
                    .max(d.model.z_axis.truncate().length());
                (c, mesh.bounds.extents.length() * s)
            };
            resolved.push(Resolved {
                item: d,
                material: mat,
                center,
                radius,
                bounds: mesh.bounds,
                transparent: gm.transparent,
                cutout: gm.cutout,
                double_sided: gm.double_sided,
                cast: gm.cast_shadows && (d.cast_shadows || d.shadows_only),
                priority: gm.priority,
            });
        }

        // Terrain chunks: (draw, center, radius, local bounds).
        let mut terrain_items = Vec::new();
        for t in &frame.terrain_draws {
            if !self.gpu_assets.ensure_mesh(device, assets, t.mesh)
                || !self.terrain.prepare(device, queue, assets, t.data)
            {
                continue;
            }
            let mesh = &self.gpu_assets.meshes[&t.mesh];
            let c = t.model.transform_point3(mesh.bounds.center);
            terrain_items.push((t, c, mesh.bounds.extents.length(), mesh.bounds));
        }

        // ---- lights and joints --------------------------------------------------------
        let lights: Vec<LightUniform> = frame
            .lights
            .iter()
            .take(MAX_LIGHTS)
            .map(|l| {
                let (dir, spot, ty) = match l.spot {
                    Some((d, outer, inner)) => {
                        let co = (outer.to_radians() * 0.5).cos();
                        let ci = (inner.min(outer - 0.1).max(0.0).to_radians() * 0.5).cos();
                        (
                            d.normalize_or(Vector3::Z),
                            [co, 1.0 / (ci - co).max(1e-4), 0.0, 0.0],
                            1.0,
                        )
                    }
                    None => (Vector3::Z, [0.0; 4], 0.0),
                };
                LightUniform {
                    pos_range: l.position.extend(l.range.max(0.01)).to_array(),
                    color: [l.color[0], l.color[1], l.color[2], ty],
                    dir: dir.extend(0.0).to_array(),
                    spot,
                }
            })
            .collect();
        if !lights.is_empty() {
            queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&lights));
        }
        let joint_bytes = (frame.joints.len() * 64) as u64;
        if grow_buffer(
            device,
            &mut self.joint_buffer,
            joint_bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            "joints",
        ) {
            self.groups_dirty = true;
        }
        if !frame.joints.is_empty() {
            let flat: Vec<f32> = frame
                .joints
                .iter()
                .flat_map(|m| m.to_cols_array())
                .collect();
            queue.write_buffer(&self.joint_buffer, 0, bytemuck::cast_slice(&flat));
        }
        self.rebuild_groups(device, frame.cameras.len().max(1));

        let env = &frame.environment;
        let mut instances: Vec<Instance> = Vec::with_capacity(resolved.len());
        let make_instance = |d: &DrawItem| {
            let m = d.model;
            let n =
                Matrix4x4::from_mat3(oxide_math::glam::Mat3::from_mat4(m).inverse().transpose());
            Instance {
                model: m.to_cols_array(),
                normal: [
                    n.x_axis.to_array(),
                    n.y_axis.to_array(),
                    n.z_axis.to_array(),
                ],
                extra: [d.joint_offset.unwrap_or(0), d.receive_shadows as u32, 0, 0],
            }
        };

        // ---- shadows (from the first camera) ------------------------------------------
        let mut shadow_batches: Vec<Vec<ShadowBatch>> = (0..CASCADES).map(|_| Vec::new()).collect();
        let mut cascade_vp = [Matrix4x4::IDENTITY; CASCADES];
        let mut splits = [0.0f32; CASCADES];
        let mut texels = [0.0f32; CASCADES];
        let shadows = frame.sun.as_ref().and_then(|s| s.shadows.map(|sh| (s, sh)));
        let aspect0 =
            |c: &CameraView| (w as f32 * c.viewport[2]) / (h as f32 * c.viewport[3]).max(1.0);
        if let (Some((sun, _)), Some(cam)) = (shadows, frame.cameras.first()) {
            let cascades = compute_cascades(
                cam,
                aspect0(cam),
                sun.direction,
                env.shadow_distance,
                self.shadow_size,
            );
            for (i, c) in cascades.iter().enumerate() {
                cascade_vp[i] = c.view_proj;
                splits[i] = c.split;
                texels[i] = c.texel;
                queue.write_buffer(
                    &self.shadow_buffers[i],
                    0,
                    bytemuck::cast_slice(&c.view_proj.to_cols_array()),
                );
                let mut casters: Vec<&Resolved> = resolved
                    .iter()
                    .filter(|r| r.cast && !r.transparent && c.overlaps(r.center, r.radius))
                    .collect();
                casters.sort_by_key(|r| {
                    (
                        r.cutout,
                        r.item.joint_offset.is_some(),
                        r.material,
                        r.item.mesh,
                        r.item.submesh,
                    )
                });
                let mut list: Vec<ShadowBatch> = Vec::new();
                for r in casters {
                    let key = ShadowKey {
                        cutout: r.cutout,
                        double_sided: r.double_sided,
                        skinned: r.item.joint_offset.is_some(),
                    };
                    let idx = instances.len() as u32;
                    instances.push(make_instance(r.item));
                    if let Some(last) = list.last_mut()
                        && !key.skinned
                        && last.key == key
                        && last.mesh == r.item.mesh
                        && last.submesh == r.item.submesh
                        && (!key.cutout || last.material == r.material)
                        && last.instances.end == idx
                    {
                        last.instances.end += 1;
                        continue;
                    }
                    list.push(ShadowBatch {
                        key,
                        material: r.material,
                        mesh: r.item.mesh,
                        submesh: r.item.submesh,
                        instances: idx..idx + 1,
                    });
                }
                shadow_batches[i] = list;
            }
        }

        // ---- per-camera culling and batching -------------------------------------------
        let mut camera_batches: Vec<(Vec<Batch>, Vec<Batch>)> = Vec::new();
        let mut camera_terrain: Vec<Vec<(AssetId, AssetId, u32)>> = Vec::new();
        let mut occlusion = std::mem::take(&mut self.occlusion);
        occlusion.resize_with(frame.cameras.len(), Default::default);
        for (ci, cam) in frame.cameras.iter().enumerate() {
            let aspect = aspect0(cam);
            let vp = cam.view_projection(aspect);
            let planes = frustum_planes(&vp);
            let occ = &mut occlusion[ci];
            let occlusion_on = cam.occlusion_culling;
            if occlusion_on {
                let started = std::time::Instant::now();
                occ.begin(vp, aspect);
                // Biggest on screen first, until the triangle budget runs out.
                enum Occluder<'a> {
                    Draw(&'a Resolved<'a>),
                    Terrain(&'a TerrainDraw, AssetId),
                    Box(&'a Matrix4x4),
                }
                let mut occluders: Vec<(f32, Occluder)> = Vec::new();
                let size = |c: Vector3, r: f32| r / (c - cam.position).length().max(0.01);
                for r in &resolved {
                    if r.item.occluder
                        && !r.item.shadows_only
                        && !r.transparent
                        && !r.cutout
                        && r.radius.is_finite()
                        && cam.culling_mask & (1u32 << (r.item.layer & 31)) != 0
                        && sphere_visible(&planes, r.center, r.radius)
                    {
                        let s = size(r.center, r.radius);
                        if s >= MIN_OCCLUDER_SIZE {
                            occluders.push((s, Occluder::Draw(r)));
                        }
                    }
                }
                for (t, c, r, _) in &terrain_items {
                    if let Some(m) = t.occluder
                        && sphere_visible(&planes, *c, *r)
                    {
                        occluders.push((size(*c, *r), Occluder::Terrain(t, m)));
                    }
                }
                for m in &frame.occluder_boxes {
                    occluders.push((f32::INFINITY, Occluder::Box(m)));
                }
                occluders.sort_by(|a, b| b.0.total_cmp(&a.0));
                for (_, o) in &occluders {
                    if occ.triangles >= OCCLUDER_TRIANGLE_BUDGET {
                        break;
                    }
                    match o {
                        Occluder::Draw(r) => {
                            let Some(m) = assets.meshes.get_by_id(r.item.mesh) else {
                                continue;
                            };
                            let Some(range) = m.submeshes.get(r.item.submesh as usize) else {
                                continue;
                            };
                            if (range.end - range.start) / 3 > MAX_OCCLUDER_TRIANGLES {
                                continue;
                            }
                            let idx = &m.indices[range.start as usize..range.end as usize];
                            occ.rasterize(&r.item.model, &m.vertices, idx);
                        }
                        Occluder::Terrain(t, mesh) => {
                            if let Some(m) = assets.meshes.get_by_id(*mesh) {
                                occ.rasterize(&t.model, &m.vertices, &m.indices);
                            }
                        }
                        Occluder::Box(m) => occ.rasterize_box(m),
                    }
                    stats.occluders += 1;
                }
                occ.finish();
                stats.occluder_triangles += occ.triangles;
                stats.occlusion_ms += started.elapsed().as_secs_f32() * 1000.0;
            } else {
                *occ = Default::default();
            }
            let mut opaque: Vec<&Resolved> = Vec::new();
            let mut transparent: Vec<(f32, &Resolved)> = Vec::new();
            for r in &resolved {
                if r.item.shadows_only || cam.culling_mask & (1u32 << (r.item.layer & 31)) == 0 {
                    continue;
                }
                if r.radius.is_finite() && !sphere_visible(&planes, r.center, r.radius) {
                    stats.culled += 1;
                    continue;
                }
                if occlusion_on
                    && r.item.occludee
                    && r.radius.is_finite()
                    && !occ.is_visible(&r.item.model, r.bounds.center, r.bounds.extents)
                {
                    stats.occlusion_culled += 1;
                    continue;
                }
                if r.transparent {
                    transparent.push(((r.center - cam.position).length_squared(), r));
                } else {
                    opaque.push(r);
                }
            }
            opaque.sort_by_key(|r| {
                (
                    r.double_sided,
                    r.item.joint_offset.is_some(),
                    r.material,
                    r.item.mesh,
                    r.item.submesh,
                )
            });
            transparent.sort_by(|a, b| {
                a.1.priority
                    .cmp(&b.1.priority)
                    .then(b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal))
            });
            let mut ob: Vec<Batch> = Vec::new();
            for r in opaque {
                let key = MainKey {
                    transparent: false,
                    double_sided: r.double_sided,
                    skinned: r.item.joint_offset.is_some(),
                };
                let idx = instances.len() as u32;
                instances.push(make_instance(r.item));
                if let Some(last) = ob.last_mut()
                    && !key.skinned
                    && last.key == key
                    && last.material == r.material
                    && last.mesh == r.item.mesh
                    && last.submesh == r.item.submesh
                    && last.instances.end == idx
                {
                    last.instances.end += 1;
                    continue;
                }
                ob.push(Batch {
                    key,
                    material: r.material,
                    mesh: r.item.mesh,
                    submesh: r.item.submesh,
                    instances: idx..idx + 1,
                });
            }
            let tb: Vec<Batch> = transparent
                .into_iter()
                .map(|(_, r)| {
                    let idx = instances.len() as u32;
                    instances.push(make_instance(r.item));
                    Batch {
                        key: MainKey {
                            transparent: true,
                            double_sided: r.double_sided,
                            skinned: r.item.joint_offset.is_some(),
                        },
                        material: r.material,
                        mesh: r.item.mesh,
                        submesh: r.item.submesh,
                        instances: idx..idx + 1,
                    }
                })
                .collect();
            camera_batches.push((ob, tb));
            let mut tl = Vec::new();
            for (t, c, r, b) in &terrain_items {
                if !sphere_visible(&planes, *c, *r) {
                    stats.culled += 1;
                    continue;
                }
                if occlusion_on && !occ.is_visible(&t.model, b.center, b.extents) {
                    stats.occlusion_culled += 1;
                    continue;
                }
                let idx = instances.len() as u32;
                instances.push(Instance {
                    model: t.model.to_cols_array(),
                    normal: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    extra: [0, t.receive_shadows as u32, 0, 0],
                });
                tl.push((t.data, t.mesh, idx));
            }
            camera_terrain.push(tl);

            // Frame uniform for this camera.
            let view = cam.view();
            let sun_dir = frame
                .sun
                .as_ref()
                .map_or(Vector3::new(0.3, -0.8, 0.5).normalize(), |s| {
                    s.direction.normalize_or(Vector3::NEG_Y)
                });
            let (sun_color, strength, sp) = match (&frame.sun, shadows) {
                (Some(s), Some((_, sh))) => (
                    s.color,
                    sh.strength,
                    [
                        sh.bias,
                        sh.normal_bias,
                        sh.soft as u32 as f32,
                        CASCADES as f32,
                    ],
                ),
                (Some(s), None) => (s.color, 0.0, [0.0; 4]),
                (None, _) => ([0.0; 3], 0.0, [0.0; 4]),
            };
            let tint = env.sky_tint.linear();
            let ground = env.ground_color.linear();
            // Base sky colors modulated by the tint (0.5 gray = neutral, like Unity).
            // sRGB 0.5 gray (linear 0.214) is the neutral tint.
            let nt = Vector3::new(tint.r, tint.g, tint.b) / 0.214;
            let zenith = Vector3::new(0.075, 0.17, 0.46) * nt;
            let horizon = Vector3::new(0.36, 0.44, 0.56) * nt.powf(0.5);
            let (fog_mode, fog_p, fog_c) = match env.fog {
                None => (0.0, [0.0; 3], [0.0; 3]),
                Some((kind, c)) => {
                    let c = c.linear();
                    match kind {
                        FogKind::Linear { start, end } => (1.0, [0.0, start, end], [c.r, c.g, c.b]),
                        FogKind::Exponential { density } => {
                            (2.0, [density, 0.0, 0.0], [c.r, c.g, c.b])
                        }
                        FogKind::ExponentialSquared { density } => {
                            (3.0, [density, 0.0, 0.0], [c.r, c.g, c.b])
                        }
                    }
                }
            };
            let clear = match cam.clear {
                ClearMode::Color(c) => [c.r, c.g, c.b, 1.0],
                _ => [0.0, 0.0, 0.0, 1.0],
            };
            let mut cvp = [[0.0f32; 16]; CASCADES];
            for i in 0..CASCADES {
                cvp[i] = cascade_vp[i].to_cols_array();
            }
            let u = FrameUniform {
                view_proj: vp.to_cols_array(),
                view: view.to_cols_array(),
                inv_view_proj: vp.inverse().to_cols_array(),
                camera_pos: cam.position.extend(frame.time).to_array(),
                sun_dir: sun_dir.extend(frame.sun.is_some() as u32 as f32).to_array(),
                sun_color: [sun_color[0], sun_color[1], sun_color[2], strength],
                shadow_params: sp,
                cascade_splits: splits,
                cascade_vp: cvp,
                sky_zenith: zenith.extend(env.sky_exposure).to_array(),
                sky_horizon: horizon.extend(env.sun_size).to_array(),
                ground_color: [ground.r, ground.g, ground.b, env.ambient_intensity],
                fog_color: [fog_c[0], fog_c[1], fog_c[2], fog_mode],
                fog_params: [fog_p[0], fog_p[1], fog_p[2], env.reflection_intensity],
                misc: [lights.len() as f32, 0.0, w as f32, h as f32],
                clear_color: clear,
                cascade_texel: texels,
            };
            queue.write_buffer(&self.frame_buffers[ci], 0, bytemuck::bytes_of(&u));
        }

        // ---- upload instances and lines ------------------------------------------------
        let inst_bytes = (instances.len() * std::mem::size_of::<Instance>()) as u64;
        grow_buffer(
            device,
            &mut self.instance_buffer,
            inst_bytes,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "instances",
        );
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }
        let line_bytes = (frame.lines.len() * std::mem::size_of::<LineVertex>()) as u64;
        grow_buffer(
            device,
            &mut self.line_buffer,
            line_bytes,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "lines",
        );
        if !frame.lines.is_empty() {
            queue.write_buffer(&self.line_buffer, 0, bytemuck::cast_slice(&frame.lines));
        }

        // Make sure every pipeline exists before borrowing for the passes.
        for (o, t) in &camera_batches {
            for b in o.iter().chain(t) {
                self.main_pipeline(device, b.key);
            }
        }
        for list in &shadow_batches {
            for b in list {
                self.shadow_pipeline(device, b.key);
            }
        }

        let cam0 = frame.cameras.first().map_or(Vector3::ZERO, |c| c.position);
        let particle_ranges = self.particles.prepare(
            device,
            queue,
            &mut self.gpu_assets,
            assets,
            &frame.particles,
            cam0,
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene"),
        });

        // ---- shadow passes ------------------------------------------------------------
        if shadows.is_some() {
            for (i, list) in shadow_batches.iter().enumerate() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow cascade"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow_layers[i],
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_bind_group(0, &self.shadow_groups[i], &[]);
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                for b in list {
                    let mesh = &self.gpu_assets.meshes[&b.mesh];
                    let mat = &self.gpu_assets.materials[&b.material];
                    pass.set_pipeline(&self.shadow_pipelines[&b.key]);
                    pass.set_bind_group(1, &mat.bind_group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    if b.key.skinned {
                        let Some(skin) = &mesh.skin else { continue };
                        pass.set_vertex_buffer(2, skin.slice(..));
                    }
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(
                        mesh.submeshes[b.submesh as usize].clone(),
                        0,
                        b.instances.clone(),
                    );
                    stats.shadow_draw_calls += 1;
                }
            }
        }

        // ---- camera passes -------------------------------------------------------------
        let targets = self.targets.get(&(w, h)).expect("targets");
        let (color_view, resolve) = match &targets.msaa {
            Some(m) => (m, Some(&targets.resolve)),
            None => (&targets.resolve, None),
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cameras"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            for (ci, cam) in frame.cameras.iter().enumerate() {
                let [vx, vy, vw, vh] = cam.viewport;
                let px = (vx * w as f32).clamp(0.0, w as f32);
                let pw = (vw * w as f32).clamp(1.0, w as f32 - px);
                let ph = (vh * h as f32).clamp(1.0, h as f32);
                let py = (h as f32 - (vy * h as f32) - ph).clamp(0.0, h as f32 - ph);
                pass.set_viewport(px, py, pw, ph, 0.0, 1.0);
                pass.set_scissor_rect(px as u32, py as u32, (pw as u32).max(1), (ph as u32).max(1));
                pass.set_bind_group(0, &self.frame_groups[ci], &[]);
                match cam.clear {
                    ClearMode::Sky => pass.set_pipeline(&self.sky_pipeline),
                    ClearMode::Color(_) => pass.set_pipeline(&self.solid_pipeline),
                    ClearMode::DepthOnly => pass.set_pipeline(&self.depth_clear_pipeline),
                    ClearMode::Nothing => {}
                }
                if cam.clear != ClearMode::Nothing {
                    pass.draw(0..3, 0..1);
                }
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                if let Some(tp) = &self.terrain.pipeline
                    && !camera_terrain[ci].is_empty()
                {
                    pass.set_pipeline(tp);
                    for &(data, mesh_id, idx) in &camera_terrain[ci] {
                        let (Some(bg), Some(mesh)) = (
                            self.terrain.bind_group(data),
                            self.gpu_assets.meshes.get(&mesh_id),
                        ) else {
                            continue;
                        };
                        pass.set_bind_group(1, bg, &[]);
                        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                        let range = mesh.submeshes[0].clone();
                        stats.triangles += ((range.end - range.start) / 3) as u64;
                        stats.draw_calls += 1;
                        pass.draw_indexed(range, 0, idx..idx + 1);
                    }
                }
                let (ob, tb) = &camera_batches[ci];
                for b in ob.iter().chain(tb) {
                    let mesh = &self.gpu_assets.meshes[&b.mesh];
                    let mat = &self.gpu_assets.materials[&b.material];
                    pass.set_pipeline(&self.main_pipelines[&b.key]);
                    pass.set_bind_group(1, &mat.bind_group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    if b.key.skinned {
                        let Some(skin) = &mesh.skin else { continue };
                        pass.set_vertex_buffer(2, skin.slice(..));
                    }
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    let range = mesh.submeshes[b.submesh as usize].clone();
                    let n = b.instances.end - b.instances.start;
                    stats.triangles += ((range.end - range.start) / 3) as u64 * n as u64;
                    stats.instances += n;
                    stats.draw_calls += 1;
                    pass.draw_indexed(range, 0, b.instances.clone());
                }
                if !particle_ranges.is_empty() {
                    stats.draw_calls += self.particles.draw(&mut pass, &particle_ranges);
                }
                if !frame.lines.is_empty() {
                    pass.set_pipeline(&self.line_pipeline);
                    pass.set_vertex_buffer(0, self.line_buffer.slice(..));
                    pass.draw(0..frame.lines.len() as u32, 0..1);
                }
            }
        }

        // ---- post-processing into the target ------------------------------------------
        let bloom = env.bloom.filter(|_| !frame.cameras.is_empty());
        self.post.run(
            device,
            queue,
            &mut encoder,
            &targets.resolve,
            &targets.bloom,
            target,
            format,
            env.exposure,
            bloom,
            env.vignette,
        );
        queue.submit([encoder.finish()]);
        self.occlusion = occlusion;
        self.last_stats = stats;
        stats
    }
}

struct Cascade {
    view_proj: Matrix4x4,
    split: f32,
    texel: f32,
    light_view: Matrix4x4,
    center: Vector3,
    radius: f32,
}

impl Cascade {
    /// Whether a caster's bounding sphere can throw a shadow into this cascade.
    fn overlaps(&self, c: Vector3, r: f32) -> bool {
        if !r.is_finite() {
            return true;
        }
        let p = self.light_view.transform_point3(c);
        (p.x - self.center.x).abs() <= self.radius + r
            && (p.y - self.center.y).abs() <= self.radius + r
            && p.z - r <= self.center.z + self.radius
    }
}

fn compute_cascades(
    cam: &CameraView,
    aspect: f32,
    sun_dir: Vector3,
    shadow_distance: f32,
    res: u32,
) -> Vec<Cascade> {
    let near = cam.near.max(0.01);
    let far = cam.far.min(shadow_distance).max(near + 1.0);
    let lambda = 0.8;
    let dir = sun_dir.normalize_or(Vector3::NEG_Y);
    let up = if dir.y.abs() > 0.99 {
        Vector3::Z
    } else {
        Vector3::Y
    };
    let light_view = Matrix4x4::look_to(Vector3::ZERO, dir, up);
    let mut out = Vec::new();
    let mut prev = near;
    for i in 0..CASCADES {
        let p = (i + 1) as f32 / CASCADES as f32;
        let log = near * (far / near).powf(p);
        let lin = near + (far - near) * p;
        let split = lambda * log + (1.0 - lambda) * lin;
        // Slice corners in view space.
        let corners: Vec<Vector3> = [prev, split]
            .iter()
            .flat_map(|&d| {
                let (hw, hh) = match cam.projection {
                    ProjectionKind::Perspective { fov } => {
                        let t = (fov.to_radians() * 0.5).tan();
                        (d * t * aspect, d * t)
                    }
                    ProjectionKind::Orthographic { size } => (size * aspect, size),
                };
                [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
                    .map(|(sx, sy)| Vector3::new(sx * hw, sy * hh, d))
            })
            .map(|v| cam.position + cam.rotation * v)
            .collect();
        let center_ws = corners.iter().copied().sum::<Vector3>() / corners.len() as f32;
        let mut radius = corners
            .iter()
            .map(|c| c.distance(center_ws))
            .fold(0.0, f32::max);
        radius = (radius * 16.0).ceil() / 16.0;
        let texel = radius * 2.0 / res as f32;
        let mut c = light_view.transform_point3(center_ws);
        // Snap to whole texels so the shadow doesn't shimmer as the camera moves.
        c.x = (c.x / texel).floor() * texel;
        c.y = (c.y / texel).floor() * texel;
        let back = radius.max(300.0);
        let proj = Matrix4x4::ortho(
            c.x - radius,
            c.x + radius,
            c.y - radius,
            c.y + radius,
            c.z - back,
            c.z + radius,
        );
        out.push(Cascade {
            view_proj: proj * light_view,
            split,
            texel,
            light_view,
            center: c,
            radius,
        });
        prev = split;
    }
    out
}

fn frustum_planes(m: &Matrix4x4) -> [Vector4; 6] {
    let r0 = m.row(0);
    let r1 = m.row(1);
    let r2 = m.row(2);
    let r3 = m.row(3);
    // Reversed-Z: 0 <= z <= w.
    [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2].map(|p| {
        let l = p.truncate().length();
        if l > 0.0 { p / l } else { p }
    })
}

fn sphere_visible(planes: &[Vector4; 6], c: Vector3, r: f32) -> bool {
    planes.iter().all(|p| p.truncate().dot(c) + p.w >= -r)
}
