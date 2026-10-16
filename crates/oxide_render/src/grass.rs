//! Terrain grass and detail billboards: instanced bottom-anchored quads per texture, drawn
//! opaque (alpha-tested) after the terrain.

use crate::frame::{GrassBatch, GrassInstance};
use crate::gpu_assets::GpuAssets;
use crate::renderer::{DEPTH_FORMAT, HDR_FORMAT};
use oxide_assets::{AssetId, AssetServer};
use std::collections::HashMap;

pub struct GrassGpu {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    pipelines: HashMap<u32, wgpu::RenderPipeline>,
    groups: HashMap<Option<AssetId>, wgpu::BindGroup>,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    buffer: wgpu::Buffer,
}

impl GrassGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame_layout: &wgpu::BindGroupLayout,
        screen_layout: &wgpu::BindGroupLayout,
        module: wgpu::ShaderModule,
    ) -> Self {
        use wgpu::util::DeviceExt;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grass layout"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grass pipeline layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout), Some(screen_layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("grass sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let white = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("grass white"),
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
            label: Some("grass"),
            size: 48 * 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline_layout,
            module,
            pipelines: HashMap::new(),
            groups: HashMap::new(),
            sampler,
            white,
            buffer,
        }
    }

    pub fn ensure_pipelines(&mut self, device: &wgpu::Device, samples: u32) {
        if self.pipelines.contains_key(&samples) {
            return;
        }
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grass"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GrassInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3, 1 => Float32, 2 => Float32, 3 => Float32,
                        4 => Float32, 5 => Float32, 6 => Float32x4
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                alpha_to_coverage_enabled: false,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
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
        });
        self.pipelines.insert(samples, p);
    }

    /// Uploads the instances; returns draw ranges per texture.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut GpuAssets,
        assets: &AssetServer,
        batches: &[GrassBatch],
    ) -> Vec<(Option<AssetId>, std::ops::Range<u32>)> {
        let mut all: Vec<GrassInstance> = Vec::new();
        let mut ranges = Vec::new();
        for b in batches.iter().filter(|b| !b.instances.is_empty()) {
            if !self.groups.contains_key(&b.texture) {
                let view = match b.texture {
                    Some(id) => match gpu_assets.texture_view(device, queue, assets, id) {
                        Some(v) => v,
                        None => continue,
                    },
                    None => &self.white,
                };
                let g = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("grass texture"),
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
                self.groups.insert(b.texture, g);
            }
            let start = all.len() as u32;
            all.extend_from_slice(&b.instances);
            ranges.push((b.texture, start..all.len() as u32));
        }
        let bytes = (all.len() * std::mem::size_of::<GrassInstance>()) as u64;
        if self.buffer.size() < bytes {
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("grass"),
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
        ranges: &[(Option<AssetId>, std::ops::Range<u32>)],
        samples: u32,
    ) -> u32 {
        let Some(p) = self.pipelines.get(&samples) else {
            return 0;
        };
        pass.set_pipeline(p);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        let mut calls = 0;
        for (tex, r) in ranges {
            let Some(g) = self.groups.get(tex) else {
                continue;
            };
            pass.set_bind_group(1, g, &[]);
            pass.draw(0..12, r.clone());
            calls += 1;
        }
        calls
    }
}
