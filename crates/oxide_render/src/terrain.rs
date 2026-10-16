//! GPU side of terrains: per-TerrainData splat map + layer texture array + uniforms, and the
//! terrain pipeline.

use oxide_assets::{AssetId, AssetServer, Texture2D};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Most layers a terrain draws (four control maps).
pub const MAX_LAYERS: usize = 16;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainUniform {
    /// World size xyz, w = layer count.
    size: [f32; 4],
    /// x = has holes, y = control map count.
    flags: [f32; 4],
    /// Per layer: 1/tile x, 1/tile z, smoothness, metallic.
    tiles: [[f32; 4]; MAX_LAYERS],
    /// Per layer: tile offset x, z, normal scale, has normal map.
    offsets: [[f32; 4]; MAX_LAYERS],
    tints: [[f32; 4]; MAX_LAYERS],
}

struct GpuTerrain {
    versions: oxide_assets::TerrainVersions,
    /// Kept alive for the bind group.
    _layers: wgpu::Texture,
    _normals: wgpu::Texture,
    control: wgpu::Texture,
    holes: wgpu::Texture,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// (control map count, resolution) and holes size the textures were made for.
    control_shape: (u32, u32),
    holes_size: u32,
}

pub struct TerrainGpu {
    pub layout: wgpu::BindGroupLayout,
    cache: HashMap<AssetId, GpuTerrain>,
    layer_sampler: wgpu::Sampler,
    control_sampler: wgpu::Sampler,
    holes_sampler: wgpu::Sampler,
}

/// Resizes RGBA8 pixels with bilinear filtering.
fn resize(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for y in 0..dh {
        for x in 0..dw {
            let fx = (x as f32 + 0.5) / dw as f32 * sw as f32 - 0.5;
            let fy = (y as f32 + 0.5) / dh as f32 * sh as f32 - 0.5;
            let (x0, y0) = (fx.floor().max(0.0) as u32, fy.floor().max(0.0) as u32);
            let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
            let (tx, ty) = (
                (fx - x0 as f32).clamp(0.0, 1.0),
                (fy - y0 as f32).clamp(0.0, 1.0),
            );
            for c in 0..4 {
                let p = |xx: u32, yy: u32| src[((yy * sw + xx) * 4 + c) as usize] as f32;
                let v = (p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx) * (1.0 - ty)
                    + (p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx) * ty;
                out[((y * dw + x) * 4 + c) as usize] = v.round() as u8;
            }
        }
    }
    out
}

impl TerrainGpu {
    pub fn new(device: &wgpu::Device) -> Self {
        let tex = |binding, dim| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: dim,
                multisampled: false,
            },
            count: None,
        };
        let samp = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex(1, wgpu::TextureViewDimension::D2Array),
                tex(2, wgpu::TextureViewDimension::D2Array),
                samp(3),
                samp(4),
                tex(5, wgpu::TextureViewDimension::D2Array),
                tex(6, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });
        let layer_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain layers"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let control_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain control"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let holes_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain holes"),
            ..Default::default()
        });
        Self {
            layout,
            cache: HashMap::new(),
            layer_sampler,
            control_sampler,
            holes_sampler,
        }
    }

    /// Builds (or refreshes) the GPU data of a TerrainData: layer textures when the layers
    /// change, control maps and holes by uploading new pixels while painting. Returns false
    /// if it doesn't exist.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &AssetServer,
        id: AssetId,
    ) -> bool {
        let Some(data) = assets.terrains.get_by_id(id) else {
            return false;
        };
        let v = data.versions;
        let cached = self.cache.get(&id);
        let rebuild_layers = cached.is_none_or(|g| g.versions.layers != v.layers);
        let control_shape = (
            data.alphamaps.len().max(1) as u32,
            data.splat_resolution.max(1),
        );
        let holes_size = if data.holes.is_empty() {
            1
        } else {
            data.resolution - 1
        };
        let reshape =
            cached.is_none_or(|g| g.control_shape != control_shape || g.holes_size != holes_size);
        if rebuild_layers || reshape {
            let g = self.build(device, queue, assets, data, control_shape, holes_size);
            self.cache.insert(id, g);
            return true;
        }
        let g = self.cache.get_mut(&id).expect("cached");
        if g.versions.alphamaps != v.alphamaps {
            write_control(queue, &g.control, data);
        }
        if g.versions.holes != v.holes {
            write_holes(queue, &g.holes, data);
            queue.write_buffer(&g.uniform, 0, bytemuck::bytes_of(&uniform(data)));
        }
        g.versions = v;
        true
    }

    fn build(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &AssetServer,
        data: &oxide_assets::TerrainData,
        control_shape: (u32, u32),
        holes_size: u32,
    ) -> GpuTerrain {
        let size = 512u32;
        let layers: Vec<&oxide_assets::TerrainLayer> =
            data.layers.iter().take(MAX_LAYERS).collect();
        let count = layers.len().max(1);
        let pixels = |h: Option<&oxide_assets::Handle<Texture2D>>, fallback: [u8; 4]| {
            let src = h
                .and_then(|h| assets.textures.get(h))
                // Pre-compressed (KTX2) textures have no CPU pixels to build the array from.
                .filter(|t| !t.mips.is_empty());
            match src {
                Some(t) => resize(&t.mips[0], t.width, t.height, size, size),
                None => fallback.repeat((size * size) as usize),
            }
        };
        let make_array = |label: &str, srgb: bool, f: &dyn Fn(usize) -> Vec<u8>| {
            let mut bytes = Vec::new();
            let mut mip_count = 1;
            for i in 0..count {
                let t = Texture2D::from_rgba8(label, size, size, f(i), srgb);
                mip_count = t.mip_count();
                for m in &t.mips {
                    bytes.extend_from_slice(m);
                }
            }
            device.create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: count as u32,
                    },
                    mip_level_count: mip_count,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: if srgb {
                        wgpu::TextureFormat::Rgba8UnormSrgb
                    } else {
                        wgpu::TextureFormat::Rgba8Unorm
                    },
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &bytes,
            )
        };
        let array = make_array("terrain layers", true, &|i| {
            pixels(layers.get(i).and_then(|l| l.diffuse.as_ref()), [255; 4])
        });
        let normals = make_array("terrain normals", false, &|i| {
            pixels(
                layers.get(i).and_then(|l| l.normal_map.as_ref()),
                [128, 128, 255, 255],
            )
        });
        let (maps, s) = control_shape;
        let control = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terrain control"),
            size: wgpu::Extent3d {
                width: s,
                height: s,
                depth_or_array_layers: maps,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write_control(queue, &control, data);
        let holes = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terrain holes"),
            size: wgpu::Extent3d {
                width: holes_size,
                height: holes_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write_holes(queue, &holes, data);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain uniform"),
            contents: bytemuck::bytes_of(&uniform(data)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let array_view = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let (layers_view, normals_view, control_view) = (
            array_view(&array),
            array_view(&normals),
            array_view(&control),
        );
        let holes_view = holes.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&control_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&layers_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.layer_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.control_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&normals_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&holes_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&self.holes_sampler),
                },
            ],
        });
        GpuTerrain {
            versions: data.versions,
            _layers: array,
            _normals: normals,
            control,
            holes,
            uniform: buffer,
            bind_group,
            control_shape,
            holes_size,
        }
    }

    pub fn bind_group(&self, id: AssetId) -> Option<&wgpu::BindGroup> {
        self.cache.get(&id).map(|g| &g.bind_group)
    }
}

fn uniform(data: &oxide_assets::TerrainData) -> TerrainUniform {
    let layers: Vec<&oxide_assets::TerrainLayer> = data.layers.iter().take(MAX_LAYERS).collect();
    let mut u = TerrainUniform {
        size: [data.size.x, data.size.y, data.size.z, layers.len() as f32],
        flags: [
            f32::from(u8::from(!data.holes.is_empty())),
            data.alphamaps.len() as f32,
            0.0,
            0.0,
        ],
        tiles: [[0.1, 0.1, 0.1, 0.0]; MAX_LAYERS],
        offsets: [[0.0; 4]; MAX_LAYERS],
        tints: [[1.0; 4]; MAX_LAYERS],
    };
    for (i, l) in layers.iter().enumerate() {
        u.tiles[i] = [
            1.0 / l.tile_size.x.max(0.01),
            1.0 / l.tile_size.y.max(0.01),
            l.smoothness,
            l.metallic,
        ];
        u.offsets[i] = [
            l.tile_offset.x,
            l.tile_offset.y,
            l.normal_scale,
            f32::from(u8::from(l.normal_map.is_some())),
        ];
        let c = l.tint.linear();
        u.tints[i] = [c.r, c.g, c.b, 1.0];
    }
    u
}

fn write_control(queue: &wgpu::Queue, tex: &wgpu::Texture, data: &oxide_assets::TerrainData) {
    let s = data.splat_resolution.max(1);
    for (i, m) in data.alphamaps.iter().enumerate() {
        if i as u32 >= tex.depth_or_array_layers() || m.len() as u32 != s * s {
            continue;
        }
        let bytes: &[u8] = bytemuck::cast_slice(m.as_slice());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: i as u32,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(s * 4),
                rows_per_image: Some(s),
            },
            wgpu::Extent3d {
                width: s,
                height: s,
                depth_or_array_layers: 1,
            },
        );
    }
}

fn write_holes(queue: &wgpu::Queue, tex: &wgpu::Texture, data: &oxide_assets::TerrainData) {
    let w = tex.width();
    let solid = [255u8];
    let bytes: Vec<u8> = if data.holes.is_empty() || w != data.resolution - 1 {
        solid.repeat((w * w) as usize)
    } else {
        data.holes
            .iter()
            .map(|&h| if h != 0 { 255 } else { 0 })
            .collect()
    };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w),
            rows_per_image: Some(w),
        },
        wgpu::Extent3d {
            width: w,
            height: w,
            depth_or_array_layers: 1,
        },
    );
}
