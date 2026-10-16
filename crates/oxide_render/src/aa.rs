//! Anti-aliasing passes: per-object motion vectors and the TAA resolve, and FXAA.
//! The renderer decides when they run (see `SceneRenderer::render_scene`).
//!
//! Reference: <https://docs.unity3d.com/Manual/urp/anti-aliasing.html>

use crate::gpu_assets::{SkinVertex, Vertex};
use crate::renderer::HDR_FORMAT;
use std::collections::HashMap;

/// Motion vectors: UV-space velocity in `xy`, `a` = 1 where an object wrote one.
pub const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// FXAA input: the tonemapped image, sRGB-encoded by the composite pass.
pub const LDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// One object in the motion-vector pass.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MotionInstance {
    pub model: [f32; 16],
    pub prev: [f32; 16],
    /// `x` joint offset, `y` previous joint offset (skinned draws).
    pub extra: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionUniform {
    jittered_vp: [f32; 16],
    curr_vp: [f32; 16],
    prev_vp: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TaaUniform {
    pub inv_vp: [f32; 16],
    pub prev_vp: [f32; 16],
    /// x current-frame weight, y variance clamp scale, z history valid.
    pub params: [f32; 4],
    pub texel: [f32; 4],
}

pub(crate) struct AaGpu {
    motion_layout: wgpu::BindGroupLayout,
    pub motion_static: wgpu::RenderPipeline,
    pub motion_skinned: wgpu::RenderPipeline,
    motion_uniform: wgpu::Buffer,
    taa_layout: wgpu::BindGroupLayout,
    taa_pipeline: wgpu::RenderPipeline,
    taa_uniform: wgpu::Buffer,
    fxaa_layout: wgpu::BindGroupLayout,
    fxaa_pipeline_layout: wgpu::PipelineLayout,
    fxaa_module: wgpu::ShaderModule,
    fxaa_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    fxaa_uniform: wgpu::Buffer,
    linear: wgpu::Sampler,
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

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
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
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

impl AaGpu {
    pub fn new(device: &wgpu::Device) -> Self {
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        // ---- motion vectors ---------------------------------------------------------
        let motion_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX),
                storage(1),
                storage(2),
                texture_entry(3, wgpu::TextureSampleType::Depth),
            ],
        });
        let motion_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("motion.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/motion.wgsl").into()),
        });
        let motion_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("motion pipeline layout"),
            bind_group_layouts: &[Some(&motion_layout)],
            immediate_size: 0,
        });
        const POSITION: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
        const INSTANCE: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
            1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
            5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Uint32x4
        ];
        const SKIN: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![10 => Uint16x4, 11 => Float32x4];
        let motion = |skinned: bool| {
            let mut buffers = vec![
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &POSITION,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MotionInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &INSTANCE,
                }),
            ];
            if skinned {
                buffers.push(Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SkinVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SKIN,
                }));
            }
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if skinned { "motion skinned" } else { "motion" }),
                layout: Some(&motion_pl),
                vertex: wgpu::VertexState {
                    module: &motion_module,
                    entry_point: Some(if skinned { "vs_skinned" } else { "vs_main" }),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &motion_module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: MOTION_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let motion_static = motion(false);
        let motion_skinned = motion(true);
        let buffer = |label: &str, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let motion_uniform = buffer("motion uniform", std::mem::size_of::<MotionUniform>());

        // ---- TAA ----------------------------------------------------------------------
        let taa_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("taa layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1, float),
                texture_entry(2, float),
                texture_entry(3, float),
                texture_entry(4, wgpu::TextureSampleType::Depth),
                sampler_entry(5),
            ],
        });
        let fullscreen = |label: &str,
                          src: &str,
                          layout: &wgpu::BindGroupLayout,
                          format: wgpu::TextureFormat|
         -> (
            wgpu::RenderPipeline,
            wgpu::PipelineLayout,
            wgpu::ShaderModule,
        ) {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            let p = Self::fullscreen_pipeline(device, label, &pl, &module, format);
            (p, pl, module)
        };
        let (taa_pipeline, _, _) = fullscreen(
            "taa.wgsl",
            include_str!("shaders/taa.wgsl"),
            &taa_layout,
            HDR_FORMAT,
        );
        let taa_uniform = buffer("taa uniform", std::mem::size_of::<TaaUniform>());

        // ---- FXAA ---------------------------------------------------------------------
        let fxaa_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fxaa layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1, float),
                sampler_entry(2),
            ],
        });
        let (_, fxaa_pipeline_layout, fxaa_module) = fullscreen(
            "fxaa.wgsl",
            include_str!("shaders/fxaa.wgsl"),
            &fxaa_layout,
            LDR_FORMAT,
        );
        let fxaa_uniform = buffer("fxaa uniform", 16);
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("aa linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            motion_layout,
            motion_static,
            motion_skinned,
            motion_uniform,
            taa_layout,
            taa_pipeline,
            taa_uniform,
            fxaa_layout,
            fxaa_pipeline_layout,
            fxaa_module,
            fxaa_pipelines: HashMap::new(),
            fxaa_uniform,
            linear,
        }
    }

    fn fullscreen_pipeline(
        device: &wgpu::Device,
        label: &str,
        layout: &wgpu::PipelineLayout,
        module: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Writes the motion pass matrices and returns its bind group.
    #[allow(clippy::too_many_arguments)]
    pub fn motion_group(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        jittered_vp: [f32; 16],
        curr_vp: [f32; 16],
        prev_vp: [f32; 16],
        joints: &wgpu::Buffer,
        prev_joints: &wgpu::Buffer,
        depth: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        queue.write_buffer(
            &self.motion_uniform,
            0,
            bytemuck::bytes_of(&MotionUniform {
                jittered_vp,
                curr_vp,
                prev_vp,
            }),
        );
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motion group"),
            layout: &self.motion_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.motion_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: joints.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: prev_joints.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        })
    }

    /// Resolves `current` + `history` into `out`.
    #[allow(clippy::too_many_arguments)]
    pub fn taa(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        uniform: TaaUniform,
        current: &wgpu::TextureView,
        history: &wgpu::TextureView,
        motion: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        out: &wgpu::TextureView,
    ) {
        queue.write_buffer(&self.taa_uniform, 0, bytemuck::bytes_of(&uniform));
        let tv = wgpu::BindingResource::TextureView;
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("taa group"),
            layout: &self.taa_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.taa_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tv(current),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tv(history),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: tv(motion),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: tv(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
            ],
        });
        fullscreen_pass(encoder, "taa", out, &self.taa_pipeline, &group);
    }

    /// FXAA from the sRGB-encoded `src` (`w`×`h`) into `target`.
    #[allow(clippy::too_many_arguments)]
    pub fn fxaa(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        w: u32,
        h: u32,
    ) {
        let (pl, module) = (&self.fxaa_pipeline_layout, &self.fxaa_module);
        let pipeline = self
            .fxaa_pipelines
            .entry(format)
            .or_insert_with(|| Self::fullscreen_pipeline(device, "fxaa", pl, module, format));
        let u = [
            1.0 / w as f32,
            1.0 / h as f32,
            format.is_srgb() as u32 as f32,
            0.0,
        ];
        queue.write_buffer(&self.fxaa_uniform, 0, bytemuck::cast_slice(&u));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fxaa group"),
            layout: &self.fxaa_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.fxaa_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
            ],
        });
        fullscreen_pass(encoder, "fxaa", target, pipeline, &group);
    }
}

/// Halton(2, 3) sub-pixel jitter in pixels (-0.5..0.5) for frame `index`, 8-frame cycle.
pub fn jitter(index: u64) -> (f32, f32) {
    fn halton(mut i: u32, base: u32) -> f32 {
        let (mut f, mut r) = (1.0, 0.0);
        while i > 0 {
            f /= base as f32;
            r += f * (i % base) as f32;
            i /= base;
        }
        r
    }
    let i = (index % 8) as u32 + 1;
    (halton(i, 2) - 0.5, halton(i, 3) - 0.5)
}

#[cfg(test)]
mod tests {
    #[test]
    fn jitter_covers_the_pixel_and_averages_to_center() {
        let samples: Vec<(f32, f32)> = (0..8).map(super::jitter).collect();
        let (mx, my) = samples
            .iter()
            .fold((0.0, 0.0), |a, s| (a.0 + s.0 / 8.0, a.1 + s.1 / 8.0));
        assert!(mx.abs() < 0.07 && my.abs() < 0.07, "mean ({mx}, {my})");
        assert!(samples.iter().all(|s| s.0.abs() <= 0.5 && s.1.abs() <= 0.5));
        let distinct: std::collections::HashSet<_> = samples
            .iter()
            .map(|s| ((s.0 * 1000.0) as i32, (s.1 * 1000.0) as i32))
            .collect();
        assert_eq!(distinct.len(), 8);
    }
}
