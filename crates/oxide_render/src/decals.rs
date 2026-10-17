//! URP's Decal Projector (DBuffer technique): decals render into a screen-sized buffer
//! from the depth prepass, and Lit surfaces blend it into their albedo before lighting, so
//! decals are lit and shadowed like the surface under them and never touch transparents.

use crate::gpu_assets::GpuAssets;
use oxide_assets::{AssetId, AssetServer};
use oxide_math::{Matrix4x4, Vector3};

/// Format of the decal buffer: premultiplied albedo and coverage.
pub const DBUFFER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The decal prepass depth (when SSAO's isn't there) and the DBuffer.
pub(crate) struct DecalTargets {
    pub depth: wgpu::TextureView,
    pub dbuffer: wgpu::TextureView,
}

impl DecalTargets {
    pub fn new(device: &wgpu::Device, w: u32, h: u32) -> Self {
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
        Self {
            depth: tex("decal depth prepass", crate::renderer::DEPTH_FORMAT),
            dbuffer: tex("dbuffer", DBUFFER_FORMAT),
        }
    }
}

/// One Decal Projector in the frame.
#[derive(Clone, Debug)]
pub struct DecalDraw {
    /// The unit box (-0.5..0.5) → world; the decal projects along its local -Z.
    pub to_world: Matrix4x4,
    pub texture: Option<AssetId>,
    /// Linear colour; alpha includes the opacity and distance fade.
    pub color: [f32; 4],
    /// UV scale (xy) and offset (zw).
    pub uv: [f32; 4],
    /// Angle fade start and end (degrees between the surface and the projector).
    pub angle_fade: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DecalUniform {
    to_local: [f32; 16],
    to_world: [f32; 16],
    color: [f32; 4],
    uv: [f32; 4],
    fade: [f32; 4],
    axis: [f32; 4],
}

pub struct DecalGpu {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
}

/// A decal ready to draw: its bind group (uniform, texture, sampler, depth).
pub struct PreparedDecal {
    group: wgpu::BindGroup,
}

impl DecalGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame_layout: &wgpu::BindGroupLayout,
        module: wgpu::ShaderModule,
    ) -> Self {
        let tex = |binding, sample_type, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("decal layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex(
                    1,
                    wgpu::TextureSampleType::Float { filterable: true },
                    wgpu::TextureViewDimension::D2,
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(
                    3,
                    wgpu::TextureSampleType::Depth,
                    wgpu::TextureViewDimension::D2,
                ),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("decal pipeline layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout)],
            immediate_size: 0,
        });
        let over = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("decals"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            // Back faces, so the camera may be inside a decal's box.
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Front),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: DBUFFER_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: over,
                        alpha: over,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("decal sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        use wgpu::util::DeviceExt;
        let white = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("decal white"),
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
        Self {
            layout,
            pipeline,
            sampler,
            white,
        }
    }

    /// Uniforms and bind groups for this frame's decals (`depth`: the prepass depth).
    pub fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut GpuAssets,
        assets: &AssetServer,
        decals: &[DecalDraw],
        depth: &wgpu::TextureView,
    ) -> Vec<PreparedDecal> {
        use wgpu::util::DeviceExt;
        // Texture views first (loading needs `gpu_assets` mutably).
        for d in decals {
            if let Some(t) = d.texture {
                let _ = gpu_assets.texture_view(device, queue, assets, t);
            }
        }
        decals
            .iter()
            .map(|d| {
                let axis = d
                    .to_world
                    .transform_vector3(Vector3::NEG_Z)
                    .normalize_or_zero();
                let u = DecalUniform {
                    to_local: d.to_world.inverse().to_cols_array(),
                    to_world: d.to_world.to_cols_array(),
                    color: d.color,
                    uv: d.uv,
                    fade: [d.angle_fade[0], d.angle_fade[1], 0.0, 0.0],
                    axis: [axis.x, axis.y, axis.z, 0.0],
                };
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("decal"),
                    contents: bytemuck::bytes_of(&u),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let view = d
                    .texture
                    .and_then(|t| gpu_assets.cached_texture_view(t))
                    .unwrap_or(&self.white);
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("decal"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(depth),
                        },
                    ],
                });
                PreparedDecal { group }
            })
            .collect()
    }

    /// Draws the decals into the DBuffer (cleared first).
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame_group: &wgpu::BindGroup,
        dbuffer: &wgpu::TextureView,
        decals: &[PreparedDecal],
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("decals"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dbuffer,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, frame_group, &[]);
        for d in decals {
            pass.set_bind_group(1, &d.group, &[]);
            pass.draw(0..36, 0..1);
        }
    }
}
