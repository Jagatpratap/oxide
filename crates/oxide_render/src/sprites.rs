//! The sprite pass: instanced quads per texture, alpha blended after transparent objects,
//! sorted like Unity's 2D renderer (sorting layer, order in layer, then back to front).

use crate::frame::{SpriteDraw, SpriteInstance};
use crate::gpu_assets::GpuAssets;
use crate::renderer::{DEPTH_FORMAT, HDR_FORMAT};
use oxide_assets::{AssetId, AssetServer, FilterMode};
use std::collections::HashMap;

pub struct SpriteGpu {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    pipelines: HashMap<u32, wgpu::RenderPipeline>,
    groups: HashMap<Option<AssetId>, (wgpu::BindGroup, u64)>,
    linear: wgpu::Sampler,
    point: wgpu::Sampler,
    white: wgpu::TextureView,
    buffer: wgpu::Buffer,
}

impl SpriteGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame_layout: &wgpu::BindGroupLayout,
        module: wgpu::ShaderModule,
    ) -> Self {
        use wgpu::util::DeviceExt;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite layout"),
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
            label: Some("sprite pipeline layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout)],
            immediate_size: 0,
        });
        let sampler = |filter: wgpu::FilterMode, label| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        let white = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("sprite white"),
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
            label: Some("sprites"),
            size: (std::mem::size_of::<SpriteInstance>() * 1024) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline_layout,
            module,
            pipelines: HashMap::new(),
            groups: HashMap::new(),
            linear: sampler(wgpu::FilterMode::Linear, "sprite linear"),
            point: sampler(wgpu::FilterMode::Nearest, "sprite point"),
            white,
            buffer,
        }
    }

    pub fn ensure_pipelines(&mut self, device: &wgpu::Device, samples: u32) {
        if self.pipelines.contains_key(&samples) {
            return;
        }
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprites"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SpriteInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4],
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
                module: &self.module,
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
        self.pipelines.insert(samples, p);
    }

    /// Bind group for a texture, rebuilt when the texture is re-imported.
    fn group(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut GpuAssets,
        assets: &AssetServer,
        tex: Option<AssetId>,
    ) {
        let version = tex.and_then(|id| assets.textures.version(id)).unwrap_or(0);
        if self.groups.get(&tex).is_some_and(|(_, v)| *v == version) {
            return;
        }
        let point = tex
            .and_then(|id| assets.textures.get_by_id(id))
            .is_some_and(|t| t.filter_mode == FilterMode::Point);
        let view = match tex {
            Some(id) => match gpu_assets.texture_view(device, queue, assets, id) {
                Some(v) => v,
                None => return,
            },
            None => &self.white,
        };
        let g = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite texture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(if point {
                        &self.point
                    } else {
                        &self.linear
                    }),
                },
            ],
        });
        self.groups.insert(tex, (g, version));
    }

    /// Sorts, uploads and returns draw ranges `(texture, start..end)`.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut GpuAssets,
        assets: &AssetServer,
        draws: &[SpriteDraw],
        camera: oxide_math::Vector3,
    ) -> Vec<(Option<AssetId>, std::ops::Range<u32>)> {
        if draws.is_empty() {
            return Vec::new();
        }
        let center = |d: &SpriteDraw| {
            let m = d.instance.model;
            oxide_math::Vector3::new(
                m[3][0] + 0.5 * (m[0][0] + m[1][0]),
                m[3][1] + 0.5 * (m[0][1] + m[1][1]),
                m[3][2] + 0.5 * (m[0][2] + m[1][2]),
            )
        };
        let mut order: Vec<(i32, i32, f32, usize)> = draws
            .iter()
            .enumerate()
            .map(|(i, d)| {
                (
                    d.sorting_layer,
                    d.order,
                    center(d).distance_squared(camera),
                    i,
                )
            })
            .collect();
        order.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(b.2.total_cmp(&a.2))
                .then(a.3.cmp(&b.3))
        });
        let mut all: Vec<SpriteInstance> = Vec::with_capacity(draws.len());
        let mut ranges: Vec<(Option<AssetId>, std::ops::Range<u32>)> = Vec::new();
        for (_, _, _, i) in order {
            let d = &draws[i];
            self.group(device, queue, gpu_assets, assets, d.texture);
            let at = all.len() as u32;
            all.push(d.instance);
            match ranges.last_mut() {
                Some((t, r)) if *t == d.texture && r.end == at => r.end = at + 1,
                _ => ranges.push((d.texture, at..at + 1)),
            }
        }
        let bytes = (all.len() * std::mem::size_of::<SpriteInstance>()) as u64;
        if self.buffer.size() < bytes {
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sprites"),
                size: bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&all));
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
            let Some((g, _)) = self.groups.get(tex) else {
                continue;
            };
            pass.set_bind_group(1, g, &[]);
            pass.draw(0..6, r.clone());
            calls += 1;
        }
        calls
    }
}
