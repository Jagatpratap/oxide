//! Depth-based post effects before bloom: depth resolve, Depth Of Field (Gaussian and
//! Bokeh) and camera/object Motion Blur, in URP's order.
//! References: <https://docs.unity3d.com/Manual/urp/post-processing-depth-of-field.html>,
//! <https://docs.unity3d.com/Manual/urp/Post-Processing-Motion-Blur.html>

use crate::frame::{DofMode, PostSettings};
use crate::renderer::HDR_FORMAT;

/// Scene depth copied out of the depth buffer for post effects.
pub const DEPTH_COPY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const COC_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// Motion blur samples per pixel.
const MOTION_SAMPLES: f32 = 10.0;
/// Sensor height of Unity's default physical camera (Super 35), metres.
const FILM_HEIGHT: f32 = 0.024;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FxUniform {
    inv_vp: [f32; 16],
    prev_vp: [f32; 16],
    dof: [f32; 4],
    mb: [f32; 4],
    proj: [f32; 4],
}

/// Per-view textures for these passes (made on first use).
pub(crate) struct FxTargets {
    pub depth: wgpu::TextureView,
    coc: wgpu::TextureView,
    half: [wgpu::TextureView; 2],
    pub out: [wgpu::TextureView; 2],
}

/// What a view needs this frame.
pub(crate) struct FxInputs<'a> {
    pub post: &'a PostSettings,
    pub inv_vp: [f32; 16],
    pub prev_vp: [f32; 16],
    pub near: f32,
    pub far: f32,
    pub orthographic: bool,
    pub height: u32,
    /// TAA motion vectors, when they exist.
    pub motion: Option<&'a wgpu::TextureView>,
}

pub(crate) struct PostFx {
    resolve_ms_layout: wgpu::BindGroupLayout,
    resolve_layout: wgpu::BindGroupLayout,
    resolve_ms: wgpu::RenderPipeline,
    resolve_single: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    coc: wgpu::RenderPipeline,
    prefilter: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    motion_blur: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    linear: wgpu::Sampler,
    dummy: wgpu::TextureView,
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(&pl),
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
            entry_point: Some(entry),
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

fn pass(
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

impl PostFx {
    pub fn new(device: &wgpu::Device) -> Self {
        let frag = wgpu::ShaderStages::FRAGMENT;
        let depth_layout = |multisampled| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("depth resolve layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: frag,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled,
                    },
                    count: None,
                }],
            })
        };
        let resolve_ms_layout = depth_layout(true);
        let resolve_layout = depth_layout(false);
        let src = include_str!("shaders/depth_resolve.wgsl");
        let module = |label: &str, ty: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.replace("DEPTH_TYPE", ty).into()),
            })
        };
        let ms_module = module("depth resolve (msaa)", "texture_depth_multisampled_2d");
        let single_module = module("depth resolve", "texture_depth_2d");
        let resolve_ms = pipeline(
            device,
            &resolve_ms_layout,
            &ms_module,
            "fs_main",
            DEPTH_COPY_FORMAT,
        );
        let resolve_single = pipeline(
            device,
            &resolve_layout,
            &single_module,
            "fs_main",
            DEPTH_COPY_FORMAT,
        );

        let tex = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: frag,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("postfx layout"),
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
                tex(1, true),
                tex(2, false),
                tex(3, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: frag,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let fx_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("postfx.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/postfx.wgsl").into()),
        });
        let p = |entry: &str, format| pipeline(device, &layout, &fx_module, entry, format);
        let coc = p("fs_coc", COC_FORMAT);
        let prefilter = p("fs_dof_prefilter", HDR_FORMAT);
        let blur = p("fs_dof_blur", HDR_FORMAT);
        let composite = p("fs_dof_composite", HDR_FORMAT);
        let motion_blur = p("fs_motion_blur", HDR_FORMAT);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("postfx uniform"),
            size: std::mem::size_of::<FxUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("postfx linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("postfx dummy"),
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
            resolve_ms_layout,
            resolve_layout,
            resolve_ms,
            resolve_single,
            layout,
            coc,
            prefilter,
            blur,
            composite,
            motion_blur,
            uniform,
            linear,
            dummy,
        }
    }

    pub fn targets(device: &wgpu::Device, w: u32, h: u32) -> FxTargets {
        let tex = |label: &str, w: u32, h: u32, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w.max(1),
                        height: h.max(1),
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
        FxTargets {
            depth: tex("scene depth copy", w, h, DEPTH_COPY_FORMAT),
            coc: tex("dof coc", w, h, COC_FORMAT),
            half: [
                tex("dof half", w / 2, h / 2, HDR_FORMAT),
                tex("dof half", w / 2, h / 2, HDR_FORMAT),
            ],
            out: [
                tex("postfx out", w, h, HDR_FORMAT),
                tex("postfx out", w, h, HDR_FORMAT),
            ],
        }
    }

    /// Whether `post` needs any of these passes.
    pub fn wanted(post: &PostSettings) -> bool {
        post.enabled && (post.depth_of_field.mode != DofMode::Off || post.motion_blur[0] > 0.0)
    }

    /// Copies `depth` (MSAA when `samples` > 1) into `t.depth`.
    pub fn resolve_depth(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        samples: u32,
        t: &FxTargets,
    ) {
        let (layout, pipe) = if samples > 1 {
            (&self.resolve_ms_layout, &self.resolve_ms)
        } else {
            (&self.resolve_layout, &self.resolve_single)
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("depth resolve"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth),
            }],
        });
        pass(encoder, "depth resolve", &t.depth, pipe, &group);
    }

    fn group(
        &self,
        device: &wgpu::Device,
        a: &wgpu::TextureView,
        b: &wgpu::TextureView,
        c: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let tv = wgpu::BindingResource::TextureView;
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("postfx group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tv(a),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tv(b),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: tv(c),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
            ],
        })
    }

    /// Runs DoF and/or motion blur on `src`. Returns the index into `t.out` holding the
    /// result, or `None` if nothing ran.
    pub fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        t: &FxTargets,
        inputs: &FxInputs,
    ) -> Option<usize> {
        let post = inputs.post;
        let dof = &post.depth_of_field;
        // Pixel radii scale with the view height (URP tunes for 1080p).
        let scale = inputs.height as f32 / 1080.0;
        let (mode, a, b, max_radius) = match dof.mode {
            DofMode::Off => (0.0, 0.0, 0.0, 0.0),
            DofMode::Gaussian => (
                1.0,
                dof.gaussian_start,
                dof.gaussian_end,
                dof.gaussian_max_radius * 8.0 * scale,
            ),
            DofMode::Bokeh => {
                // Thin lens: CoC diameter on the sensor for an object at infinity, as a
                // radius in pixels; the shader scales it by (1 - focus / depth).
                let f = dof.focal_length.max(1.0) / 1000.0;
                let p = dof.focus_distance.max(f + 0.01);
                let coc = f * f / (dof.aperture.max(0.1) * (p - f));
                let px = coc / FILM_HEIGHT * inputs.height as f32 * 0.5;
                (2.0, p, px, 24.0 * scale)
            }
        };
        let mb_on = post.motion_blur[0] > 0.0;
        if mode == 0.0 && !mb_on {
            return None;
        }
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&FxUniform {
                inv_vp: inputs.inv_vp,
                prev_vp: inputs.prev_vp,
                dof: [mode, a, b, max_radius.max(1.0)],
                mb: [
                    post.motion_blur[0],
                    post.motion_blur[1].max(0.0),
                    inputs.motion.is_some() as u32 as f32,
                    MOTION_SAMPLES,
                ],
                proj: [
                    inputs.near,
                    inputs.far,
                    inputs.orthographic as u32 as f32,
                    0.0,
                ],
            }),
        );
        let mut current: Option<usize> = None;
        if mode > 0.0 {
            let g = self.group(device, &self.dummy, &t.depth, &self.dummy);
            pass(encoder, "dof coc", &t.coc, &self.coc, &g);
            let g = self.group(device, src, &t.coc, &self.dummy);
            pass(encoder, "dof prefilter", &t.half[0], &self.prefilter, &g);
            let g = self.group(device, &t.half[0], &self.dummy, &self.dummy);
            pass(encoder, "dof blur", &t.half[1], &self.blur, &g);
            let g = self.group(device, src, &t.coc, &t.half[1]);
            pass(encoder, "dof composite", &t.out[0], &self.composite, &g);
            current = Some(0);
        }
        if mb_on {
            let (input, out) = match current {
                Some(i) => (&t.out[i], 1 - i),
                None => (src, 0),
            };
            let motion = inputs.motion.unwrap_or(&self.dummy);
            let g = self.group(device, input, &t.depth, motion);
            pass(encoder, "motion blur", &t.out[out], &self.motion_blur, &g);
            current = Some(out);
        }
        current
    }
}
