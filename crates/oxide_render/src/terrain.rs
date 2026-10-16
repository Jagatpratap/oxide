//! GPU side of terrains: per-TerrainData splat map + layer texture array + uniforms, and the
//! terrain pipeline.

use oxide_assets::{AssetId, AssetServer, Texture2D};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainUniform {
    size: [f32; 4],
    tiles: [[f32; 4]; 4],
    tints: [[f32; 4]; 4],
}

struct GpuTerrain {
    version: u64,
    bind_group: wgpu::BindGroup,
}

pub struct TerrainGpu {
    pub layout: wgpu::BindGroupLayout,
    cache: HashMap<AssetId, GpuTerrain>,
    layer_sampler: wgpu::Sampler,
    control_sampler: wgpu::Sampler,
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
                tex(1, wgpu::TextureViewDimension::D2),
                tex(2, wgpu::TextureViewDimension::D2Array),
                samp(3),
                samp(4),
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
        Self {
            layout,
            cache: HashMap::new(),
            layer_sampler,
            control_sampler,
        }
    }

    /// Builds (or refreshes) the GPU data of a TerrainData. Returns false if it doesn't exist.
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
        if self
            .cache
            .get(&id)
            .is_some_and(|g| g.version == data.version)
        {
            return true;
        }
        let size = 512u32;
        let layers: Vec<&oxide_assets::TerrainLayer> = data.layers.iter().take(4).collect();
        let count = layers.len().max(1);
        let mut bytes = Vec::new();
        let mut mip_count = 1;
        for i in 0..count {
            let src: Option<&Texture2D> = layers
                .get(i)
                .and_then(|l| l.diffuse.as_ref())
                .and_then(|h| assets.textures.get(h))
                // Pre-compressed (KTX2) textures have no CPU pixels to build the array from.
                .filter(|t| !t.mips.is_empty());
            let base = match src {
                Some(t) => resize(&t.mips[0], t.width, t.height, size, size),
                None => vec![255u8; (size * size * 4) as usize],
            };
            let t = Texture2D::from_rgba8("terrain layer", size, size, base, true);
            mip_count = t.mip_count();
            for m in &t.mips {
                bytes.extend_from_slice(m);
            }
        }
        let array = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("terrain layers"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: count as u32,
                },
                mip_level_count: mip_count,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &bytes,
        );
        let s = data.splat_resolution.max(1);
        let control_bytes: Vec<u8> = data.splat.iter().flat_map(|w| *w).collect();
        let control = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("terrain control"),
                size: wgpu::Extent3d {
                    width: s,
                    height: s,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &control_bytes,
        );
        let mut u = TerrainUniform {
            size: [data.size.x, data.size.y, data.size.z, layers.len() as f32],
            tiles: [[0.1, 0.1, 0.1, 0.0]; 4],
            tints: [[1.0; 4]; 4],
        };
        for (i, l) in layers.iter().enumerate() {
            u.tiles[i] = [
                1.0 / l.tile_size.x.max(0.01),
                1.0 / l.tile_size.y.max(0.01),
                l.smoothness,
                l.metallic,
            ];
            let c = l.tint.linear();
            u.tints[i] = [c.r, c.g, c.b, 1.0];
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain uniform"),
            contents: bytemuck::bytes_of(&u),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let array_view = array.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let control_view = control.create_view(&Default::default());
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
                    resource: wgpu::BindingResource::TextureView(&array_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.layer_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.control_sampler),
                },
            ],
        });
        self.cache.insert(
            id,
            GpuTerrain {
                version: data.version,
                bind_group,
            },
        );
        true
    }

    pub fn bind_group(&self, id: AssetId) -> Option<&wgpu::BindGroup> {
        self.cache.get(&id).map(|g| &g.bind_group)
    }
}
