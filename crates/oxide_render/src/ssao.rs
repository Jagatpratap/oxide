//! Screen Space Ambient Occlusion (URP's SSAO renderer feature): a depth prepass of the
//! opaque geometry, AO from it, a depth-aware blur, then lit shaders darken ambient (and a
//! little direct) light with it. <https://docs.unity3d.com/Manual/urp/post-processing-ssao.html>

use crate::renderer::DEPTH_FORMAT;

const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SsaoUniform {
    proj: [f32; 16],
    inv_proj: [f32; 16],
    params: [f32; 4],
    blur_dir: [f32; 4],
}

/// Per-view SSAO textures (made on first use).
pub(crate) struct SsaoTargets {
    /// Depth prepass (single sample).
    pub depth: wgpu::TextureView,
    ao: wgpu::TextureView,
    pub(crate) blurred: wgpu::TextureView,
    /// Group 2 of the lit and terrain pipelines, with the final AO.
    pub screen_group: wgpu::BindGroup,
}

pub(crate) struct SsaoGpu {
    layout: wgpu::BindGroupLayout,
    ao: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    uniforms: [wgpu::Buffer; 3],
    /// Layout of group 2 in lit shaders: the AO texture and the decal buffer.
    pub screen_layout: wgpu::BindGroupLayout,
    /// Group 2 when SSAO and decals are off (white AO, no decals).
    pub white_group: wgpu::BindGroup,
    white: wgpu::TextureView,
    no_decals: wgpu::TextureView,
}

impl SsaoGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        use wgpu::util::DeviceExt;
        let frag = wgpu::ShaderStages::FRAGMENT;
        let float_tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: frag,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: frag,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: frag,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                float_tex(2),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ssao.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ssao.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssao pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
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
                        format: AO_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let ao = make("fs_ao");
        let blur = make("fs_blur");
        let uniforms = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ssao uniform"),
                size: std::mem::size_of::<SsaoUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let screen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("screen layout (ssao, decals)"),
            entries: &[float_tex(0), float_tex(1)],
        });
        let white = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("ssao white"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: AO_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[255],
            )
            .create_view(&Default::default());
        let no_decals = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("no decals"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: crate::decals::DBUFFER_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[0; 8],
            )
            .create_view(&Default::default());
        let mut s = Self {
            layout,
            ao,
            blur,
            uniforms,
            white_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("screen group (placeholder)"),
                layout: &screen_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&white),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&no_decals),
                    },
                ],
            }),
            screen_layout,
            white,
            no_decals,
        };
        s.white_group = s.screen_group(device, None, None);
        s
    }

    /// Group 2 of the lit pipelines: the AO texture (white when none) and the decal buffer
    /// (empty when none).
    pub fn screen_group(
        &self,
        device: &wgpu::Device,
        ao: Option<&wgpu::TextureView>,
        decals: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("screen group"),
            layout: &self.screen_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(ao.unwrap_or(&self.white)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(decals.unwrap_or(&self.no_decals)),
                },
            ],
        })
    }

    pub fn targets(&self, device: &wgpu::Device, w: u32, h: u32) -> SsaoTargets {
        let tex = |label: &str, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let ao = tex("ssao", AO_FORMAT);
        let blurred = tex("ssao blurred", AO_FORMAT);
        let screen_group = self.screen_group(device, Some(&ao), None);
        SsaoTargets {
            depth: tex("depth prepass", DEPTH_FORMAT),
            ao,
            blurred,
            screen_group,
        }
    }

    /// AO from `t.depth` into `t.ao` (blurred through `t.blurred` and back).
    pub fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &SsaoTargets,
        proj: oxide_math::Matrix4x4,
        settings: &crate::frame::SsaoSettings,
    ) {
        let base = SsaoUniform {
            proj: proj.to_cols_array(),
            inv_proj: proj.inverse().to_cols_array(),
            params: [
                settings.radius.max(0.01),
                settings.intensity.max(0.0),
                settings.falloff_distance.max(0.1),
                settings.samples.clamp(4, 32) as f32,
            ],
            blur_dir: [0.0; 4],
        };
        for (i, dir) in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]].into_iter().enumerate() {
            let mut u = base;
            u.blur_dir = [dir[0], dir[1], 0.0, 0.0];
            queue.write_buffer(&self.uniforms[i], 0, bytemuck::bytes_of(&u));
        }
        // AO into `ao`, horizontal blur into `blurred`, vertical blur back into `ao` (which
        // the lit shaders read). The AO pass ignores its texture input.
        let plan: [(
            &wgpu::RenderPipeline,
            &wgpu::TextureView,
            &wgpu::TextureView,
        ); 3] = [
            (&self.ao, &t.blurred, &t.ao),
            (&self.blur, &t.ao, &t.blurred),
            (&self.blur, &t.blurred, &t.ao),
        ];
        for (i, (pipeline, input, output)) in plan.into_iter().enumerate() {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ssao group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms[i].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&t.depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(input),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ssao"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
