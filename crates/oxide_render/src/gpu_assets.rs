//! GPU copies of meshes, textures and materials, uploaded on first use and refreshed when
//! the asset's version changes.

use oxide_assets::{
    AssetId, AssetServer, FilterMode, Material, Mesh, ShaderKind, SurfaceType, Texture2D,
    TextureWrapMode,
};
use oxide_math::Bounds;
use std::collections::HashMap;
use std::ops::Range;
use wgpu::util::DeviceExt;

/// Interleaved static vertex (slot 0): position, normal, tangent, uv, uv2, color.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv: [f32; 2],
    pub uv2: [f32; 2],
    pub color: [f32; 4],
}

/// Skinning attributes (slot 2).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinVertex {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

pub struct GpuMesh {
    pub version: u64,
    pub vertices: wgpu::Buffer,
    pub skin: Option<wgpu::Buffer>,
    pub indices: wgpu::Buffer,
    pub submeshes: Vec<Range<u32>>,
    pub bounds: Bounds,
}

pub struct GpuTexture {
    pub version: u64,
    pub view: wgpu::TextureView,
    pub wrap: TextureWrapMode,
    pub filter: FilterMode,
    pub width: u32,
    pub height: u32,
    /// A Render Texture's target (cameras render into it).
    pub render_target: bool,
}

/// Format of Render Textures (tonemapped color, like Unity's default ARGB32 sRGB).
pub const RENDER_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialUniform {
    pub base_color: [f32; 4],
    pub emission: [f32; 4],
    pub pbr: [f32; 4],
    pub tiling_offset: [f32; 4],
    pub flags: [f32; 4],
}

pub struct GpuMaterial {
    pub version: u64,
    pub texture_generation: u64,
    pub bind_group: wgpu::BindGroup,
    pub transparent: bool,
    pub cutout: bool,
    pub double_sided: bool,
    pub cast_shadows: bool,
    pub priority: i32,
}

pub struct GpuAssets {
    pub meshes: HashMap<AssetId, GpuMesh>,
    pub textures: HashMap<AssetId, GpuTexture>,
    pub materials: HashMap<AssetId, GpuMaterial>,
    samplers: HashMap<(TextureWrapMode, FilterMode), wgpu::Sampler>,
    /// Bumped whenever any texture is (re)uploaded so dependent materials rebuild.
    texture_generation: u64,
    pub material_layout: wgpu::BindGroupLayout,
    white: wgpu::TextureView,
    black: wgpu::TextureView,
    normal: wgpu::TextureView,
}

fn solid(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    rgba: [u8; 4],
    srgb: bool,
) -> wgpu::TextureView {
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("solid texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
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
        &rgba,
    );
    tex.create_view(&Default::default())
}

impl GpuAssets {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material layout"),
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
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        Self {
            meshes: HashMap::new(),
            textures: HashMap::new(),
            materials: HashMap::new(),
            samplers: HashMap::new(),
            texture_generation: 0,
            material_layout,
            white: solid(device, queue, [255; 4], true),
            black: solid(device, queue, [0, 0, 0, 255], true),
            normal: solid(device, queue, [128, 128, 255, 255], false),
        }
    }

    pub fn sampler(
        &mut self,
        device: &wgpu::Device,
        wrap: TextureWrapMode,
        filter: FilterMode,
    ) -> &wgpu::Sampler {
        self.samplers.entry((wrap, filter)).or_insert_with(|| {
            let mode = match wrap {
                TextureWrapMode::Repeat => wgpu::AddressMode::Repeat,
                TextureWrapMode::Clamp => wgpu::AddressMode::ClampToEdge,
                TextureWrapMode::Mirror => wgpu::AddressMode::MirrorRepeat,
            };
            let (mag, mip, aniso) = match filter {
                FilterMode::Point => (
                    wgpu::FilterMode::Nearest,
                    wgpu::MipmapFilterMode::Nearest,
                    1,
                ),
                FilterMode::Bilinear => {
                    (wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Nearest, 1)
                }
                FilterMode::Trilinear => {
                    (wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Linear, 8)
                }
            };
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("material sampler"),
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                mag_filter: mag,
                min_filter: mag,
                mipmap_filter: mip,
                anisotropy_clamp: aniso,
                ..Default::default()
            })
        })
    }

    /// Uploads `id` if it is new or changed. Returns false if the mesh does not exist.
    pub fn ensure_mesh(
        &mut self,
        device: &wgpu::Device,
        assets: &AssetServer,
        id: AssetId,
    ) -> bool {
        let Some(version) = assets.meshes.version(id) else {
            return false;
        };
        if self.meshes.get(&id).is_some_and(|m| m.version == version) {
            return true;
        }
        let mesh = assets.meshes.get_by_id(id).expect("versioned mesh exists");
        if mesh.vertices.is_empty() || mesh.indices.is_empty() {
            return false;
        }
        self.meshes.insert(id, upload_mesh(device, mesh, version));
        true
    }

    fn ensure_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &AssetServer,
        id: AssetId,
    ) -> bool {
        let Some(version) = assets.textures.version(id) else {
            return self.ensure_render_texture(device, assets, id);
        };
        if self.textures.get(&id).is_some_and(|t| t.version == version) {
            return true;
        }
        let tex = assets
            .textures
            .get_by_id(id)
            .expect("versioned texture exists");
        self.textures
            .insert(id, upload_texture(device, queue, tex, version));
        self.texture_generation += 1;
        true
    }

    /// Creates (or resizes) the GPU target of a Render Texture asset.
    fn ensure_render_texture(
        &mut self,
        device: &wgpu::Device,
        assets: &AssetServer,
        id: AssetId,
    ) -> bool {
        let Some(version) = assets.render_textures.version(id) else {
            return false;
        };
        if self.textures.get(&id).is_some_and(|t| t.version == version) {
            return true;
        }
        let rt = assets
            .render_textures
            .get_by_id(id)
            .expect("versioned render texture exists");
        let (width, height) = (rt.width.max(1), rt.height.max(1));
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(&rt.name),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: RENDER_TEXTURE_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
            .create_view(&Default::default());
        self.textures.insert(
            id,
            GpuTexture {
                version,
                view,
                wrap: TextureWrapMode::Clamp,
                filter: FilterMode::Bilinear,
                width,
                height,
                render_target: true,
            },
        );
        self.texture_generation += 1;
        true
    }

    /// The target of Render Texture `id` (made if needed): its view and size.
    pub fn render_target(
        &mut self,
        device: &wgpu::Device,
        assets: &AssetServer,
        id: AssetId,
    ) -> Option<(wgpu::TextureView, u32, u32)> {
        if !self.ensure_render_texture(device, assets, id) {
            return None;
        }
        self.textures
            .get(&id)
            .filter(|t| t.render_target)
            .map(|t| (t.view.clone(), t.width, t.height))
    }

    /// GPU view of a texture asset (uploading it if needed), e.g. for UI images.
    pub fn texture_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &AssetServer,
        id: AssetId,
    ) -> Option<&wgpu::TextureView> {
        if !self.ensure_texture(device, queue, assets, id) {
            return None;
        }
        self.textures.get(&id).map(|t| &t.view)
    }

    /// Uploads a material (and its textures). Falls back to the default material.
    pub fn ensure_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &AssetServer,
        id: AssetId,
    ) -> AssetId {
        let id = if assets.materials.version(id).is_some() {
            id
        } else {
            AssetId::from_path(oxide_assets::DEFAULT_MATERIAL)
        };
        let version = assets.materials.version(id).unwrap_or(0);
        let mat = assets.materials.get_by_id(id).cloned().unwrap_or_default();
        let tex_ids: Vec<AssetId> = [
            &mat.base_map,
            &mat.metallic_gloss_map,
            &mat.normal_map,
            &mat.occlusion_map,
            &mat.emission_map,
        ]
        .into_iter()
        .flatten()
        .map(|h| h.id())
        .collect();
        for t in tex_ids {
            self.ensure_texture(device, queue, assets, t);
        }
        if self.materials.get(&id).is_some_and(|m| {
            m.version == version && m.texture_generation == self.texture_generation
        }) {
            return id;
        }
        let gm = self.build_material(device, &mat, version);
        self.materials.insert(id, gm);
        id
    }

    fn view_or<'a>(
        &'a self,
        h: &Option<oxide_assets::Handle<Texture2D>>,
        fallback: &'a wgpu::TextureView,
    ) -> &'a wgpu::TextureView {
        h.as_ref()
            .and_then(|h| self.textures.get(&h.id()))
            .map_or(fallback, |t| &t.view)
    }

    fn build_material(&mut self, device: &wgpu::Device, m: &Material, version: u64) -> GpuMaterial {
        let base = m.base_color.linear();
        let em = m.emission_color.linear();
        let k = m.emission_intensity;
        let uniform = MaterialUniform {
            base_color: [base.r, base.g, base.b, m.base_color.a],
            emission: [em.r * k, em.g * k, em.b * k, 0.0],
            pbr: [
                m.metallic,
                1.0 - m.smoothness,
                m.normal_scale,
                if m.occlusion_map.is_some() {
                    m.occlusion_strength
                } else {
                    0.0
                },
            ],
            tiling_offset: [m.tiling.x, m.tiling.y, m.offset.x, m.offset.y],
            flags: [
                m.alpha_cutoff,
                (m.shader == ShaderKind::Unlit) as u32 as f32,
                (m.surface_type == SurfaceType::Cutout) as u32 as f32,
                (m.surface_type == SurfaceType::Transparent) as u32 as f32,
            ],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material uniform"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let (wrap, filter) = m
            .base_map
            .as_ref()
            .and_then(|h| self.textures.get(&h.id()))
            .map_or((TextureWrapMode::Repeat, FilterMode::Trilinear), |t| {
                (t.wrap, t.filter)
            });
        self.sampler(device, wrap, filter);
        let sampler = &self.samplers[&(wrap, filter)];
        let emission_fallback =
            if m.emission_color.r + m.emission_color.g + m.emission_color.b > 0.0 {
                &self.white
            } else {
                &self.black
            };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material bind group"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or(&m.base_map, &self.white),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or(&m.metallic_gloss_map, &self.white),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or(&m.normal_map, &self.normal),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or(&m.occlusion_map, &self.white),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or(&m.emission_map, emission_fallback),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        GpuMaterial {
            version,
            texture_generation: self.texture_generation,
            bind_group,
            transparent: m.surface_type == SurfaceType::Transparent,
            cutout: m.surface_type == SurfaceType::Cutout,
            double_sided: m.double_sided,
            cast_shadows: m.cast_shadows,
            priority: m.sorting_priority,
        }
    }
}

fn upload_mesh(device: &wgpu::Device, m: &Mesh, version: u64) -> GpuMesh {
    let n = m.vertices.len();
    let verts: Vec<Vertex> = (0..n)
        .map(|i| Vertex {
            position: m.vertices[i].to_array(),
            normal: m.normals.get(i).map_or([0.0, 1.0, 0.0], |v| v.to_array()),
            tangent: m
                .tangents
                .get(i)
                .map_or([1.0, 0.0, 0.0, 1.0], |v| v.to_array()),
            uv: m.uv.get(i).map_or([0.0; 2], |v| v.to_array()),
            uv2: m.uv2.get(i).map_or([0.0; 2], |v| v.to_array()),
            color: m.colors.get(i).map_or([1.0; 4], |v| v.to_array()),
        })
        .collect();
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&m.name),
        contents: bytemuck::cast_slice(&verts),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let skin = m.is_skinned().then(|| {
        let sv: Vec<SkinVertex> = (0..n)
            .map(|i| SkinVertex {
                joints: m.bone_indices[i],
                weights: m
                    .bone_weights
                    .get(i)
                    .map_or([1.0, 0.0, 0.0, 0.0], |w| w.to_array()),
            })
            .collect();
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("skin attributes"),
            contents: bytemuck::cast_slice(&sv),
            usage: wgpu::BufferUsages::VERTEX,
        })
    });
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("indices"),
        contents: bytemuck::cast_slice(&m.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    GpuMesh {
        version,
        vertices,
        skin,
        indices,
        submeshes: m.submeshes.clone(),
        bounds: m.bounds,
    }
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    t: &Texture2D,
    version: u64,
) -> GpuTexture {
    let format = if t.srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let mut data = Vec::new();
    for m in &t.mips {
        data.extend_from_slice(m);
    }
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&t.name),
            size: wgpu::Extent3d {
                width: t.width,
                height: t.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: t.mip_count(),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    GpuTexture {
        version,
        view: tex.create_view(&Default::default()),
        wrap: t.wrap_mode,
        filter: t.filter_mode,
        width: t.width,
        height: t.height,
        render_target: false,
    }
}
