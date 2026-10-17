//! 3D text (TextMeshPro components in the world): SDF glyph quads, alpha blended after
//! transparent objects, depth tested, back to front.

use crate::gpu_assets::GpuAssets;
use crate::renderer::{DEPTH_FORMAT, HDR_FORMAT};
use oxide_assets::{AssetId, AssetServer};
use oxide_math::Vector3;
use std::collections::HashMap;

/// The shared SDF shading functions (also used by Canvas text).
pub const SDF_WGSL: &str = include_str!("shaders/text_sdf.wgsl");

/// TextMeshPro material settings, in the form the shader takes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SdfMaterial {
    /// Linear face colour (multiplied by each glyph's vertex colour).
    pub face: [f32; 4],
    pub outline: [f32; 4],
    pub underlay: [f32; 4],
    /// -1..1.
    pub dilate: f32,
    /// 0..1.
    pub softness: f32,
    /// 0..1.
    pub outline_width: f32,
    pub underlay_on: bool,
    pub underlay_offset: [f32; 2],
    pub underlay_dilate: f32,
    pub underlay_softness: f32,
}

/// One glyph quad in world space.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextInstance {
    /// Corners bottom-left, bottom-right, top-right, top-left (xyz each), packed.
    pub corners: [[f32; 4]; 3],
    /// Atlas rectangle (min x, min y, max x, max y), normalized.
    pub uv: [f32; 4],
    pub face: [f32; 4],
    pub outline: [f32; 4],
    pub underlay: [f32; 4],
    /// Dilate, softness, outline width, spread (atlas px).
    pub shape: [f32; 4],
    /// Underlay offset xy, dilate, softness.
    pub under: [f32; 4],
    /// Atlas size, underlay on, pass (0 = all, 1 = underlay only, 2 = face only,
    /// 3 = sprite: plain texture colours).
    pub atlas: [f32; 4],
    /// Corner colours bl, br, tr, tl as packed sRGB RGBA8 (Color Gradients), multiplying
    /// the face colour.
    pub tint: [u32; 4],
}

/// Packs an sRGB colour as RGBA8 (the `tint` format).
pub fn pack_rgba8(c: [f32; 4]) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    b(c[0]) | (b(c[1]) << 8) | (b(c[2]) << 16) | (b(c[3]) << 24)
}

impl TextInstance {
    /// This glyph's shadow alone (drawn for a whole text before its faces, so shadows
    /// never cover neighbouring letters).
    pub fn underlay_only(mut self) -> Self {
        self.atlas[3] = 1.0;
        self
    }

    /// This glyph without its shadow.
    pub fn face_only(mut self) -> Self {
        self.atlas[3] = 2.0;
        self
    }
}

impl TextInstance {
    /// A glyph from its four world corners (bl, br, tr, tl).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        corners: [Vector3; 4],
        uv: [f32; 4],
        color: [f32; 4],
        bold_dilate: f32,
        m: &SdfMaterial,
        spread: f32,
        atlas_size: [f32; 2],
    ) -> Self {
        let c = corners;
        Self {
            corners: [
                [c[0].x, c[0].y, c[0].z, c[1].x],
                [c[1].y, c[1].z, c[2].x, c[2].y],
                [c[2].z, c[3].x, c[3].y, c[3].z],
            ],
            uv,
            face: [
                m.face[0] * color[0],
                m.face[1] * color[1],
                m.face[2] * color[2],
                m.face[3] * color[3],
            ],
            outline: [
                m.outline[0],
                m.outline[1],
                m.outline[2],
                m.outline[3] * color[3],
            ],
            underlay: [
                m.underlay[0],
                m.underlay[1],
                m.underlay[2],
                m.underlay[3] * color[3],
            ],
            shape: [
                (m.dilate + bold_dilate).clamp(-1.0, 1.0),
                m.softness,
                m.outline_width,
                spread,
            ],
            under: [
                m.underlay_offset[0],
                m.underlay_offset[1],
                m.underlay_dilate,
                m.underlay_softness,
            ],
            atlas: [
                atlas_size[0],
                atlas_size[1],
                if m.underlay_on { 1.0 } else { 0.0 },
                0.0,
            ],
            tint: [u32::MAX; 4],
        }
    }

    /// A `<sprite>` quad: samples the texture's colours times `color` (linear).
    pub fn sprite(corners: [Vector3; 4], uv: [f32; 4], color: [f32; 4]) -> Self {
        let c = corners;
        Self {
            corners: [
                [c[0].x, c[0].y, c[0].z, c[1].x],
                [c[1].y, c[1].z, c[2].x, c[2].y],
                [c[2].z, c[3].x, c[3].y, c[3].z],
            ],
            uv,
            face: color,
            atlas: [1.0, 1.0, 0.0, 3.0],
            tint: [u32::MAX; 4],
            ..Default::default()
        }
    }

    /// Per-corner colour multipliers (sRGB; bl, br, tr, tl).
    pub fn with_tint(mut self, corners: [[f32; 4]; 4]) -> Self {
        self.tint = corners.map(pack_rgba8);
        self
    }
}

/// One text object's glyphs on one atlas page.
#[derive(Clone, Debug)]
pub struct TextDraw {
    pub texture: AssetId,
    /// Where the text is, for back-to-front sorting.
    pub center: Vector3,
    pub instances: Vec<TextInstance>,
}

pub struct TextGpu {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    pipelines: HashMap<u32, wgpu::RenderPipeline>,
    groups: HashMap<AssetId, (wgpu::BindGroup, u64)>,
    sampler: wgpu::Sampler,
    buffer: wgpu::Buffer,
}

impl TextGpu {
    pub fn new(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        module: wgpu::ShaderModule,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text layout"),
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
            label: Some("text pipeline layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("text sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        Self {
            layout,
            pipeline_layout,
            module,
            pipelines: HashMap::new(),
            groups: HashMap::new(),
            sampler,
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("text"),
                size: (std::mem::size_of::<TextInstance>() * 256) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    pub fn ensure_pipelines(&mut self, device: &wgpu::Device, samples: u32) {
        if self.pipelines.contains_key(&samples) {
            return;
        }
        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TextInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
                        4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
                        8 => Float32x4, 9 => Float32x4, 10 => Uint32x4
                    ],
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
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(premultiplied),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(samples, p);
    }

    fn group(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut GpuAssets,
        assets: &AssetServer,
        tex: AssetId,
    ) -> bool {
        let version = assets.textures.version(tex).unwrap_or(0);
        if self.groups.get(&tex).is_some_and(|(_, v)| *v == version) {
            return true;
        }
        let Some(view) = gpu_assets.texture_view(device, queue, assets, tex) else {
            return false;
        };
        let g = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text atlas"),
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
        self.groups.insert(tex, (g, version));
        true
    }

    /// Sorts back to front, uploads, and returns draw ranges per atlas page.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut GpuAssets,
        assets: &AssetServer,
        draws: &[TextDraw],
        camera: Vector3,
    ) -> Vec<(AssetId, std::ops::Range<u32>)> {
        if draws.is_empty() {
            return Vec::new();
        }
        let mut order: Vec<usize> = (0..draws.len()).collect();
        order.sort_by(|&a, &b| {
            draws[b]
                .center
                .distance_squared(camera)
                .total_cmp(&draws[a].center.distance_squared(camera))
                .then(a.cmp(&b))
        });
        let mut all: Vec<TextInstance> = Vec::new();
        let mut ranges: Vec<(AssetId, std::ops::Range<u32>)> = Vec::new();
        for i in order {
            let d = &draws[i];
            if d.instances.is_empty() || !self.group(device, queue, gpu_assets, assets, d.texture) {
                continue;
            }
            let at = all.len() as u32;
            all.extend_from_slice(&d.instances);
            let end = all.len() as u32;
            match ranges.last_mut() {
                Some((t, r)) if *t == d.texture && r.end == at => r.end = end,
                _ => ranges.push((d.texture, at..end)),
            }
        }
        let bytes = std::mem::size_of_val(all.as_slice()) as u64;
        if self.buffer.size() < bytes {
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("text"),
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
        ranges: &[(AssetId, std::ops::Range<u32>)],
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
