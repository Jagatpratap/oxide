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
    /// The same texels without sRGB decoding, for egui (which expects gamma-encoded bytes).
    pub ui_view: wgpu::TextureView,
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
    /// Specular colour (linear), w = model: 0 Lit metallic, 1 Lit specular, 2 Simple Lit,
    /// 3 Baked Lit.
    pub spec: [f32; 4],
}

/// Vec4 slots of a Shader Graph material's property uniform.
pub const GRAPH_SLOTS: usize = 32;
/// Texture slots of a Shader Graph material (bindings 1-5, the Lit maps' slots).
pub const GRAPH_TEXTURES: usize = 5;

pub struct GpuMaterial {
    pub version: u64,
    pub texture_generation: u64,
    /// The compiled Shader Graph this material draws with (its `generation` too, so the
    /// material rebuilds when the graph is recompiled).
    pub shader: Option<(AssetId, u64)>,
    /// The graph has its own shadow-pass hooks (vertex changes, alpha clipping).
    pub shadow_shader: bool,
    pub bind_group: wgpu::BindGroup,
    pub transparent: bool,
    /// Transparent blend mode ([`oxide_assets::material::BlendMode::index`]).
    pub blend: u8,
    pub cutout: bool,
    pub double_sided: bool,
    pub cast_shadows: bool,
    /// Render queue (Unity `Material.renderQueue`, resolved).
    pub queue: i32,
    /// Keyword constants this material sets (its shader variant), and the Global keywords
    /// whose state comes from the frame.
    pub constants: Vec<(String, f64)>,
    pub global_keywords: Vec<oxide_assets::shader::ShaderKeyword>,
}

pub struct GpuAssets {
    pub meshes: HashMap<AssetId, GpuMesh>,
    pub textures: HashMap<AssetId, GpuTexture>,
    /// Cubemap assets (cube views) and Texture2DArray assets (2D array views).
    pub cubemaps: HashMap<AssetId, GpuTexture>,
    pub texture_arrays: HashMap<AssetId, GpuTexture>,
    pub materials: HashMap<AssetId, GpuMaterial>,
    samplers: HashMap<(TextureWrapMode, FilterMode), wgpu::Sampler>,
    /// Bumped whenever any texture is (re)uploaded so dependent materials rebuild.
    texture_generation: u64,
    pub material_layout: wgpu::BindGroupLayout,
    white: wgpu::TextureView,
    black: wgpu::TextureView,
    normal: wgpu::TextureView,
}

/// A view of `texture` in its non-sRGB format (it must list that in `view_formats`).
fn ui_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(texture.format().remove_srgb_suffix()),
        ..Default::default()
    })
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
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        Self {
            meshes: HashMap::new(),
            textures: HashMap::new(),
            cubemaps: HashMap::new(),
            texture_arrays: HashMap::new(),
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
        let texture = device.create_texture(&wgpu::TextureDescriptor {
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
            view_formats: &[RENDER_TEXTURE_FORMAT.remove_srgb_suffix()],
        });
        self.textures.insert(
            id,
            GpuTexture {
                version,
                view: texture.create_view(&Default::default()),
                ui_view: ui_view(&texture),
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

    /// View of a texture asset for egui (no sRGB decode), uploading it if needed.
    pub fn ui_texture_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &AssetServer,
        id: AssetId,
    ) -> Option<&wgpu::TextureView> {
        if !self.ensure_texture(device, queue, assets, id) {
            return None;
        }
        self.textures.get(&id).map(|t| &t.ui_view)
    }

    /// GPU view of a texture asset (uploading it if needed).
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

    /// An already uploaded texture's view (see [`Self::texture_view`]).
    pub fn cached_texture_view(&self, id: AssetId) -> Option<&wgpu::TextureView> {
        self.textures.get(&id).map(|t| &t.view)
    }

    /// GPU cube view of a Cubemap asset (uploading it if needed).
    pub fn cubemap_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &AssetServer,
        id: AssetId,
    ) -> Option<&wgpu::TextureView> {
        let version = assets.cubemaps.version(id)?;
        if self.cubemaps.get(&id).is_none_or(|t| t.version != version) {
            let c = assets.cubemaps.get_by_id(id)?;
            let t = upload_layers(device, queue, &c.name, &c.faces, true, version)?;
            self.cubemaps.insert(id, t);
        }
        self.cubemaps.get(&id).map(|t| &t.view)
    }

    /// GPU 2D-array view of a Texture2DArray asset (uploading it if needed).
    pub fn texture_array_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &AssetServer,
        id: AssetId,
    ) -> Option<&wgpu::TextureView> {
        let version = assets.texture_arrays.version(id)?;
        if self
            .texture_arrays
            .get(&id)
            .is_none_or(|t| t.version != version)
        {
            let a = assets.texture_arrays.get_by_id(id)?;
            let t = upload_layers(device, queue, &a.name, &a.layers, false, version)?;
            self.texture_arrays.insert(id, t);
        }
        self.texture_arrays.get(&id).map(|t| &t.view)
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
        // A Shader Graph that compiled (its surface info); otherwise the built-in shader.
        let graph = mat.shader_graph.as_ref().and_then(|h| {
            let gid = AssetId::from_path(h.path());
            let s = assets.shaders.get_by_id(gid)?.surface.clone()?;
            Some((gid, s))
        });
        let mut tex_ids: Vec<AssetId> = [
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
        if let Some((_, s)) = &graph {
            for p in s.properties.iter().filter(|p| p.kind == "Texture2D") {
                let t = mat
                    .property(&p.name)
                    .and_then(|v| v.texture.clone())
                    .or_else(|| p.default_texture.clone());
                tex_ids.extend(t.map(|h| h.id()));
            }
        }
        for t in tex_ids {
            self.ensure_texture(device, queue, assets, t);
        }
        let shader = graph.as_ref().map(|(g, s)| (*g, s.generation));
        if self.materials.get(&id).is_some_and(|m| {
            m.version == version
                && m.texture_generation == self.texture_generation
                && m.shader == shader
        }) {
            return id;
        }
        let gm = self.build_material(device, &mat, version, graph.as_ref());
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

    fn build_material(
        &mut self,
        device: &wgpu::Device,
        m: &Material,
        version: u64,
        graph: Option<&(AssetId, oxide_assets::SurfaceShader)>,
    ) -> GpuMaterial {
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
            flags: match graph {
                Some((_, g)) => [
                    m.alpha_cutoff,
                    g.unlit as u32 as f32,
                    g.alpha_clip as u32 as f32,
                    g.transparent as u32 as f32,
                ],
                None => [
                    m.alpha_cutoff,
                    (m.shader == ShaderKind::Unlit) as u32 as f32,
                    (m.surface_type == SurfaceType::Cutout) as u32 as f32,
                    (m.surface_type == SurfaceType::Transparent) as u32 as f32,
                ],
            },
            spec: {
                let sc = m.specular_color.linear();
                let model = match (graph.is_some(), m.shader, m.workflow_mode) {
                    (true, ..) => 0.0,
                    (_, ShaderKind::SimpleLit, _) => 2.0,
                    (_, ShaderKind::BakedLit, _) => 3.0,
                    (_, _, oxide_assets::WorkflowMode::Specular) => 1.0,
                    _ => 0.0,
                };
                [sc.r, sc.g, sc.b, model]
            },
        };
        // Shader Graph property values by slot (material values, else the graph defaults).
        let mut props = [[0.0f32; 4]; GRAPH_SLOTS];
        let mut graph_tex: [Option<oxide_assets::Handle<Texture2D>>; GRAPH_TEXTURES] =
            Default::default();
        if let Some((_, g)) = graph {
            for p in &g.properties {
                let set = m.property(&p.name);
                if p.kind == "Texture2D" {
                    if let Some(slot) = graph_tex.get_mut(p.slot as usize) {
                        *slot = set
                            .and_then(|v| v.texture.clone())
                            .or_else(|| p.default_texture.clone());
                    }
                    continue;
                }
                if let Some(k) = p.as_keyword() {
                    if let Some(slot) = props.get_mut(p.slot as usize) {
                        slot[0] = k.value(&|n| m.is_keyword_enabled(n)) as f32;
                    }
                    continue;
                }
                let mut v = set.map_or(p.default, |v| v.value.to_array());
                if p.kind == "Color" {
                    let c = oxide_math::Color::new(v[0], v[1], v[2], v[3]).linear();
                    v = [c.r, c.g, c.b, v[3]];
                }
                if let Some(slot) = props.get_mut(p.slot as usize) {
                    *slot = v;
                }
            }
        }
        let graph_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shader graph properties"),
            contents: bytemuck::cast_slice(&props),
            usage: wgpu::BufferUsages::UNIFORM,
        });
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
        // A Shader Graph's textures take the Lit maps' bindings.
        let g = graph.is_some();
        let pick = |i: usize, lit: &Option<oxide_assets::Handle<Texture2D>>| {
            if g { graph_tex[i].clone() } else { lit.clone() }
        };
        let (t_base, t_mr, t_normal, t_occ, t_em) = (
            pick(0, &m.base_map),
            pick(1, &m.metallic_gloss_map),
            pick(2, &m.normal_map),
            pick(3, &m.occlusion_map),
            pick(4, &m.emission_map),
        );
        let normal_fallback = if g { &self.white } else { &self.normal };
        let emission_fallback = if g { &self.white } else { emission_fallback };
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
                        self.view_or(&t_base, &self.white),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(self.view_or(&t_mr, &self.white)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or(&t_normal, normal_fallback),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(self.view_or(&t_occ, &self.white)),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or(&t_em, emission_fallback),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: graph_buffer.as_entire_binding(),
                },
            ],
        });
        let (transparent, cutout, double_sided, cast_shadows) = match graph {
            Some((_, g)) => (g.transparent, g.alpha_clip, g.double_sided, g.cast_shadows),
            None => (
                m.surface_type == SurfaceType::Transparent,
                m.surface_type == SurfaceType::Cutout,
                m.double_sided,
                m.cast_shadows,
            ),
        };
        GpuMaterial {
            blend: if graph.is_some() {
                0
            } else {
                m.blend_mode.index()
            },
            version,
            texture_generation: self.texture_generation,
            shader: graph.map(|(g, s)| (*g, s.generation)),
            shadow_shader: graph.is_some_and(|(_, s)| s.shadow_source.is_some()),
            bind_group,
            transparent,
            cutout,
            double_sided,
            cast_shadows,
            // A Shader Graph's surface type sets the From Shader queue.
            queue: {
                let mut q = m.clone();
                if let Some((_, g)) = graph {
                    q.surface_type = if g.transparent {
                        SurfaceType::Transparent
                    } else if g.alpha_clip {
                        SurfaceType::Cutout
                    } else {
                        SurfaceType::Opaque
                    };
                }
                q.effective_render_queue()
            },
            constants: match graph {
                // A Shader Graph's Local variant keywords, from the material's keywords.
                Some((_, s)) => s
                    .keywords
                    .iter()
                    .filter(|k| !k.global)
                    .map(|k| {
                        (
                            oxide_assets::shader::keyword_constant(&k.reference),
                            k.value(&|n| m.is_keyword_enabled(n)) as f64,
                        )
                    })
                    .collect(),
                // The built-in Lit shader's keywords, from the material's maps and options.
                None => m
                    .builtin_keywords()
                    .into_iter()
                    .map(|(k, on)| (oxide_assets::shader::keyword_constant(k), on as u32 as f64))
                    .collect(),
            },
            global_keywords: graph.map_or_else(Vec::new, |(_, s)| {
                s.keywords.iter().filter(|k| k.global).cloned().collect()
            }),
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

/// The wgpu format for a compressed texture, if the device supports it.
fn compressed_format(
    device: &wgpu::Device,
    f: oxide_assets::CompressedFormat,
    srgb: bool,
) -> Option<wgpu::TextureFormat> {
    use oxide_assets::CompressedFormat as C;
    use wgpu::TextureFormat as T;
    let features = device.features();
    let bc = features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
    let astc = features.contains(wgpu::Features::TEXTURE_COMPRESSION_ASTC);
    let etc2 = features.contains(wgpu::Features::TEXTURE_COMPRESSION_ETC2);
    let astc_block = |block| {
        Some(T::Astc {
            block,
            channel: if srgb {
                wgpu::AstcChannel::UnormSrgb
            } else {
                wgpu::AstcChannel::Unorm
            },
        })
    };
    match f {
        C::Bc1 if bc => Some(if srgb {
            T::Bc1RgbaUnormSrgb
        } else {
            T::Bc1RgbaUnorm
        }),
        C::Bc3 if bc => Some(if srgb {
            T::Bc3RgbaUnormSrgb
        } else {
            T::Bc3RgbaUnorm
        }),
        C::Bc4 if bc => Some(T::Bc4RUnorm),
        C::Bc5 if bc => Some(T::Bc5RgUnorm),
        C::Bc7 if bc => Some(if srgb {
            T::Bc7RgbaUnormSrgb
        } else {
            T::Bc7RgbaUnorm
        }),
        C::Astc4x4 if astc => astc_block(wgpu::AstcBlock::B4x4),
        C::Astc6x6 if astc => astc_block(wgpu::AstcBlock::B6x6),
        C::Astc8x8 if astc => astc_block(wgpu::AstcBlock::B8x8),
        C::Etc2Rgb8 if etc2 => Some(if srgb {
            T::Etc2Rgb8UnormSrgb
        } else {
            T::Etc2Rgb8Unorm
        }),
        C::Etc2Rgba8 if etc2 => Some(if srgb {
            T::Etc2Rgba8UnormSrgb
        } else {
            T::Etc2Rgba8Unorm
        }),
        C::EacR11 if etc2 => Some(T::EacR11Unorm),
        C::EacRg11 if etc2 => Some(T::EacRg11Unorm),
        _ => None,
    }
}

/// Uploads same-size layers as one texture: a cube (six faces) or a 2D array. Uses the
/// layers' compressed copies when they all have one in a format the GPU supports.
pub(crate) fn upload_layers(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    name: &str,
    layers: &[Texture2D],
    cube: bool,
    version: u64,
) -> Option<GpuTexture> {
    let first = layers.first()?;
    if (cube && layers.len() != 6)
        || layers
            .iter()
            .any(|l| (l.width, l.height) != (first.width, first.height))
    {
        oxide_core::debug_log_warning!("{name}: layers differ in size or count");
        return None;
    }
    let compressed = first.compressed.as_ref().and_then(|c| {
        let f = compressed_format(device, c.format, first.srgb)?;
        layers
            .iter()
            .all(|l| {
                l.compressed
                    .as_ref()
                    .is_some_and(|x| x.format == c.format && x.mips.len() == c.mips.len())
            })
            .then_some((f, c.mips.len() as u32))
    });
    let (format, mips, data) = match compressed {
        Some((f, n)) => (
            f,
            n,
            layers
                .iter()
                .flat_map(|l| l.compressed.as_ref().unwrap().mips.concat())
                .collect::<Vec<u8>>(),
        ),
        None => {
            if layers
                .iter()
                .any(|l| l.mips.len() != first.mips.len() || l.mips.is_empty())
            {
                oxide_core::debug_log_warning!("{name}: layers have no matching pixels");
                return None;
            }
            (
                if first.srgb {
                    wgpu::TextureFormat::Rgba8UnormSrgb
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                },
                first.mips.len() as u32,
                layers.iter().flat_map(|l| l.mips.concat()).collect(),
            )
        }
    };
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d {
                width: first.width,
                height: first.height,
                depth_or_array_layers: layers.len() as u32,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(if cube {
            wgpu::TextureViewDimension::Cube
        } else {
            wgpu::TextureViewDimension::D2Array
        }),
        ..Default::default()
    });
    // egui previews show the first layer.
    let ui_view = tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2),
        base_array_layer: 0,
        array_layer_count: Some(1),
        ..Default::default()
    });
    Some(GpuTexture {
        version,
        view,
        ui_view,
        wrap: first.wrap_mode,
        filter: first.filter_mode,
        width: first.width,
        height: first.height,
        render_target: false,
    })
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    t: &Texture2D,
    version: u64,
) -> GpuTexture {
    // The block-compressed copy when the GPU supports its format, else RGBA8.
    let compressed = t
        .compressed
        .as_ref()
        .and_then(|c| compressed_format(device, c.format, t.srgb).map(|f| (f, c)));
    let rgba8 = if t.srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let (format, data, mips, (width, height)) = match compressed {
        Some((f, c)) => (f, c.mips.concat(), c.mips.len() as u32, (t.width, t.height)),
        None if !t.mips.is_empty() => (rgba8, t.mips.concat(), t.mip_count(), (t.width, t.height)),
        None => {
            // A pre-compressed texture in a format this GPU can't sample.
            oxide_core::debug_log_warning!(
                "{}: this GPU doesn't support its compressed format; showing grey",
                t.name
            );
            (rgba8, vec![128, 128, 128, 255], 1, (1, 1))
        }
    };
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&t.name),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[format.remove_srgb_suffix()],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    GpuTexture {
        version,
        view: tex.create_view(&Default::default()),
        ui_view: ui_view(&tex),
        wrap: t.wrap_mode,
        filter: t.filter_mode,
        width: t.width,
        height: t.height,
        render_target: false,
    }
}
