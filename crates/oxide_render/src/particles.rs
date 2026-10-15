//! Particle billboards: per-texture bind groups and alpha/additive pipelines.

use crate::frame::{ParticleBatch, ParticleBlend, ParticleInstance};
use crate::gpu_assets::GpuAssets;
use crate::renderer::{DEPTH_FORMAT, HDR_FORMAT};
use oxide_assets::{AssetId, AssetServer};
use std::collections::HashMap;

pub struct ParticleGpu {
    layout: wgpu::BindGroupLayout,
    alpha: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    groups: HashMap<Option<AssetId>, wgpu::BindGroup>,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    buffer: wgpu::Buffer,
}

impl ParticleGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame_layout: &wgpu::BindGroupLayout,
        module: &wgpu::ShaderModule,
        samples: u32,
    ) -> Self {
        use wgpu::util::DeviceExt;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle pipeline layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout)],
            immediate_size: 0,
        });
        let make = |blend: wgpu::BlendState, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<ParticleInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x4, 3 => Float32],
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let alpha = make(wgpu::BlendState::ALPHA_BLENDING, "particles alpha");
        let additive = make(additive_blend, "particles additive");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particle sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let white = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("particle white"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[255; 4],
            )
            .create_view(&Default::default());
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles"),
            size: 48 * 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            alpha,
            additive,
            groups: HashMap::new(),
            sampler,
            white,
            buffer,
        }
    }

    fn group(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut GpuAssets,
        assets: &AssetServer,
        tex: Option<AssetId>,
    ) {
        if self.groups.contains_key(&tex) {
            return;
        }
        let view = match tex {
            Some(id) => match gpu_assets.texture_view(device, queue, assets, id) {
                Some(v) => v,
                None => return,
            },
            None => &self.white,
        };
        let g = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle texture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.groups.insert(tex, g);
    }

    /// Sorts, uploads and returns draw ranges `(texture, blend, start..end)`.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut GpuAssets,
        assets: &AssetServer,
        batches: &[ParticleBatch],
        camera: oxide_math::Vector3,
    ) -> Vec<(Option<AssetId>, ParticleBlend, std::ops::Range<u32>)> {
        let mut all: Vec<ParticleInstance> = Vec::new();
        let mut ranges = Vec::new();
        // Alpha batches back-to-front by their farthest particle, additive ones in any order.
        let mut order: Vec<&ParticleBatch> =
            batches.iter().filter(|b| !b.particles.is_empty()).collect();
        let dist = |b: &ParticleBatch| {
            b.particles
                .iter()
                .map(|p| oxide_math::Vector3::from_array(p.position).distance_squared(camera))
                .fold(0.0, f32::max)
        };
        order.sort_by(|a, b| dist(b).total_cmp(&dist(a)));
        for b in order {
            self.group(device, queue, gpu_assets, assets, b.texture);
            let start = all.len() as u32;
            let mut ps = b.particles.clone();
            if b.blend == ParticleBlend::Alpha {
                ps.sort_by(|x, y| {
                    let dx = oxide_math::Vector3::from_array(x.position).distance_squared(camera);
                    let dy = oxide_math::Vector3::from_array(y.position).distance_squared(camera);
                    dy.total_cmp(&dx)
                });
            }
            all.extend(ps);
            ranges.push((b.texture, b.blend, start..all.len() as u32));
        }
        let bytes = (all.len() * std::mem::size_of::<ParticleInstance>()) as u64;
        if self.buffer.size() < bytes {
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particles"),
                size: bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !all.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&all));
        }
        ranges
    }

    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        ranges: &[(Option<AssetId>, ParticleBlend, std::ops::Range<u32>)],
    ) -> u32 {
        let mut calls = 0;
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        for (tex, blend, r) in ranges {
            let Some(g) = self.groups.get(tex) else {
                continue;
            };
            pass.set_pipeline(match blend {
                ParticleBlend::Alpha => &self.alpha,
                ParticleBlend::Additive => &self.additive,
            });
            pass.set_bind_group(1, g, &[]);
            pass.draw(0..6, r.clone());
            calls += 1;
        }
        calls
    }
}
