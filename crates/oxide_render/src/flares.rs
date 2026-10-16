//! Lens flares (Unity URP's Lens Flare (SRP)): each source's elements are placed along the
//! line from the source through the screen centre, then drawn as procedural shapes or
//! textures over the HDR image before bloom. Occlusion reads the scene depth on the GPU, so
//! a flare fades as its source goes behind geometry.

use crate::frame::CameraView;
use crate::renderer::HDR_FORMAT;
use oxide_assets::{AssetId, AssetServer};
use oxide_math::{Vector2, Vector3};
use std::collections::HashMap;

/// Samples in a curve lookup table (curves are sampled on the CPU at extraction).
pub const FLARE_LUT: usize = 16;
/// A curve sampled at `FLARE_LUT` evenly spaced points over 0..1.
pub type FlareLut = [f32; FLARE_LUT];

/// Linear lookup into a [`FlareLut`] at `t` (clamped to 0..1).
pub fn sample_lut(lut: &FlareLut, t: f32) -> f32 {
    let x = t.clamp(0.0, 1.0) * (FLARE_LUT - 1) as f32;
    let i = (x.floor() as usize).min(FLARE_LUT - 2);
    let f = x - i as f32;
    lut[i] + (lut[i + 1] - lut[i]) * f
}

/// Unity `SRPLensFlareType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlareShape {
    #[default]
    Image,
    Circle,
    Polygon,
    Ring,
}

/// Unity `SRPLensFlareBlendMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FlareBlend {
    #[default]
    Additive,
    Screen,
    Premultiply,
    Lerp,
}

impl FlareBlend {
    const ALL: [FlareBlend; 4] = [
        FlareBlend::Additive,
        FlareBlend::Screen,
        FlareBlend::Premultiply,
        FlareBlend::Lerp,
    ];

    fn state(self) -> wgpu::BlendState {
        use wgpu::{BlendComponent, BlendFactor as F, BlendOperation::Add};
        let c = |src, dst| BlendComponent {
            src_factor: src,
            dst_factor: dst,
            operation: Add,
        };
        let (color, alpha) = match self {
            FlareBlend::Additive => (c(F::One, F::One), c(F::Zero, F::One)),
            FlareBlend::Screen => (c(F::One, F::OneMinusSrc), c(F::Zero, F::One)),
            FlareBlend::Premultiply => (c(F::One, F::OneMinusSrcAlpha), c(F::Zero, F::One)),
            FlareBlend::Lerp => (c(F::SrcAlpha, F::OneMinusSrcAlpha), c(F::Zero, F::One)),
        };
        wgpu::BlendState { color, alpha }
    }
}

/// Radial distortion of an element's size (Unity "Enable Radial Distortion").
#[derive(Clone, Debug, PartialEq)]
pub struct FlareDistortion {
    /// Size at the edge of the screen (fractions of the screen height).
    pub target_size: [f32; 2],
    /// Blend from `size` (0) to `target_size` (1) by distance from the centre (or source).
    pub curve: FlareLut,
    pub relative_to_center: bool,
}

/// One drawn element, already expanded from the asset's "Multiple Elements" settings.
#[derive(Clone, Debug, PartialEq)]
pub struct FlareElement {
    pub texture: Option<AssetId>,
    pub shape: FlareShape,
    pub blend: FlareBlend,
    /// Linear HDR colour (tint × intensity × gradient); alpha is opacity.
    pub color: [f32; 4],
    pub modulate_by_light_color: bool,
    /// Along the ray from the source (0) through the screen centre (1) and beyond.
    pub position: f32,
    /// Offset in screen-height units.
    pub position_offset: [f32; 2],
    /// Rotates the ray around the source (degrees).
    pub angular_offset: f32,
    pub translation_scale: [f32; 2],
    /// Width and height; 1 is a tenth of the screen height.
    pub size: [f32; 2],
    /// Image: keep the texture's aspect ratio.
    pub preserve_aspect_ratio: bool,
    /// Degrees.
    pub rotation: f32,
    /// Turns the element to face the source.
    pub auto_rotate: bool,
    /// Circle / Polygon / Ring: (gradient, edge offset, roundness, side count).
    pub shape_params: [f32; 4],
    /// Draws the shape's outside instead of its inside.
    pub inverse: bool,
    pub distortion: Option<FlareDistortion>,
}

/// Screen-space occlusion of a source (Unity "Occlusion" foldout).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlareOcclusion {
    /// World-space radius sampled around the source.
    pub radius: f32,
    pub samples: u32,
    /// Moves the tested point toward the camera (world units).
    pub offset: f32,
}

/// One Lens Flare component, ready to lay out for a camera.
#[derive(Clone, Debug, PartialEq)]
pub struct LensFlareSource {
    /// World position; for directional sources, the direction toward the source.
    pub position: Vector3,
    pub directional: bool,
    pub light_color: [f32; 3],
    pub intensity: f32,
    pub scale: f32,
    /// Spot light shape: (forward, cos of the outer half angle, cos of the inner one).
    pub spot: Option<(Vector3, f32, f32)>,
    pub max_attenuation_distance: f32,
    /// Intensity by distance / `max_attenuation_distance`.
    pub distance_attenuation: FlareLut,
    pub max_attenuation_scale: f32,
    /// Scale by distance / `max_attenuation_scale`.
    pub scale_by_distance: FlareLut,
    /// Intensity by the source's distance from the screen centre (0) to its edge (1).
    pub radial_screen_attenuation: FlareLut,
    pub occlusion: Option<FlareOcclusion>,
    /// Keeps drawing while the source is outside the screen.
    pub allow_off_screen: bool,
    pub elements: Vec<FlareElement>,
}

/// GPU data of one element quad.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FlareInstance {
    /// NDC centre (xy) and half size in screen-height NDC units (zw).
    pub center_size: [f32; 4],
    pub color: [f32; 4],
    /// Gradient, edge offset, roundness, side count.
    pub shape: [f32; 4],
    /// Shape kind, inverse, rotation (radians), aspect (width / height).
    pub misc: [f32; 4],
    /// Source NDC (xy), source depth (reverse-Z), sample radius in NDC-height units.
    pub occlusion: [f32; 4],
    /// Sample count (0 = no occlusion), blend mode, unused × 2.
    pub occlusion2: [f32; 4],
}

/// Half size, in NDC-height units, of an element of size 1 (a tenth of the screen height).
const UNIT_HALF_SIZE: f32 = 0.1;

fn rotate(v: Vector2, deg: f32) -> Vector2 {
    let (s, c) = deg.to_radians().sin_cos();
    Vector2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// Lays out every flare for `cam` on a view of `aspect` (width / height). Returns the
/// element quads with their textures, in source then element order.
pub fn layout(
    sources: &[LensFlareSource],
    cam: &CameraView,
    aspect: f32,
    texture_aspect: &dyn Fn(AssetId) -> Option<f32>,
) -> Vec<(Option<AssetId>, FlareBlend, FlareInstance)> {
    let vp = cam.view_projection(aspect);
    let up = cam.rotation * Vector3::Y;
    let mut out = Vec::new();
    for src in sources {
        let clip = if src.directional {
            vp * src.position.normalize_or(Vector3::Z).extend(0.0)
        } else {
            vp * src.position.extend(1.0)
        };
        if clip.w <= 1e-5 {
            continue; // behind the camera
        }
        let ndc = Vector2::new(clip.x / clip.w, clip.y / clip.w);
        if !src.allow_off_screen && (ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0) {
            continue;
        }
        let dist = if src.directional {
            0.0
        } else {
            src.position.distance(cam.position)
        };
        let ratio = |max: f32| if max > 0.0 { dist / max } else { 0.0 };
        let mut intensity = src.intensity
            * sample_lut(
                &src.distance_attenuation,
                ratio(src.max_attenuation_distance),
            )
            * sample_lut(&src.radial_screen_attenuation, ndc.length().min(1.0));
        if let Some((fwd, cos_outer, cos_inner)) = src.spot {
            let to_cam = (cam.position - src.position).normalize_or(Vector3::Z);
            let c = fwd.dot(to_cam);
            let t = ((c - cos_outer) / (cos_inner - cos_outer).max(1e-4)).clamp(0.0, 1.0);
            intensity *= t * t * (3.0 - 2.0 * t);
        }
        if intensity <= 1e-5 {
            continue;
        }
        let scale =
            src.scale * sample_lut(&src.scale_by_distance, ratio(src.max_attenuation_scale));

        // Occlusion: depth of the (offset) source and the sample radius on screen.
        let (depth, radius, samples) = match src.occlusion {
            Some(o) if o.samples > 0 => {
                if src.directional {
                    let r = o.radius.to_radians().tan().max(0.0);
                    let dir = src.position.normalize_or(Vector3::Z);
                    let side = vp * (dir + up * r).extend(0.0);
                    let rr = if side.w > 1e-5 {
                        (side.y / side.w - ndc.y)
                            .abs()
                            .max((side.x / side.w - ndc.x).abs())
                    } else {
                        0.0
                    };
                    (0.0, rr, o.samples)
                } else {
                    let toward = (cam.position - src.position).normalize_or(Vector3::Z);
                    let p = src.position + toward * o.offset.min(dist * 0.99);
                    let c = vp * p.extend(1.0);
                    let s = vp * (p + up * o.radius).extend(1.0);
                    let d = if c.w > 1e-5 { c.z / c.w } else { 0.0 };
                    let rr = if s.w > 1e-5 && c.w > 1e-5 {
                        (s.y / s.w - c.y / c.w).abs()
                    } else {
                        0.0
                    };
                    (d, rr, o.samples)
                }
            }
            _ => (0.0, 0.0, 0),
        };

        // Work in screen-height units so rotations and offsets aren't stretched.
        let s = Vector2::new(ndc.x * aspect, ndc.y);
        let max_len = Vector2::new(aspect, 1.0).length();
        for e in &src.elements {
            let ray = rotate(-s, e.angular_offset);
            let ts = Vector2::from(e.translation_scale);
            let center = s + ray * e.position * ts + Vector2::from(e.position_offset);
            let mut size = Vector2::from(e.size) * scale;
            if e.preserve_aspect_ratio
                && let Some(a) = e.texture.and_then(texture_aspect)
            {
                size.x = size.y * a;
            }
            if let Some(d) = &e.distortion {
                let r = if d.relative_to_center {
                    center.length()
                } else {
                    (center - s).length()
                } / max_len;
                let k = sample_lut(&d.curve, r);
                size = size.lerp(Vector2::from(d.target_size) * scale, k);
            }
            let mut rot = e.rotation;
            if e.auto_rotate {
                rot += (s - center).to_angle().to_degrees() - 90.0;
            }
            let lc = if e.modulate_by_light_color {
                src.light_color
            } else {
                [1.0; 3]
            };
            let col = [
                e.color[0] * lc[0] * intensity,
                e.color[1] * lc[1] * intensity,
                e.color[2] * lc[2] * intensity,
                e.color[3],
            ];
            if size.x <= 0.0 || size.y <= 0.0 || col[3] <= 0.0 {
                continue;
            }
            let half = size * UNIT_HALF_SIZE;
            out.push((
                if e.shape == FlareShape::Image {
                    e.texture
                } else {
                    None
                },
                e.blend,
                FlareInstance {
                    center_size: [center.x / aspect, center.y, half.x, half.y],
                    color: col,
                    shape: e.shape_params,
                    misc: [
                        e.shape as u32 as f32,
                        if e.inverse { 1.0 } else { 0.0 },
                        rot.to_radians(),
                        aspect,
                    ],
                    occlusion: [ndc.x, ndc.y, depth, radius],
                    occlusion2: [samples as f32, e.blend as u32 as f32, 0.0, 0.0],
                },
            ));
        }
    }
    out
}

/// GPU side: pipelines per blend mode, textures and the instance buffer.
pub struct FlareGpu {
    scene_layout: wgpu::BindGroupLayout,
    tex_layout: wgpu::BindGroupLayout,
    blit: wgpu::RenderPipeline,
    pipelines: HashMap<FlareBlend, wgpu::RenderPipeline>,
    groups: HashMap<Option<AssetId>, (wgpu::BindGroup, u64)>,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    no_depth: wgpu::TextureView,
    buffer: wgpu::Buffer,
}

impl FlareGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        use wgpu::util::DeviceExt;
        let tex_entry = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flare scene"),
            entries: &[tex_entry(0, true), sampler_entry, tex_entry(2, false)],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flare texture"),
            entries: &[tex_entry(0, true), sampler_entry],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flares.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/flares.wgsl").into()),
        });
        let make = |label, layouts: &[Option<&wgpu::BindGroupLayout>], vs, fs, blend, inst| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4];
            let buffers = [Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<FlareInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &attrs,
            })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: if inst { &buffers } else { &[] },
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let blit = make(
            "flare blit",
            &[Some(&scene_layout)],
            "vs_blit",
            "fs_blit",
            None,
            false,
        );
        let pipelines = FlareBlend::ALL
            .into_iter()
            .map(|b| {
                let p = make(
                    "flares",
                    &[Some(&scene_layout), Some(&tex_layout)],
                    "vs_flare",
                    "fs_flare",
                    Some(b.state()),
                    true,
                );
                (b, p)
            })
            .collect();
        let tex1 = |label, format, data: &[u8]| {
            device
                .create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    data,
                )
                .create_view(&Default::default())
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("flare sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self {
            scene_layout,
            tex_layout,
            blit,
            pipelines,
            groups: HashMap::new(),
            sampler,
            white: tex1(
                "flare white",
                wgpu::TextureFormat::Rgba8UnormSrgb,
                &[255; 4],
            ),
            no_depth: tex1("flare no depth", crate::postfx::DEPTH_COPY_FORMAT, &[0; 4]),
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("flares"),
                size: (std::mem::size_of::<FlareInstance>() * 64) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    fn texture_group(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut crate::gpu_assets::GpuAssets,
        assets: &AssetServer,
        tex: Option<AssetId>,
    ) -> bool {
        let version = tex.and_then(|id| assets.textures.version(id)).unwrap_or(0);
        if self.groups.get(&tex).is_some_and(|(_, v)| *v == version) {
            return true;
        }
        let view = match tex {
            Some(id) => match gpu_assets.texture_view(device, queue, assets, id) {
                Some(v) => v,
                None => return false,
            },
            None => &self.white,
        };
        let g = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flare texture"),
            layout: &self.tex_layout,
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

    /// Uploads the quads; returns draw ranges grouped by (texture, blend), in draw order.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_assets: &mut crate::gpu_assets::GpuAssets,
        assets: &AssetServer,
        quads: &[(Option<AssetId>, FlareBlend, FlareInstance)],
    ) -> Vec<(Option<AssetId>, FlareBlend, std::ops::Range<u32>)> {
        let mut all = Vec::with_capacity(quads.len());
        let mut ranges: Vec<(Option<AssetId>, FlareBlend, std::ops::Range<u32>)> = Vec::new();
        for (tex, blend, inst) in quads {
            if !self.texture_group(device, queue, gpu_assets, assets, *tex) {
                continue;
            }
            let at = all.len() as u32;
            all.push(*inst);
            match ranges.last_mut() {
                Some((t, b, r)) if t == tex && b == blend && r.end == at => r.end = at + 1,
                _ => ranges.push((*tex, *blend, at..at + 1)),
            }
        }
        let bytes = std::mem::size_of_val(all.as_slice()) as u64;
        if self.buffer.size() < bytes {
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("flares"),
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

    /// Copies `src` into `dst`, then draws the prepared quads on top.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        dst: &wgpu::TextureView,
        ranges: &[(Option<AssetId>, FlareBlend, std::ops::Range<u32>)],
    ) {
        let scene = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flare scene"),
            layout: &self.scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(depth.unwrap_or(&self.no_depth)),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Lens Flares"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dst,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_bind_group(0, &scene, &[]);
        pass.set_pipeline(&self.blit);
        pass.draw(0..3, 0..1);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        for (tex, blend, r) in ranges {
            let Some((g, _)) = self.groups.get(tex) else {
                continue;
            };
            pass.set_pipeline(&self.pipelines[blend]);
            pass.set_bind_group(1, g, &[]);
            pass.draw(0..6, r.clone());
        }
    }
}

/// Does any source need the scene depth?
pub fn needs_depth(sources: &[LensFlareSource]) -> bool {
    sources
        .iter()
        .any(|s| s.occlusion.is_some_and(|o| o.samples > 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{AntiAliasingSettings, ClearMode, ProjectionKind};
    use oxide_math::Quaternion;

    fn camera() -> CameraView {
        CameraView {
            position: Vector3::ZERO,
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Perspective { fov: 60.0 },
            near: 0.1,
            far: 1000.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Sky,
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: AntiAliasingSettings::default(),
        }
    }

    fn element(position: f32) -> FlareElement {
        FlareElement {
            texture: None,
            shape: FlareShape::Circle,
            blend: FlareBlend::Additive,
            color: [1.0; 4],
            modulate_by_light_color: false,
            position,
            position_offset: [0.0; 2],
            angular_offset: 0.0,
            translation_scale: [1.0; 2],
            size: [1.0; 2],
            preserve_aspect_ratio: false,
            rotation: 0.0,
            auto_rotate: false,
            shape_params: [1.0, 0.0, 0.0, 6.0],
            inverse: false,
            distortion: None,
        }
    }

    fn source(position: Vector3) -> LensFlareSource {
        LensFlareSource {
            position,
            directional: false,
            light_color: [1.0; 3],
            intensity: 1.0,
            scale: 1.0,
            spot: None,
            max_attenuation_distance: 100.0,
            distance_attenuation: [1.0; FLARE_LUT],
            max_attenuation_scale: 100.0,
            scale_by_distance: [1.0; FLARE_LUT],
            radial_screen_attenuation: [1.0; FLARE_LUT],
            occlusion: None,
            allow_off_screen: false,
            elements: vec![element(0.0), element(1.0), element(2.0)],
        }
    }

    #[test]
    fn elements_lie_on_the_line_through_the_screen_centre() {
        // Camera looks down +Z; a source up and to the right.
        let src = source(Vector3::new(3.0, 2.0, 10.0));
        let q = layout(&[src], &camera(), 16.0 / 9.0, &|_| None);
        assert_eq!(q.len(), 3);
        let p: Vec<Vector2> = q
            .iter()
            .map(|(_, _, i)| Vector2::new(i.center_size[0], i.center_size[1]))
            .collect();
        assert!(p[0].x > 0.0 && p[0].y > 0.0, "at the source: {:?}", p[0]);
        assert!(p[1].length() < 1e-4, "position 1 is the centre: {:?}", p[1]);
        assert!(
            (p[2] + p[0]).length() < 1e-4,
            "position 2 mirrors the source"
        );
    }

    #[test]
    fn off_screen_behind_and_attenuated_sources() {
        let cam = camera();
        let behind = source(Vector3::new(0.0, 0.0, -10.0));
        assert!(layout(&[behind], &cam, 1.0, &|_| None).is_empty());
        let mut off = source(Vector3::new(50.0, 0.0, 10.0));
        assert!(layout(&[off.clone()], &cam, 1.0, &|_| None).is_empty());
        off.allow_off_screen = true;
        assert_eq!(layout(&[off], &cam, 1.0, &|_| None).len(), 3);
        // Distance attenuation falls to 0 at max distance.
        let mut far = source(Vector3::new(0.0, 0.0, 50.0));
        far.max_attenuation_distance = 50.0;
        for (i, v) in far.distance_attenuation.iter_mut().enumerate() {
            *v = 1.0 - i as f32 / (FLARE_LUT - 1) as f32;
        }
        assert!(layout(&[far.clone()], &cam, 1.0, &|_| None).is_empty());
        far.max_attenuation_distance = 100.0;
        let q = layout(&[far], &cam, 1.0, &|_| None);
        assert!((q[0].2.color[0] - 0.5).abs() < 0.01, "{:?}", q[0].2.color);
    }

    #[test]
    fn spot_lights_only_flare_inside_their_cone() {
        let cam = camera();
        let mut s = source(Vector3::new(0.0, 0.0, 10.0));
        // Facing the camera (-Z), 30° outer cone.
        s.spot = Some((
            -Vector3::Z,
            15f32.to_radians().cos(),
            10f32.to_radians().cos(),
        ));
        assert_eq!(layout(&[s.clone()], &cam, 1.0, &|_| None).len(), 3);
        s.spot = Some((
            Vector3::X,
            15f32.to_radians().cos(),
            10f32.to_radians().cos(),
        ));
        assert!(layout(&[s], &cam, 1.0, &|_| None).is_empty());
    }

    #[test]
    fn lut_sampling() {
        let mut l = [0.0; FLARE_LUT];
        for (i, v) in l.iter_mut().enumerate() {
            *v = i as f32;
        }
        assert_eq!(sample_lut(&l, 0.0), 0.0);
        assert_eq!(sample_lut(&l, 1.0), 15.0);
        assert!((sample_lut(&l, 0.5) - 7.5).abs() < 1e-5);
        assert_eq!(sample_lut(&l, 2.0), 15.0);
    }
}
