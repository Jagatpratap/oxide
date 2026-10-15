//! Post-processing stack: bloom (13-tap downsample / tent upsample chain) and the final
//! composite with exposure, ACES tonemapping and vignette.
//! Reference: <https://docs.unity3d.com/Manual/urp/integration-with-post-processing.html>

use crate::renderer::HDR_FORMAT;
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniform {
    params: [f32; 4],
    params2: [f32; 4],
    texel: [f32; 4],
    _pad: [f32; 52],
}

const SLOT: u64 = 256;
const SLOTS: u64 = 32;

pub struct PostProcessor {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    prefilter: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    composite: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bloom: Vec<(wgpu::TextureView, u32, u32)>,
    dummy: wgpu::TextureView,
}

impl PostProcessor {
    pub fn new(device: &wgpu::Device) -> Self {
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(48),
                    },
                    count: None,
                },
                tex(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let make = |entry: &str, format, blend| {
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
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let prefilter = make("fs_prefilter", HDR_FORMAT, None);
        let down = make("fs_down", HDR_FORMAT, None);
        let up = make("fs_up", HDR_FORMAT, Some(additive));
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post uniforms"),
            size: SLOT * SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("post dummy"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self {
            layout,
            pipeline_layout,
            module,
            prefilter,
            down,
            up,
            composite: HashMap::new(),
            uniforms,
            sampler,
            bloom: Vec::new(),
            dummy,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        self.bloom.clear();
        let (mut bw, mut bh) = ((w / 2).max(1), (h / 2).max(1));
        while self.bloom.len() < 6 && bw >= 4 && bh >= 4 {
            let view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("bloom"),
                    size: wgpu::Extent3d {
                        width: bw,
                        height: bh,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            self.bloom.push((view, bw, bh));
            bw /= 2;
            bh /= 2;
        }
    }

    fn group(
        &self,
        device: &wgpu::Device,
        src: &wgpu::TextureView,
        bloom: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(48),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(bloom),
                },
            ],
        })
    }

    fn pass(
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        load: bool,
        pipeline: &wgpu::RenderPipeline,
        group: &wgpu::BindGroup,
        slot: u32,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if load {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[slot * SLOT as u32]);
        pass.draw(0..3, 0..1);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        exposure: f32,
        bloom: Option<(f32, f32)>,
        vignette: f32,
    ) {
        if !self.composite.contains_key(&format) {
            let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("composite"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some("fs_composite"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            self.composite.insert(format, p);
        }
        let (intensity, threshold) = bloom.unwrap_or((0.0, 1.0));
        let bloom_on = bloom.is_some() && !self.bloom.is_empty() && intensity > 0.0;
        let mut slots: Vec<PostUniform> = Vec::new();
        let mut u = |texel: (u32, u32)| {
            slots.push(PostUniform {
                params: [threshold, 0.5, intensity, exposure],
                params2: [
                    vignette,
                    (!format.is_srgb()) as u32 as f32,
                    bloom_on as u32 as f32,
                    0.0,
                ],
                texel: [1.0 / texel.0 as f32, 1.0 / texel.1 as f32, 0.0, 0.0],
                _pad: [0.0; 52],
            });
            slots.len() as u32 - 1
        };
        // Slot plan: composite first, then bloom passes.
        let composite_slot = u((1, 1));
        let mut plan: Vec<(u32, usize, Option<usize>, bool)> = Vec::new(); // slot, dst, src(None=hdr), up
        if bloom_on {
            let first = self.bloom[0].clone();
            let s = u((first.1 * 2, first.2 * 2));
            plan.push((s, 0, None, false));
            for i in 1..self.bloom.len() {
                let (_, sw, sh) = &self.bloom[i - 1];
                let s = u((*sw, *sh));
                plan.push((s, i, Some(i - 1), false));
            }
            for i in (0..self.bloom.len() - 1).rev() {
                let (_, sw, sh) = &self.bloom[i + 1];
                let s = u((*sw, *sh));
                plan.push((s, i, Some(i + 1), true));
            }
        }
        queue.write_buffer(&self.uniforms, 0, bytemuck::cast_slice(&slots));
        for (slot, dst, src, up) in plan {
            let src_view = match src {
                None => hdr,
                Some(i) => &self.bloom[i].0,
            };
            let g = self.group(device, src_view, &self.dummy);
            let pipeline = match (src, up) {
                (None, _) => &self.prefilter,
                (Some(_), false) => &self.down,
                (Some(_), true) => &self.up,
            };
            Self::pass(encoder, &self.bloom[dst].0, up, pipeline, &g, slot);
        }
        let bloom_view = if bloom_on {
            &self.bloom[0].0
        } else {
            &self.dummy
        };
        let g = self.group(device, hdr, bloom_view);
        Self::pass(
            encoder,
            target,
            false,
            &self.composite[&format],
            &g,
            composite_slot,
        );
    }
}
