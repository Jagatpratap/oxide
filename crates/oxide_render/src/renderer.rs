//! The built-in forward renderer (Oxide's URP equivalent).
//!
//! Per frame: upload changed assets → cull and batch draws per camera (GPU instancing for
//! repeated mesh+material pairs) → 4 cascaded sun shadow maps → for each camera an MSAA HDR
//! pass (clear/sky, opaque front-to-back, transparent back-to-front, debug lines) →
//! bloom and tonemapping into the target.
//!
//! References: <https://docs.unity3d.com/Manual/urp/rendering-in-universalrp.html>,
//! <https://docs.unity3d.com/Manual/shadow-cascades.html>

use crate::Gpu;
use crate::frame::*;
use crate::gpu_assets::{GpuAssets, SkinVertex, Vertex};
use crate::post::PostProcessor;
use crate::reflection::{PROBE_SIZE, ReflectionProbes};
use crate::render_graph::TextureDesc;
use oxide_assets::{AssetId, AssetServer, Texture2D};
use oxide_math::{Matrix4x4, Matrix4x4Ext, Vector3, Vector4};
use std::collections::HashMap;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const CASCADES: usize = 4;
/// Point and spot lights per frame (clustered, so each pixel only visits nearby ones).
const MAX_LIGHTS: usize = 1024;
/// Occluder triangles rasterized per camera per frame at most.
const OCCLUDER_TRIANGLE_BUDGET: u32 = 60_000;
/// Meshes with more triangles than this don't occlude (use a simpler LOD as the occluder).
const MAX_OCCLUDER_TRIANGLES: u32 = 6_000;
/// Occluders smaller than this (bounding radius / distance) are skipped.
const MIN_OCCLUDER_SIZE: f32 = 0.02;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    view_proj: [f32; 16],
    view: [f32; 16],
    inv_view_proj: [f32; 16],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    shadow_params: [f32; 4],
    cascade_splits: [f32; 4],
    cascade_vp: [[f32; 16]; CASCADES],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    ground_color: [f32; 4],
    fog_color: [f32; 4],
    fog_params: [f32; 4],
    misc: [f32; 4],
    clear_color: [f32; 4],
    cascade_texel: [f32; 4],
    ssao: [f32; 4],
    cluster_viewport: [f32; 4],
    cluster_params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightUniform {
    pos_range: [f32; 4],
    color: [f32; 4],
    dir: [f32; 4],
    /// x cos(outer/2), y 1/(cos(inner/2) - cos(outer/2)), z first shadow tile + 1 (0 = none),
    /// w shadow strength.
    spot: [f32; 4],
    /// x depth bias, y normal bias, z soft, w world texel size per metre of distance.
    shadow: [f32; 4],
}

/// Additional-light shadow atlas: square tiles, a spot light takes one, a point light six.
const ATLAS_SIZE: u32 = 4096;
const ATLAS_TILE: u32 = 512;
const ATLAS_TILES: usize = ((ATLAS_SIZE / ATLAS_TILE) * (ATLAS_SIZE / ATLAS_TILE)) as usize;

/// A shadowed point/spot light's atlas tiles this frame.
struct ShadowTiles {
    light: usize,
    first: usize,
    matrices: Vec<Matrix4x4>,
    texel: f32,
}

/// Baked lighting of one draw (indexed from `Instance::normal[1][3]`).
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceGi {
    lightmap_st: [f32; 4],
    /// x lightmap index (-1 none), y 1 when `sh` holds light probe data.
    info: [f32; 4],
    sh: [[f32; 4]; 9],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    model: [f32; 16],
    normal: [[f32; 4]; 3],
    extra: [u32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
struct MainKey {
    samples: u32,
    transparent: bool,
    double_sided: bool,
    skinned: bool,
    /// Render Objects features can change the depth test and turn depth writes off.
    depth_test: DepthTest,
    depth_write: bool,
    /// Mesh edges in line mode (Scene view wireframe draw modes).
    wire: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct ShadowKey {
    cutout: bool,
    double_sided: bool,
    skinned: bool,
}

struct Batch {
    key: MainKey,
    material: AssetId,
    mesh: AssetId,
    submesh: u32,
    instances: std::ops::Range<u32>,
}

struct ShadowBatch {
    key: ShadowKey,
    material: AssetId,
    mesh: AssetId,
    submesh: u32,
    instances: std::ops::Range<u32>,
}

/// Pipelines of the scene pass that depend on the MSAA sample count.
struct FixedPipelines {
    sky: wgpu::RenderPipeline,
    solid: wgpu::RenderPipeline,
    depth_clear: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    terrain: wgpu::RenderPipeline,
}

/// Object transforms and bone matrices of the previous and current frame (for motion
/// vectors). Several renders of one frame (Scene and Game views) share the same entry.
#[derive(Default)]
struct MotionHistory {
    frame: Option<u64>,
    prev_models: HashMap<(u64, u32), Matrix4x4>,
    curr_models: HashMap<(u64, u32), Matrix4x4>,
    prev_joints: Vec<Matrix4x4>,
    curr_joints: Vec<Matrix4x4>,
    prev_joint_offsets: HashMap<u64, u32>,
    curr_joint_offsets: HashMap<u64, u32>,
}

/// Accumulated TAA result of one view (camera × size).
struct TaaHistory {
    textures: [wgpu::TextureView; 2],
    /// Index of the texture holding the latest result.
    current: usize,
    valid: bool,
    /// Unjittered view-projection of the last frame.
    prev_vp: Matrix4x4,
    position: Vector3,
    forward: Vector3,
    last_used: u64,
}

struct Targets {
    last_used: u64,
    /// Motion vectors, made on first TAA use.
    motion: Option<wgpu::TextureView>,
    /// Depth of field / motion blur textures, made on first use.
    fx: Option<crate::postfx::FxTargets>,
    /// SSAO textures, made on first use.
    ssao: Option<crate::ssao::SsaoTargets>,
    bloom: Vec<(wgpu::TextureView, u32, u32)>,
    msaa: Option<wgpu::TextureView>,
    resolve_tex: wgpu::Texture,
    resolve: wgpu::TextureView,
    depth: wgpu::TextureView,
}

fn grow_buffer(
    device: &wgpu::Device,
    buf: &mut wgpu::Buffer,
    needed: u64,
    usage: wgpu::BufferUsages,
    label: &str,
) -> bool {
    if buf.size() >= needed {
        return false;
    }
    let size = needed.next_power_of_two().max(1024);
    *buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    });
    true
}

pub struct SceneRenderer {
    /// The GPU draws polygons as lines (wireframe draw modes).
    wire_supported: bool,
    pub gpu_assets: GpuAssets,
    terrain: crate::terrain::TerrainGpu,
    particles: crate::particles::ParticleGpu,
    sprites: crate::sprites::SpriteGpu,
    /// MSAA sample counts the GPU supports for the HDR and depth formats.
    supported_samples: Vec<u32>,
    frame_layout: wgpu::BindGroupLayout,
    shadow_layout: wgpu::BindGroupLayout,
    main_layout: wgpu::PipelineLayout,
    shadow_pipeline_layout: wgpu::PipelineLayout,
    lit_module: wgpu::ShaderModule,
    shadow_module: wgpu::ShaderModule,
    main_pipelines: HashMap<MainKey, wgpu::RenderPipeline>,
    shadow_pipelines: HashMap<ShadowKey, wgpu::RenderPipeline>,
    sky_module: wgpu::ShaderModule,
    lines_module: wgpu::ShaderModule,
    terrain_module: wgpu::ShaderModule,
    frame_only_layout: wgpu::PipelineLayout,
    terrain_pipeline_layout: wgpu::PipelineLayout,
    /// Sky/clear, line and terrain pipelines per MSAA sample count.
    fixed: HashMap<u32, FixedPipelines>,
    aa: crate::aa::AaGpu,
    motion_buffer: wgpu::Buffer,
    prev_joint_buffer: wgpu::Buffer,
    motion: MotionHistory,
    taa_histories: HashMap<(u64, u32, u32), TaaHistory>,
    postfx: crate::postfx::PostFx,
    ssao: crate::ssao::SsaoGpu,
    prepass_layout: wgpu::PipelineLayout,
    /// Depth-only pipelines for the SSAO prepass: (cutout, double sided, skinned).
    prepass_pipelines: HashMap<(bool, bool, bool), wgpu::RenderPipeline>,
    terrain_prepass: wgpu::RenderPipeline,
    /// Per view: (frame index, view-projection this frame, last frame's), for motion blur.
    view_prev: HashMap<(u64, u32, u32), (u64, Matrix4x4, Matrix4x4)>,
    /// Frame each Render Texture was last rendered in.
    rt_frames: HashMap<AssetId, u64>,
    /// Baked lightmaps as one texture array, and which assets (and versions) it holds.
    lightmap_array: wgpu::TextureView,
    lightmap_sources: Vec<(AssetId, u64)>,
    lightmap_count: usize,
    gi_buffer: wgpu::Buffer,
    fullscreen: crate::fullscreen::FullscreenGpu,
    graph_pool: crate::render_graph::TexturePool,
    frame_buffers: Vec<wgpu::Buffer>,
    /// Per camera: cluster cells and light index list (Forward+).
    cluster_buffers: Vec<(wgpu::Buffer, wgpu::Buffer)>,
    frame_groups: Vec<wgpu::BindGroup>,
    light_buffer: wgpu::Buffer,
    joint_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    line_buffer: wgpu::Buffer,
    shadow_size: u32,
    shadow_layers: Vec<wgpu::TextureView>,
    shadow_array: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    shadow_buffers: Vec<wgpu::Buffer>,
    shadow_groups: Vec<wgpu::BindGroup>,
    /// Point/spot shadow atlas and one uniform + group per tile.
    atlas: wgpu::TextureView,
    atlas_buffers: Vec<wgpu::Buffer>,
    atlas_groups: Vec<wgpu::BindGroup>,
    atlas_matrices: wgpu::Buffer,
    groups_dirty: bool,
    /// Render targets per output size (Scene and Game views render at different sizes).
    targets: HashMap<(u32, u32, u32), Targets>,
    frame_index: u64,
    post: PostProcessor,
    /// One occlusion buffer per camera of the last frame.
    occlusion: Vec<crate::occlusion::OcclusionBuffer>,
    pub last_stats: RenderStats,
    last_samples: u32,
    reflections: ReflectionProbes,
}

/// A cube capture in progress: shadows cover a sphere around `center`, and the probe being
/// captured doesn't reflect itself.
#[derive(Clone, Copy)]
struct Capture {
    center: Vector3,
    exclude: Option<u32>,
}

/// Probe captures render all six faces into one 3×2 atlas.
const CAPTURE_W: u32 = PROBE_SIZE * 3;
const CAPTURE_H: u32 = PROBE_SIZE * 2;

fn vertex_layouts(skinned: bool) -> Vec<Option<wgpu::VertexBufferLayout<'static>>> {
    const VERTEX: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2, 4 => Float32x2, 5 => Float32x4
    ];
    const INSTANCE: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
        6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4,
        10 => Float32x4, 11 => Float32x4, 12 => Float32x4, 13 => Uint32x4
    ];
    const SKIN: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![14 => Uint16x4, 15 => Float32x4];
    let mut v = vec![
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE,
        }),
    ];
    if skinned {
        v.push(Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SkinVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &SKIN,
        }));
    }
    v
}

fn shader(device: &wgpu::Device, label: &str, src: &str) -> wgpu::ShaderModule {
    let full = format!("{}\n{}", include_str!("shaders/common.wgsl"), src);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(full.into()),
    })
}

fn storage_entry(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
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

impl SceneRenderer {
    pub fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;
        let hdr_flags = gpu.adapter.get_texture_format_features(HDR_FORMAT).flags;
        let depth_flags = gpu.adapter.get_texture_format_features(DEPTH_FORMAT).flags;
        let supported_samples = [1, 2, 4, 8]
            .into_iter()
            .filter(|&n| {
                n == 1
                    || hdr_flags.sample_count_supported(n) && depth_flags.sample_count_supported(n)
            })
            .collect();
        let gpu_assets = GpuAssets::new(device, &gpu.queue);
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame layout"),
            entries: &[
                uniform_entry(0, vf),
                storage_entry(1, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                storage_entry(4, wgpu::ShaderStages::VERTEX),
                storage_entry(5, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::CubeArray,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                storage_entry(8, wgpu::ShaderStages::FRAGMENT),
                storage_entry(9, wgpu::ShaderStages::FRAGMENT),
                storage_entry(10, wgpu::ShaderStages::FRAGMENT),
                storage_entry(11, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                storage_entry(14, wgpu::ShaderStages::VERTEX_FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow view layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX),
                storage_entry(1, wgpu::ShaderStages::VERTEX),
            ],
        });
        let ssao = crate::ssao::SsaoGpu::new(device, &gpu.queue);
        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("main pipeline layout"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&gpu_assets.material_layout),
                Some(&ssao.screen_layout),
            ],
            immediate_size: 0,
        });
        let prepass_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("depth prepass layout"),
            bind_group_layouts: &[Some(&frame_layout), Some(&gpu_assets.material_layout)],
            immediate_size: 0,
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow pipeline layout"),
                bind_group_layouts: &[Some(&shadow_layout), Some(&gpu_assets.material_layout)],
                immediate_size: 0,
            });
        let frame_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("frame-only layout"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let lit_src = format!(
            "{}\n{}",
            include_str!("shaders/lit.wgsl"),
            include_str!("shaders/lighting.wgsl")
        );
        let lit_module = shader(device, "lit.wgsl", &lit_src);

        let shadow_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
        });
        let sky_module = shader(device, "sky.wgsl", include_str!("shaders/sky.wgsl"));
        let lines_module = shader(device, "lines.wgsl", include_str!("shaders/lines.wgsl"));

        let shadow_size = 2048;
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun shadow cascades"),
            size: wgpu::Extent3d {
                width: shadow_size,
                height: shadow_size,
                depth_or_array_layers: CASCADES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_layers = (0..CASCADES as u32)
            .map(|i| {
                shadow_tex.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("shadow cascade"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let shadow_array = shadow_tex.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let shadow_buffers = (0..CASCADES)
            .map(|_| buf("shadow view", 64, wgpu::BufferUsages::UNIFORM))
            .collect();
        let post = PostProcessor::new(device);
        let terrain = crate::terrain::TerrainGpu::new(device);
        let terrain_module = shader(
            device,
            "terrain.wgsl",
            &format!(
                "{}\n{}",
                include_str!("shaders/terrain.wgsl"),
                include_str!("shaders/lighting.wgsl")
            ),
        );
        let terrain_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("terrain pipeline layout"),
                bind_group_layouts: &[
                    Some(&frame_layout),
                    Some(&terrain.layout),
                    Some(&ssao.screen_layout),
                ],
                immediate_size: 0,
            });
        let terrain_prepass = {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("terrain prepass layout"),
                bind_group_layouts: &[Some(&frame_layout), Some(&terrain.layout)],
                immediate_size: 0,
            });
            let buffers = vertex_layouts(false);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("terrain depth prepass"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &terrain_module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        };
        let particle_module = shader(
            device,
            "particles.wgsl",
            include_str!("shaders/particles.wgsl"),
        );
        let particles =
            crate::particles::ParticleGpu::new(device, &gpu.queue, &frame_layout, particle_module);
        let sprite_module = shader(device, "sprites.wgsl", include_str!("shaders/sprites.wgsl"));
        let sprites =
            crate::sprites::SpriteGpu::new(device, &gpu.queue, &frame_layout, sprite_module);
        Self {
            wire_supported: device
                .features()
                .contains(wgpu::Features::POLYGON_MODE_LINE),
            terrain,
            particles,
            sprites,
            gpu_assets,
            supported_samples,
            frame_layout,
            shadow_layout,
            main_layout,
            shadow_pipeline_layout,
            lit_module,
            shadow_module,
            main_pipelines: HashMap::new(),
            shadow_pipelines: HashMap::new(),
            sky_module,
            lines_module,
            terrain_module,
            frame_only_layout: frame_only,
            terrain_pipeline_layout,
            fixed: HashMap::new(),
            aa: crate::aa::AaGpu::new(device),
            motion_buffer: buf("motion instances", 144 * 256, wgpu::BufferUsages::VERTEX),
            prev_joint_buffer: buf("previous joints", 64 * 256, wgpu::BufferUsages::STORAGE),
            motion: MotionHistory::default(),
            taa_histories: HashMap::new(),
            postfx: crate::postfx::PostFx::new(device),
            ssao,
            prepass_layout,
            prepass_pipelines: HashMap::new(),
            terrain_prepass,
            view_prev: HashMap::new(),
            rt_frames: HashMap::new(),
            lightmap_array: lightmap_array(device, &gpu.queue, &[], 1),
            lightmap_sources: Vec::new(),
            lightmap_count: 0,
            gi_buffer: buf("baked lighting", 176 * 64, wgpu::BufferUsages::STORAGE),
            fullscreen: crate::fullscreen::FullscreenGpu::new(device, &gpu.queue),
            graph_pool: Default::default(),
            frame_buffers: Vec::new(),
            cluster_buffers: Vec::new(),
            frame_groups: Vec::new(),
            light_buffer: buf(
                "lights",
                (MAX_LIGHTS * std::mem::size_of::<LightUniform>()) as u64,
                wgpu::BufferUsages::STORAGE,
            ),
            joint_buffer: buf("joints", 64 * 256, wgpu::BufferUsages::STORAGE),
            instance_buffer: buf("instances", 128 * 1024, wgpu::BufferUsages::VERTEX),
            line_buffer: buf("lines", 28 * 1024, wgpu::BufferUsages::VERTEX),
            shadow_size,
            shadow_layers,
            shadow_array,
            shadow_sampler,
            shadow_buffers,
            shadow_groups: Vec::new(),
            atlas: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("additional light shadow atlas"),
                    size: wgpu::Extent3d {
                        width: ATLAS_SIZE,
                        height: ATLAS_SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: SHADOW_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default()),
            atlas_buffers: (0..ATLAS_TILES)
                .map(|_| buf("shadow tile view", 64, wgpu::BufferUsages::UNIFORM))
                .collect(),
            atlas_groups: Vec::new(),
            atlas_matrices: buf(
                "shadow tile matrices",
                (ATLAS_TILES * 64) as u64,
                wgpu::BufferUsages::STORAGE,
            ),
            groups_dirty: true,
            targets: HashMap::new(),
            frame_index: 0,
            post,
            occlusion: Vec::new(),
            last_stats: RenderStats::default(),
            last_samples: 1,
            reflections: ReflectionProbes::new(device, &gpu.queue),
        }
    }

    /// The largest supported MSAA sample count not above `wanted`.
    pub fn sample_count_for(&self, wanted: u32) -> u32 {
        self.supported_samples
            .iter()
            .copied()
            .filter(|&n| n <= wanted.max(1))
            .max()
            .unwrap_or(1)
    }

    /// MSAA samples used by the last render.
    /// Whether the wireframe draw modes can draw edges on this GPU.
    pub fn supports_wireframe(&self) -> bool {
        self.wire_supported
    }

    pub fn sample_count(&self) -> u32 {
        self.last_samples
    }

    fn ensure_fixed(&mut self, device: &wgpu::Device, samples: u32) {
        if self.fixed.contains_key(&samples) {
            return;
        }
        let ms = wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        };
        let (sky_module, frame_only) = (&self.sky_module, &self.frame_only_layout);
        let fullscreen = |entry: &str, write_color: bool, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(frame_only),
                vertex: wgpu::VertexState {
                    module: sky_module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                // Writes depth 0 (reversed-Z far plane): this also clears depth in the viewport.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: ms,
                fragment: Some(wgpu::FragmentState {
                    module: sky_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: None,
                        write_mask: if write_color {
                            wgpu::ColorWrites::ALL
                        } else {
                            wgpu::ColorWrites::empty()
                        },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sky = fullscreen("fs_main", true, "sky pipeline");
        let solid = fullscreen("fs_solid", true, "solid clear pipeline");
        let depth_clear = fullscreen("fs_solid", false, "depth clear pipeline");
        let line = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("line pipeline"),
            layout: Some(frame_only),
            vertex: wgpu::VertexState {
                module: &self.lines_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: ms,
            fragment: Some(wgpu::FragmentState {
                module: &self.lines_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let buffers = vertex_layouts(false);
        let terrain = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain pipeline"),
            layout: Some(&self.terrain_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.terrain_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                front_face: wgpu::FrontFace::Cw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: ms,
            fragment: Some(wgpu::FragmentState {
                module: &self.terrain_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.fixed.insert(
            samples,
            FixedPipelines {
                sky,
                solid,
                depth_clear,
                line,
                terrain,
            },
        );
    }

    fn main_pipeline(&mut self, device: &wgpu::Device, key: MainKey) -> &wgpu::RenderPipeline {
        let (module, layout, ms) = (&self.lit_module, &self.main_layout, key.samples);
        self.main_pipelines.entry(key).or_insert_with(|| {
            let buffers = vertex_layouts(key.skinned);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("lit {key:?}")),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(if key.skinned { "vs_skinned" } else { "vs_main" }),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: if key.double_sided || key.wire {
                        None
                    } else {
                        Some(wgpu::Face::Back)
                    },
                    polygon_mode: if key.wire {
                        wgpu::PolygonMode::Line
                    } else {
                        wgpu::PolygonMode::Fill
                    },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(!key.transparent && key.depth_write),
                    // Reversed-Z: nearer is greater.
                    depth_compare: Some(match key.depth_test {
                        DepthTest::LessEqual => wgpu::CompareFunction::Greater,
                        DepthTest::Greater => wgpu::CompareFunction::Less,
                        DepthTest::Always => wgpu::CompareFunction::Always,
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: ms,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(if key.wire { "fs_wire" } else { "fs_main" }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: Some(if key.transparent {
                            wgpu::BlendState::ALPHA_BLENDING
                        } else {
                            wgpu::BlendState::REPLACE
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    fn prepass_pipeline(&mut self, device: &wgpu::Device, key: (bool, bool, bool)) {
        let (cutout, double_sided, skinned) = key;
        let (module, layout) = (&self.lit_module, &self.prepass_layout);
        self.prepass_pipelines.entry(key).or_insert_with(|| {
            let buffers = vertex_layouts(skinned);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("depth prepass"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(if skinned { "vs_skinned" } else { "vs_main" }),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: if double_sided {
                        None
                    } else {
                        Some(wgpu::Face::Back)
                    },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: cutout.then(|| wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_depth_cutout"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
    }

    fn shadow_pipeline(&mut self, device: &wgpu::Device, key: ShadowKey) -> &wgpu::RenderPipeline {
        let (module, layout) = (&self.shadow_module, &self.shadow_pipeline_layout);
        self.shadow_pipelines.entry(key).or_insert_with(|| {
            let buffers = vertex_layouts(key.skinned);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("shadow {key:?}")),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(if key.skinned { "vs_skinned" } else { "vs_main" }),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    front_face: wgpu::FrontFace::Cw,
                    // Two-sided casting avoids light leaks through open meshes.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                }),
                multisample: Default::default(),
                fragment: key.cutout.then(|| wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_cutout"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, w: u32, h: u32, samples: u32) {
        self.frame_index += 1;
        let now = self.frame_index;
        if let Some(t) = self.targets.get_mut(&(w, h, samples)) {
            t.last_used = now;
            return;
        }
        // Keep a few sizes alive; drop the least recently used beyond that.
        while self.targets.len() >= 4 {
            let oldest = *self
                .targets
                .iter()
                .min_by_key(|(_, t)| t.last_used)
                .expect("non-empty")
                .0;
            self.targets.remove(&oldest);
        }
        let tex = |label: &str, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = (samples > 1).then(|| tex("hdr msaa", HDR_FORMAT, samples, ra));
        let resolve_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: ra | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let resolve = resolve_tex.create_view(&Default::default());
        // Depth is read by TAA, depth of field and motion blur.
        let depth_usage = ra | wgpu::TextureUsages::TEXTURE_BINDING;
        let depth = tex("depth", DEPTH_FORMAT, samples, depth_usage);
        let bloom = PostProcessor::bloom_chain(device, w, h);
        self.targets.insert(
            (w, h, samples),
            Targets {
                last_used: now,
                motion: None,
                fx: None,
                ssao: None,
                bloom,
                msaa,
                resolve_tex,
                resolve,
                depth,
            },
        );
    }

    fn rebuild_groups(&mut self, device: &wgpu::Device, cameras: usize) {
        while self.cluster_buffers.len() < cameras {
            let buf = |label: &str, size: u64| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            self.cluster_buffers.push((
                buf("cluster cells", (crate::clusters::CLUSTERS * 8) as u64),
                buf("cluster lights", 64 * 1024),
            ));
            self.groups_dirty = true;
        }
        while self.frame_buffers.len() < cameras {
            self.frame_buffers
                .push(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("frame uniform"),
                    size: std::mem::size_of::<FrameUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            self.groups_dirty = true;
        }
        if !self.groups_dirty {
            return;
        }
        self.frame_groups = self
            .frame_buffers
            .iter()
            .zip(&self.cluster_buffers)
            .map(|(b, (cells, cluster_lights))| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("frame bind group"),
                    layout: &self.frame_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: b.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.light_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&self.shadow_array),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: self.joint_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: self.reflections.probe_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::TextureView(
                                &self.reflections.cube_array,
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::Sampler(&self.reflections.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 8,
                            resource: self.reflections.sh_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 9,
                            resource: cells.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 10,
                            resource: cluster_lights.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 11,
                            resource: self.atlas_matrices.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 12,
                            resource: wgpu::BindingResource::TextureView(&self.atlas),
                        },
                        wgpu::BindGroupEntry {
                            binding: 13,
                            resource: wgpu::BindingResource::TextureView(&self.lightmap_array),
                        },
                        wgpu::BindGroupEntry {
                            binding: 14,
                            resource: self.gi_buffer.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        self.atlas_groups = self
            .atlas_buffers
            .iter()
            .map(|b| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shadow tile bind group"),
                    layout: &self.shadow_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: b.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.joint_buffer.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        self.shadow_groups = self
            .shadow_buffers
            .iter()
            .map(|b| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shadow view bind group"),
                    layout: &self.shadow_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: b.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.joint_buffer.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        self.groups_dirty = false;
    }

    /// Renders `frame` into `target` (`width`×`height`, `format`) and submits the work.
    /// The occlusion buffer of camera `index` from the last frame (empty if that camera
    /// had occlusion culling off).
    pub fn occlusion_buffer(&self, index: usize) -> Option<&crate::occlusion::OcclusionBuffer> {
        self.occlusion.get(index).filter(|b| b.width() > 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gpu: &Gpu,
        assets: &AssetServer,
        frame: &RenderFrame,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> RenderStats {
        let captures = self.update_reflections(gpu, assets, frame);
        self.render_texture_cameras(gpu, assets, frame);
        let mut stats = self.render_view(gpu, assets, frame, target, format, width, height);
        stats.reflection_probes = self.reflections.active_count();
        stats.probe_captures = captures;
        self.last_stats = stats;
        stats
    }

    /// Cameras with a target texture render into it (once per frame each).
    fn render_texture_cameras(&mut self, gpu: &Gpu, assets: &AssetServer, frame: &RenderFrame) {
        for (rt, cam) in &frame.texture_cameras {
            if self.rt_frames.get(rt) == Some(&frame.frame_index) {
                continue;
            }
            self.rt_frames.insert(*rt, frame.frame_index);
            let Some((view, w, h)) = self.gpu_assets.render_target(&gpu.device, assets, *rt) else {
                continue;
            };
            let sub = RenderFrame {
                cameras: vec![cam.clone()],
                texture_cameras: Vec::new(),
                ..frame.clone()
            };
            self.render_view(
                gpu,
                assets,
                &sub,
                &view,
                crate::gpu_assets::RENDER_TEXTURE_FORMAT,
                w,
                h,
            );
        }
    }

    /// Scene, anti-aliasing and post-processing of `frame`'s cameras into `target`.
    #[allow(clippy::too_many_arguments)]
    fn render_view(
        &mut self,
        gpu: &Gpu,
        assets: &AssetServer,
        frame: &RenderFrame,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> RenderStats {
        let (w, h) = (width.max(1), height.max(1));
        let (mut encoder, stats, out) = self.render_scene(gpu, assets, frame, w, h, None);

        // ---- post-processing into the target ------------------------------------------
        let key = (w, h, out.samples);
        // Depth of field and motion blur, from the first camera's view.
        let mut fx_out = None;
        if out.fx
            && let Some(cam) = frame.cameras.first()
        {
            let aspect = (w as f32 * cam.viewport[2]) / (h as f32 * cam.viewport[3]).max(1.0);
            let vp = cam.view_projection(aspect);
            let view_key = (cam.antialiasing.history_id, w, h);
            let entry = self
                .view_prev
                .entry(view_key)
                .or_insert((frame.frame_index, vp, vp));
            if entry.0 != frame.frame_index {
                *entry = (frame.frame_index, vp, entry.1);
            }
            let prev_vp = entry.2;
            while self.view_prev.len() > 8 {
                let k = *self.view_prev.keys().next().expect("non-empty");
                self.view_prev.remove(&k);
            }
            let t = self.targets.get_mut(&key).expect("targets");
            t.fx.get_or_insert_with(|| crate::postfx::PostFx::targets(&gpu.device, w, h));
            let t = &self.targets[&key];
            let fxt = t.fx.as_ref().expect("made above");
            self.postfx
                .resolve_depth(&gpu.device, &mut encoder, &t.depth, out.samples, fxt);
            let src = match out.taa {
                Some(k) => {
                    let hist = &self.taa_histories[&k];
                    &hist.textures[hist.current]
                }
                None => &t.resolve,
            };
            fx_out = self.postfx.run(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                src,
                fxt,
                &crate::postfx::FxInputs {
                    post: &frame.environment.post,
                    inv_vp: vp.inverse().to_cols_array(),
                    prev_vp: prev_vp.to_cols_array(),
                    near: cam.near,
                    far: cam.far,
                    orthographic: matches!(cam.projection, ProjectionKind::Orthographic { .. }),
                    height: h,
                    motion: out.taa.and(t.motion.as_ref()),
                },
            );
        }
        let env = &frame.environment;
        let targets = &self.targets[&key];
        let hdr = match (fx_out, out.taa) {
            (Some(i), _) => &targets.fx.as_ref().expect("fx targets").out[i],
            (None, Some(k)) => {
                let hist = &self.taa_histories[&k];
                &hist.textures[hist.current]
            }
            (None, None) => &targets.resolve,
        };

        // Post-processing as a render graph: Full Screen Pass features before post on the
        // HDR image, bloom + uber, FXAA, then features after post on the final image.
        let full_screen = |event: RenderPassEvent| {
            frame
                .renderer_features
                .iter()
                .filter_map(move |f| match f {
                    RendererFeature::FullScreen {
                        event: e,
                        shader,
                        params,
                    } if *e == event => Some((*shader, *params)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let before = full_screen(RenderPassEvent::BeforeRenderingPostProcessing);
        let after = full_screen(RenderPassEvent::AfterRenderingPostProcessing);
        let depth_copy = targets.fx.as_ref().map(|f| &f.depth);
        let post = std::cell::RefCell::new(&mut self.post);
        let aa = std::cell::RefCell::new(&mut self.aa);
        let fsg = std::cell::RefCell::new(&mut self.fullscreen);
        let (aspect, time) = (w as f32 / h as f32, frame.time);
        let srgb = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut graph = crate::render_graph::RenderGraph::new();
        let mut cur = graph.import(hdr);
        let mut slot = 0;
        for (shader, params) in before {
            let (src, dst) = (cur, graph.create(TextureDesc::color(w, h, HDR_FORMAT)));
            let (fsg, s) = (&fsg, slot);
            graph.add_pass(
                "Full Screen Pass (before post)",
                |b| {
                    b.read(src).write(dst);
                },
                move |ctx| {
                    fsg.borrow_mut().run(
                        ctx.device,
                        ctx.queue,
                        ctx.encoder,
                        assets,
                        s,
                        shader,
                        params,
                        time,
                        &ctx.view(src),
                        depth_copy,
                        &ctx.view(dst),
                        HDR_FORMAT,
                        (w, h),
                    )
                },
            );
            cur = dst;
            slot += 1;
        }
        let final_target = graph.import(target);
        let (post_out, post_format) = if out.fxaa {
            (
                graph.create(TextureDesc::color(w, h, crate::aa::LDR_FORMAT)),
                crate::aa::LDR_FORMAT,
            )
        } else if !after.is_empty() {
            (graph.create(TextureDesc::color(w, h, srgb)), srgb)
        } else {
            (final_target, format)
        };
        {
            let (src, post) = (cur, &post);
            graph.add_pass(
                "Post-processing",
                |b| {
                    b.read(src).write(post_out);
                },
                move |ctx| {
                    post.borrow_mut().run(
                        ctx.device,
                        ctx.queue,
                        ctx.encoder,
                        &ctx.view(src),
                        &targets.bloom,
                        &ctx.view(post_out),
                        post_format,
                        &env.post,
                        aspect,
                        time,
                    )
                },
            );
            cur = post_out;
        }
        if out.fxaa {
            let (dst, dst_format) = if after.is_empty() {
                (final_target, format)
            } else {
                (graph.create(TextureDesc::color(w, h, srgb)), srgb)
            };
            let (src, aa) = (cur, &aa);
            graph.add_pass(
                "FXAA",
                |b| {
                    b.read(src).write(dst);
                },
                move |ctx| {
                    aa.borrow_mut().fxaa(
                        ctx.device,
                        ctx.queue,
                        ctx.encoder,
                        &ctx.view(src),
                        &ctx.view(dst),
                        dst_format,
                        w,
                        h,
                    )
                },
            );
            cur = dst;
        }
        let count = after.len();
        for (i, (shader, params)) in after.into_iter().enumerate() {
            let (dst, dst_format) = if i + 1 == count {
                (final_target, format)
            } else {
                (graph.create(TextureDesc::color(w, h, srgb)), srgb)
            };
            let (src, fsg, s) = (cur, &fsg, slot);
            graph.add_pass(
                "Full Screen Pass (after post)",
                |b| {
                    b.read(src).write(dst);
                },
                move |ctx| {
                    fsg.borrow_mut().run(
                        ctx.device,
                        ctx.queue,
                        ctx.encoder,
                        assets,
                        s,
                        shader,
                        params,
                        time,
                        &ctx.view(src),
                        depth_copy,
                        &ctx.view(dst),
                        dst_format,
                        (w, h),
                    )
                },
            );
            cur = dst;
            slot += 1;
        }
        graph.execute(&gpu.device, &gpu.queue, &mut encoder, &mut self.graph_pool);
        gpu.queue.submit([encoder.finish()]);
        stats
    }

    /// Captures the sky and the reflection probes that need it this frame; returns how many
    /// cubemaps were captured.
    fn update_reflections(&mut self, gpu: &Gpu, assets: &AssetServer, frame: &RenderFrame) -> u32 {
        let env = &frame.environment;
        let sky_key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            let mut f = |v: f32| v.to_bits().hash(&mut h);
            for c in [env.sky_tint, env.ground_color] {
                f(c.r);
                f(c.g);
                f(c.b);
            }
            f(env.sky_exposure);
            f(env.sun_size);
            if let Some(s) = &frame.sun {
                f(s.direction.x);
                f(s.direction.y);
                f(s.direction.z);
                s.color.iter().for_each(|&c| f(c));
            }
            if let Some((kind, c)) = env.fog {
                f(c.r);
                f(c.g);
                f(c.b);
                format!("{kind:?}").hash(&mut h);
            }
            h.finish()
        };
        let jobs = self
            .reflections
            .plan(&gpu.queue, sky_key, &frame.reflection_probes, frame.time);
        let count = jobs.len() as u32;
        for job in jobs {
            let (center, capture_frame) = match &job.probe {
                None => {
                    let sun = frame.sun.clone().map(|mut s| {
                        s.shadows = None;
                        s
                    });
                    let f = RenderFrame {
                        cameras: face_cameras(Vector3::ZERO, 0.1, 10.0, ClearMode::Sky, 0),
                        sun,
                        environment: frame.environment.clone(),
                        time: frame.time,
                        ..Default::default()
                    };
                    (Vector3::ZERO, f)
                }
                Some(p) => {
                    let mut environment = frame.environment.clone();
                    environment.shadow_distance = environment.shadow_distance.min(p.far);
                    let f = RenderFrame {
                        cameras: face_cameras(p.position, p.near, p.far, p.clear, p.culling_mask),
                        sun: frame.sun.clone(),
                        lights: frame.lights.clone(),
                        draws: frame
                            .draws
                            .iter()
                            .filter(|d| !p.baked || d.reflection_probe_static)
                            .cloned()
                            .collect(),
                        terrain_draws: frame.terrain_draws.clone(),
                        particles: if p.baked {
                            Vec::new()
                        } else {
                            frame.particles.clone()
                        },
                        joints: frame.joints.clone(),
                        environment,
                        time: frame.time,
                        ..Default::default()
                    };
                    (p.position, f)
                }
            };
            let capture = Capture {
                center,
                exclude: job.probe.as_ref().map(|_| job.slot),
            };
            let (mut encoder, _, out) = self.render_scene(
                gpu,
                assets,
                &capture_frame,
                CAPTURE_W,
                CAPTURE_H,
                Some(capture),
            );
            let targets = &self.targets[&(CAPTURE_W, CAPTURE_H, out.samples)];
            for face in 0..6u32 {
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &targets.resolve_tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: (face % 3) * PROBE_SIZE,
                            y: (face / 3) * PROBE_SIZE,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: self.reflections.capture_texture(),
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: face,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: PROBE_SIZE,
                        height: PROBE_SIZE,
                        depth_or_array_layers: 1,
                    },
                );
            }
            self.reflections.filter(&mut encoder, job.slot);
            gpu.queue.submit([encoder.finish()]);
        }
        count
    }

    /// Culls, batches and draws every camera of `frame` into the HDR target of size `w`×`h`
    /// (shadows first). Returns the unsubmitted encoder.
    fn render_scene(
        &mut self,
        gpu: &Gpu,
        assets: &AssetServer,
        frame: &RenderFrame,
        w: u32,
        h: u32,
        capture: Option<Capture>,
    ) -> (wgpu::CommandEncoder, RenderStats, SceneOutput) {
        let device = &gpu.device;
        let queue = &gpu.queue;

        // Anti-aliasing comes from the first camera; TAA needs exactly one camera.
        let aa = frame
            .cameras
            .first()
            .map(|c| c.antialiasing)
            .unwrap_or_default();
        let mut mode = if capture.is_some() {
            AntiAliasing::None
        } else {
            aa.mode
        };
        if mode == AntiAliasing::Taa && frame.cameras.len() != 1 {
            mode = AntiAliasing::Fxaa;
        }
        let taa = mode == AntiAliasing::Taa;
        // Depth is copied for depth-based post effects and Full Screen Pass shaders.
        let fx = capture.is_none()
            && (crate::postfx::PostFx::wanted(&frame.environment.post)
                || frame
                    .renderer_features
                    .iter()
                    .any(|f| matches!(f, RendererFeature::FullScreen { .. })));
        // SSAO for a single camera (its AO texture covers the whole target).
        let ssao = frame
            .environment
            .ssao
            .filter(|_| capture.is_none() && frame.cameras.len() == 1);
        let samples = if taa || !aa.allow_msaa {
            1
        } else {
            self.sample_count_for(frame.environment.msaa_samples)
        };
        self.ensure_targets(device, w, h, samples);
        self.ensure_fixed(device, samples);
        self.particles.ensure_pipelines(device, samples);
        self.sprites.ensure_pipelines(device, samples);
        if capture.is_none() {
            self.last_samples = samples;
            self.motion.advance(frame);
        }
        let mut stats = RenderStats {
            cameras: frame.cameras.len() as u32,
            ..Default::default()
        };

        // ---- upload assets and resolve draws ------------------------------------------
        struct Resolved<'a> {
            item: &'a DrawItem,
            material: AssetId,
            center: Vector3,
            radius: f32,
            /// Mesh bounds (local).
            bounds: oxide_math::Bounds,
            transparent: bool,
            cutout: bool,
            double_sided: bool,
            cast: bool,
            priority: i32,
            /// Reflection probes chosen for this object (instance `extra.zw`).
            probes: [u32; 2],
            /// Index into the baked-lighting table, or -1.
            gi: f32,
        }
        let mut gis: Vec<InstanceGi> = Vec::new();
        let exclude = capture.and_then(|c| c.exclude);
        self.update_lightmaps(gpu, assets, &frame.lightmaps);
        let mut resolved = Vec::with_capacity(frame.draws.len());
        for d in &frame.draws {
            if !self.gpu_assets.ensure_mesh(device, assets, d.mesh) {
                continue;
            }
            let mat = self
                .gpu_assets
                .ensure_material(device, queue, assets, d.material);
            let gm = &self.gpu_assets.materials[&mat];
            let mesh = &self.gpu_assets.meshes[&d.mesh];
            if d.submesh as usize >= mesh.submeshes.len() {
                continue;
            }
            let (center, radius) = if d.joint_offset.is_some() {
                (Vector3::ZERO, f32::INFINITY)
            } else {
                let c = d.model.transform_point3(mesh.bounds.center);
                let s = d
                    .model
                    .x_axis
                    .truncate()
                    .length()
                    .max(d.model.y_axis.truncate().length())
                    .max(d.model.z_axis.truncate().length());
                (c, mesh.bounds.extents.length() * s)
            };
            // Skinned draws are in world space: use the first bone's position for probes.
            let probe_pos = match d.joint_offset {
                Some(o) => frame
                    .joints
                    .get(o as usize)
                    .map_or(center, |m| m.w_axis.truncate()),
                None => center,
            };
            let gi = match (d.lightmap, &d.light_probe) {
                (Some((index, st)), _) if (index as usize) < self.lightmap_count => {
                    gis.push(InstanceGi {
                        lightmap_st: st.to_array(),
                        info: [index as f32, 0.0, 0.0, 0.0],
                        sh: [[0.0; 4]; 9],
                    });
                    gis.len() as f32 - 1.0
                }
                (_, Some(sh)) => {
                    gis.push(InstanceGi {
                        lightmap_st: [0.0; 4],
                        info: [-1.0, 1.0, 0.0, 0.0],
                        sh: sh.map(|c| c.extend(0.0).to_array()),
                    });
                    gis.len() as f32 - 1.0
                }
                _ => -1.0,
            };
            resolved.push(Resolved {
                gi,
                probes: self.reflections.select(probe_pos, exclude),
                item: d,
                material: mat,
                center,
                radius,
                bounds: mesh.bounds,
                transparent: gm.transparent,
                cutout: gm.cutout,
                double_sided: gm.double_sided,
                cast: gm.cast_shadows && (d.cast_shadows || d.shadows_only),
                priority: gm.priority,
            });
        }

        let gi_bytes = (gis.len().max(1) * std::mem::size_of::<InstanceGi>()) as u64;
        if grow_buffer(
            device,
            &mut self.gi_buffer,
            gi_bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            "baked lighting",
        ) {
            self.groups_dirty = true;
        }
        if !gis.is_empty() {
            queue.write_buffer(&self.gi_buffer, 0, bytemuck::cast_slice(&gis));
        }

        // Terrain chunks: (draw, center, radius, local bounds).
        let mut terrain_items = Vec::new();
        for t in &frame.terrain_draws {
            if !self.gpu_assets.ensure_mesh(device, assets, t.mesh)
                || !self.terrain.prepare(device, queue, assets, t.data)
            {
                continue;
            }
            let mesh = &self.gpu_assets.meshes[&t.mesh];
            let c = t.model.transform_point3(mesh.bounds.center);
            terrain_items.push((t, c, mesh.bounds.extents.length(), mesh.bounds));
        }

        // ---- lights and joints --------------------------------------------------------
        // Atlas tiles for shadowed point/spot lights, nearest to the camera first.
        let view_pos = frame.cameras.first().map_or(Vector3::ZERO, |c| c.position);
        let mut shadowed: Vec<usize> = frame
            .lights
            .iter()
            .take(MAX_LIGHTS)
            .enumerate()
            .filter(|(_, l)| l.shadow.is_some_and(|s| s.strength > 0.0))
            .map(|(i, _)| i)
            .collect();
        shadowed.sort_by(|&a, &b| {
            let d =
                |i: usize| (frame.lights[i].position - view_pos).length() - frame.lights[i].range;
            d(a).total_cmp(&d(b))
        });
        let mut tiles: Vec<ShadowTiles> = Vec::new();
        let mut next_tile = 0;
        for i in shadowed {
            let l = &frame.lights[i];
            let near = (l.range * 0.01).max(0.05);
            let far = l.range.max(near + 0.1);
            let (rotations, fov): (Vec<oxide_math::Quaternion>, f32) = match l.spot {
                Some((dir, outer, _)) => {
                    let dir = dir.normalize_or(Vector3::Z);
                    let up = if dir.y.abs() > 0.99 {
                        Vector3::Z
                    } else {
                        Vector3::Y
                    };
                    (
                        vec![oxide_math::QuaternionExt::look_rotation(dir, up)],
                        (outer + 4.0).clamp(2.0, 170.0).to_radians(),
                    )
                }
                // Faces a little wider than 90° so filtering never reads past a tile edge.
                None => (
                    crate::reflection::face_rotations().to_vec(),
                    2.0 * (1.0 + 4.0 / ATLAS_TILE as f32).atan(),
                ),
            };
            if next_tile + rotations.len() > ATLAS_TILES {
                continue;
            }
            let proj = Matrix4x4::perspective(fov, 1.0, near, far);
            let matrices = rotations
                .iter()
                .map(|r| proj * Matrix4x4::look_to(l.position, *r * Vector3::Z, *r * Vector3::Y))
                .collect::<Vec<_>>();
            tiles.push(ShadowTiles {
                light: i,
                first: next_tile,
                texel: 2.0 * (fov * 0.5).tan() / ATLAS_TILE as f32,
                matrices,
            });
            next_tile += rotations.len();
        }
        let lights: Vec<LightUniform> = frame
            .lights
            .iter()
            .take(MAX_LIGHTS)
            .enumerate()
            .map(|(li, l)| {
                let (dir, spot, ty) = match l.spot {
                    Some((d, outer, inner)) => {
                        let co = (outer.to_radians() * 0.5).cos();
                        let ci = (inner.min(outer - 0.1).max(0.0).to_radians() * 0.5).cos();
                        (
                            d.normalize_or(Vector3::Z),
                            [co, 1.0 / (ci - co).max(1e-4), 0.0, 0.0],
                            1.0,
                        )
                    }
                    None => (Vector3::Z, [0.0; 4], 0.0),
                };
                let mut spot = spot;
                let mut shadow = [0.0; 4];
                if let (Some(t), Some(s)) = (tiles.iter().find(|t| t.light == li), l.shadow) {
                    spot[2] = (t.first + 1) as f32;
                    spot[3] = s.strength.clamp(0.0, 1.0);
                    shadow = [s.bias, s.normal_bias, s.soft as u32 as f32, t.texel];
                }
                LightUniform {
                    pos_range: l.position.extend(l.range.max(0.01)).to_array(),
                    color: [l.color[0], l.color[1], l.color[2], ty],
                    dir: dir.extend(0.0).to_array(),
                    spot,
                    shadow,
                }
            })
            .collect();
        if !lights.is_empty() {
            queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&lights));
        }
        let joint_bytes = (frame.joints.len() * 64) as u64;
        if grow_buffer(
            device,
            &mut self.joint_buffer,
            joint_bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            "joints",
        ) {
            self.groups_dirty = true;
        }
        if !frame.joints.is_empty() {
            let flat: Vec<f32> = frame
                .joints
                .iter()
                .flat_map(|m| m.to_cols_array())
                .collect();
            queue.write_buffer(&self.joint_buffer, 0, bytemuck::cast_slice(&flat));
        }
        // Forward+: light lists per cluster for each camera.
        let light_spheres: Vec<(Vector3, f32)> = frame
            .lights
            .iter()
            .take(MAX_LIGHTS)
            .map(|l| (l.position, l.range.max(0.01)))
            .collect();
        let aspect_of =
            |c: &CameraView| (w as f32 * c.viewport[2]) / (h as f32 * c.viewport[3]).max(1.0);
        let grids: Vec<crate::clusters::ClusterGrid> = frame
            .cameras
            .iter()
            .map(|c| {
                crate::clusters::assign(
                    &c.view(),
                    &c.projection(aspect_of(c)),
                    c.near,
                    c.far,
                    &light_spheres,
                )
            })
            .collect();
        self.rebuild_groups(device, frame.cameras.len().max(1));
        for (i, g) in grids.iter().enumerate() {
            let needed = (g.indices.len().max(1) * 4) as u64;
            if self.cluster_buffers[i].1.size() < needed {
                self.cluster_buffers[i].1 = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cluster lights"),
                    size: needed.next_power_of_two(),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.groups_dirty = true;
            }
        }
        self.rebuild_groups(device, frame.cameras.len().max(1));
        for (i, g) in grids.iter().enumerate() {
            queue.write_buffer(
                &self.cluster_buffers[i].0,
                0,
                bytemuck::cast_slice(&g.cells),
            );
            if !g.indices.is_empty() {
                queue.write_buffer(
                    &self.cluster_buffers[i].1,
                    0,
                    bytemuck::cast_slice(&g.indices),
                );
            }
        }

        let env = &frame.environment;
        let mut instances: Vec<Instance> = Vec::with_capacity(resolved.len());
        let make_instance = |r: &Resolved| {
            let d = r.item;
            let m = d.model;
            let n =
                Matrix4x4::from_mat3(oxide_math::glam::Mat3::from_mat4(m).inverse().transpose());
            let mut n1 = n.y_axis.to_array();
            n1[3] = r.gi;
            Instance {
                model: m.to_cols_array(),
                normal: [n.x_axis.to_array(), n1, n.z_axis.to_array()],
                extra: [
                    d.joint_offset.unwrap_or(0),
                    d.receive_shadows as u32,
                    r.probes[0],
                    r.probes[1],
                ],
            }
        };

        // Sorts shadow casters into instanced batches (instances appended to `instances`).
        let batch_casters = |mut casters: Vec<&Resolved>, instances: &mut Vec<Instance>| {
            casters.sort_by_key(|r| {
                (
                    r.cutout,
                    r.item.joint_offset.is_some(),
                    r.material,
                    r.item.mesh,
                    r.item.submesh,
                )
            });
            let mut list: Vec<ShadowBatch> = Vec::new();
            for r in casters {
                let key = ShadowKey {
                    cutout: r.cutout,
                    double_sided: r.double_sided,
                    skinned: r.item.joint_offset.is_some(),
                };
                let idx = instances.len() as u32;
                instances.push(make_instance(r));
                if let Some(last) = list.last_mut()
                    && !key.skinned
                    && last.key == key
                    && last.mesh == r.item.mesh
                    && last.submesh == r.item.submesh
                    && (!key.cutout || last.material == r.material)
                    && last.instances.end == idx
                {
                    last.instances.end += 1;
                    continue;
                }
                list.push(ShadowBatch {
                    key,
                    material: r.material,
                    mesh: r.item.mesh,
                    submesh: r.item.submesh,
                    instances: idx..idx + 1,
                });
            }
            list
        };

        // ---- shadows (from the first camera) ------------------------------------------
        let mut shadow_batches: Vec<Vec<ShadowBatch>> = (0..CASCADES).map(|_| Vec::new()).collect();
        let mut cascade_vp = [Matrix4x4::IDENTITY; CASCADES];
        let mut splits = [0.0f32; CASCADES];
        let mut texels = [0.0f32; CASCADES];
        let shadows = frame.sun.as_ref().and_then(|s| s.shadows.map(|sh| (s, sh)));
        let aspect0 =
            |c: &CameraView| (w as f32 * c.viewport[2]) / (h as f32 * c.viewport[3]).max(1.0);
        if let (Some((sun, _)), Some(cam)) = (shadows, frame.cameras.first()) {
            let cascades = match capture {
                Some(c) => cascades_around(
                    c.center,
                    cam.near,
                    sun.direction,
                    env.shadow_distance,
                    self.shadow_size,
                ),
                None => compute_cascades(
                    cam,
                    aspect0(cam),
                    sun.direction,
                    env.shadow_distance,
                    self.shadow_size,
                ),
            };
            for (i, c) in cascades.iter().enumerate() {
                cascade_vp[i] = c.view_proj;
                splits[i] = c.split;
                texels[i] = c.texel;
                queue.write_buffer(
                    &self.shadow_buffers[i],
                    0,
                    bytemuck::cast_slice(&c.view_proj.to_cols_array()),
                );
                let casters = resolved
                    .iter()
                    .filter(|r| r.cast && !r.transparent && c.overlaps(r.center, r.radius))
                    .collect();
                let list = batch_casters(casters, &mut instances);
                shadow_batches[i] = list;
            }
        }

        // Additional-light shadow casters per atlas tile.
        let mut tile_batches: Vec<(usize, Vec<ShadowBatch>)> = Vec::new();
        let mut tile_matrices = vec![[0.0f32; 16]; ATLAS_TILES];
        for t in &tiles {
            let l = &frame.lights[t.light];
            for (f, m) in t.matrices.iter().enumerate() {
                let tile = t.first + f;
                tile_matrices[tile] = m.to_cols_array();
                queue.write_buffer(
                    &self.atlas_buffers[tile],
                    0,
                    bytemuck::cast_slice(&m.to_cols_array()),
                );
                let planes = frustum_planes(m);
                let casters = resolved
                    .iter()
                    .filter(|r| {
                        r.cast
                            && !r.transparent
                            && (!r.radius.is_finite()
                                || ((r.center - l.position).length() < r.radius + l.range
                                    && sphere_visible(&planes, r.center, r.radius)))
                    })
                    .collect();
                tile_batches.push((tile, batch_casters(casters, &mut instances)));
            }
        }
        if !tiles.is_empty() {
            queue.write_buffer(
                &self.atlas_matrices,
                0,
                bytemuck::cast_slice(&tile_matrices),
            );
        }

        // ---- per-camera culling and batching -------------------------------------------
        let mut camera_batches: Vec<(Vec<Batch>, Vec<Batch>)> = Vec::new();
        let mut camera_features: Vec<Vec<(RenderPassEvent, Vec<Batch>)>> = Vec::new();
        // Override materials of Render Objects features: (id, transparent, double sided).
        let feature_materials: Vec<Option<(AssetId, bool, bool)>> = frame
            .renderer_features
            .iter()
            .map(|f| match f {
                RendererFeature::RenderObjects {
                    override_material: Some(m),
                    ..
                } => {
                    let id = self.gpu_assets.ensure_material(device, queue, assets, *m);
                    let gm = &self.gpu_assets.materials[&id];
                    Some((id, gm.transparent, gm.double_sided))
                }
                _ => None,
            })
            .collect();
        // (jittered, unjittered view-projection, frustum) of the TAA camera.
        let mut taa_matrices: Option<(Matrix4x4, Matrix4x4, [Vector4; 6])> = None;
        // Projection (jittered with TAA) of the first camera, for SSAO.
        let mut cam0_proj = Matrix4x4::IDENTITY;
        let mut camera_terrain: Vec<Vec<(AssetId, AssetId, u32)>> = Vec::new();
        let mut occlusion = std::mem::take(&mut self.occlusion);
        occlusion.resize_with(frame.cameras.len(), Default::default);
        for (ci, cam) in frame.cameras.iter().enumerate() {
            let aspect = aspect0(cam);
            let vp = cam.view_projection(aspect);
            let planes = frustum_planes(&vp);
            // TAA: shift the projection by a sub-pixel amount each frame (culling stays
            // unjittered).
            let draw_vp = if taa {
                let (jx, jy) = crate::aa::jitter(frame.frame_index);
                let s = aa.taa_jitter_scale;
                let pw = (w as f32 * cam.viewport[2]).max(1.0);
                let ph = (h as f32 * cam.viewport[3]).max(1.0);
                let j = Matrix4x4::from_translation(Vector3::new(
                    2.0 * jx * s / pw,
                    2.0 * jy * s / ph,
                    0.0,
                ));
                let jittered = j * vp;
                taa_matrices = Some((jittered, vp, planes));
                if ci == 0 {
                    cam0_proj = j * cam.projection(aspect);
                }
                jittered
            } else {
                if ci == 0 {
                    cam0_proj = cam.projection(aspect);
                }
                vp
            };
            let occ = &mut occlusion[ci];
            let occlusion_on = cam.occlusion_culling;
            if occlusion_on {
                let started = std::time::Instant::now();
                occ.begin(vp, aspect);
                // Biggest on screen first, until the triangle budget runs out.
                enum Occluder<'a> {
                    Draw(&'a Resolved<'a>),
                    Terrain(&'a TerrainDraw, AssetId),
                    Box(&'a Matrix4x4),
                }
                let mut occluders: Vec<(f32, Occluder)> = Vec::new();
                let size = |c: Vector3, r: f32| r / (c - cam.position).length().max(0.01);
                for r in &resolved {
                    if r.item.occluder
                        && !r.item.shadows_only
                        && !r.transparent
                        && !r.cutout
                        && r.radius.is_finite()
                        && cam.culling_mask & (1u32 << (r.item.layer & 31)) != 0
                        && sphere_visible(&planes, r.center, r.radius)
                    {
                        let s = size(r.center, r.radius);
                        if s >= MIN_OCCLUDER_SIZE {
                            occluders.push((s, Occluder::Draw(r)));
                        }
                    }
                }
                for (t, c, r, _) in &terrain_items {
                    if let Some(m) = t.occluder
                        && sphere_visible(&planes, *c, *r)
                    {
                        occluders.push((size(*c, *r), Occluder::Terrain(t, m)));
                    }
                }
                for m in &frame.occluder_boxes {
                    occluders.push((f32::INFINITY, Occluder::Box(m)));
                }
                occluders.sort_by(|a, b| b.0.total_cmp(&a.0));
                for (_, o) in &occluders {
                    if occ.triangles >= OCCLUDER_TRIANGLE_BUDGET {
                        break;
                    }
                    match o {
                        Occluder::Draw(r) => {
                            let Some(m) = assets.meshes.get_by_id(r.item.mesh) else {
                                continue;
                            };
                            let Some(range) = m.submeshes.get(r.item.submesh as usize) else {
                                continue;
                            };
                            if (range.end - range.start) / 3 > MAX_OCCLUDER_TRIANGLES {
                                continue;
                            }
                            let idx = &m.indices[range.start as usize..range.end as usize];
                            occ.rasterize(&r.item.model, &m.vertices, idx);
                        }
                        Occluder::Terrain(t, mesh) => {
                            if let Some(m) = assets.meshes.get_by_id(*mesh) {
                                occ.rasterize(&t.model, &m.vertices, &m.indices);
                            }
                        }
                        Occluder::Box(m) => occ.rasterize_box(m),
                    }
                    stats.occluders += 1;
                }
                occ.finish();
                stats.occluder_triangles += occ.triangles;
                stats.occlusion_ms += started.elapsed().as_secs_f32() * 1000.0;
            } else {
                *occ = Default::default();
            }
            let mut opaque: Vec<&Resolved> = Vec::new();
            let mut transparent: Vec<(f32, &Resolved)> = Vec::new();
            for r in &resolved {
                if r.item.shadows_only || cam.culling_mask & (1u32 << (r.item.layer & 31)) == 0 {
                    continue;
                }
                if r.radius.is_finite() && !sphere_visible(&planes, r.center, r.radius) {
                    stats.culled += 1;
                    continue;
                }
                if occlusion_on
                    && r.item.occludee
                    && r.radius.is_finite()
                    && !occ.is_visible(&r.item.model, r.bounds.center, r.bounds.extents)
                {
                    stats.occlusion_culled += 1;
                    continue;
                }
                if r.transparent {
                    transparent.push(((r.center - cam.position).length_squared(), r));
                } else {
                    opaque.push(r);
                }
            }
            opaque.sort_by_key(|r| {
                (
                    r.double_sided,
                    r.item.joint_offset.is_some(),
                    r.material,
                    r.item.mesh,
                    r.item.submesh,
                )
            });
            transparent.sort_by(|a, b| {
                a.1.priority
                    .cmp(&b.1.priority)
                    .then(b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal))
            });
            let mut ob: Vec<Batch> = Vec::new();
            for r in opaque {
                let key = MainKey {
                    samples,
                    transparent: false,
                    double_sided: r.double_sided,
                    skinned: r.item.joint_offset.is_some(),
                    depth_test: DepthTest::LessEqual,
                    depth_write: true,
                    wire: false,
                };
                let idx = instances.len() as u32;
                instances.push(make_instance(r));
                if let Some(last) = ob.last_mut()
                    && !key.skinned
                    && last.key == key
                    && last.material == r.material
                    && last.mesh == r.item.mesh
                    && last.submesh == r.item.submesh
                    && last.instances.end == idx
                {
                    last.instances.end += 1;
                    continue;
                }
                ob.push(Batch {
                    key,
                    material: r.material,
                    mesh: r.item.mesh,
                    submesh: r.item.submesh,
                    instances: idx..idx + 1,
                });
            }
            let tb: Vec<Batch> = transparent
                .into_iter()
                .map(|(_, r)| {
                    let idx = instances.len() as u32;
                    instances.push(make_instance(r));
                    Batch {
                        key: MainKey {
                            samples,
                            transparent: true,
                            double_sided: r.double_sided,
                            skinned: r.item.joint_offset.is_some(),
                            depth_test: DepthTest::LessEqual,
                            depth_write: false,
                            wire: false,
                        },
                        material: r.material,
                        mesh: r.item.mesh,
                        submesh: r.item.submesh,
                        instances: idx..idx + 1,
                    }
                })
                .collect();
            // Pure Wireframe draws edges only (when the GPU can draw them).
            if frame.debug_view.shaded() || !self.wire_supported {
                camera_batches.push((ob, tb));
            } else {
                camera_batches.push((Vec::new(), Vec::new()));
            }

            // Render Objects features: the camera's renderers on their layers, again.
            let mut fb: Vec<(RenderPassEvent, Vec<Batch>)> = Vec::new();
            for (feature, override_mat) in frame.renderer_features.iter().zip(&feature_materials) {
                let RendererFeature::RenderObjects {
                    event,
                    layer_mask,
                    depth_test,
                    write_depth,
                    ..
                } = feature
                else {
                    continue;
                };
                let mut list = Vec::new();
                for r in &resolved {
                    let bit = 1u32 << (r.item.layer & 31);
                    if r.item.shadows_only
                        || cam.culling_mask & layer_mask & bit == 0
                        || (r.radius.is_finite() && !sphere_visible(&planes, r.center, r.radius))
                    {
                        continue;
                    }
                    let (material, transparent, double_sided) = match override_mat {
                        Some((m, t, d)) => (*m, *t, *d),
                        None => (r.material, r.transparent, r.double_sided),
                    };
                    let idx = instances.len() as u32;
                    instances.push(make_instance(r));
                    list.push(Batch {
                        key: MainKey {
                            samples,
                            transparent,
                            double_sided,
                            skinned: r.item.joint_offset.is_some(),
                            depth_test: *depth_test,
                            depth_write: *write_depth,
                            wire: false,
                        },
                        material,
                        mesh: r.item.mesh,
                        submesh: r.item.submesh,
                        instances: idx..idx + 1,
                    });
                }
                fb.push((*event, list));
            }
            // Wireframe draw modes: every visible renderer's edges after the transparents.
            if frame.debug_view.wire() && self.wire_supported {
                let mut list = Vec::new();
                for r in &resolved {
                    let bit = 1u32 << (r.item.layer & 31);
                    if r.item.shadows_only
                        || cam.culling_mask & bit == 0
                        || (r.radius.is_finite() && !sphere_visible(&planes, r.center, r.radius))
                    {
                        continue;
                    }
                    let idx = instances.len() as u32;
                    instances.push(make_instance(r));
                    list.push(Batch {
                        key: MainKey {
                            samples,
                            transparent: true,
                            double_sided: true,
                            skinned: r.item.joint_offset.is_some(),
                            depth_test: DepthTest::LessEqual,
                            depth_write: false,
                            wire: true,
                        },
                        material: r.material,
                        mesh: r.item.mesh,
                        submesh: r.item.submesh,
                        instances: idx..idx + 1,
                    });
                }
                fb.push((RenderPassEvent::AfterRenderingTransparents, list));
            }
            camera_features.push(fb);
            let mut tl = Vec::new();
            for (t, c, r, b) in &terrain_items {
                let probes = self.reflections.select(*c, exclude);
                if !sphere_visible(&planes, *c, *r) {
                    stats.culled += 1;
                    continue;
                }
                if occlusion_on && !occ.is_visible(&t.model, b.center, b.extents) {
                    stats.occlusion_culled += 1;
                    continue;
                }
                let idx = instances.len() as u32;
                instances.push(Instance {
                    model: t.model.to_cols_array(),
                    normal: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, -1.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    extra: [0, t.receive_shadows as u32, probes[0], probes[1]],
                });
                tl.push((t.data, t.mesh, idx));
            }
            camera_terrain.push(tl);

            // Frame uniform for this camera.
            let view = cam.view();
            let sun_dir = frame
                .sun
                .as_ref()
                .map_or(Vector3::new(0.3, -0.8, 0.5).normalize(), |s| {
                    s.direction.normalize_or(Vector3::NEG_Y)
                });
            let (sun_color, strength, sp) = match (&frame.sun, shadows) {
                (Some(s), Some((_, sh))) => (
                    s.color,
                    sh.strength,
                    [
                        sh.bias,
                        sh.normal_bias,
                        sh.soft as u32 as f32,
                        CASCADES as f32,
                    ],
                ),
                (Some(s), None) => (s.color, 0.0, [0.0; 4]),
                (None, _) => ([0.0; 3], 0.0, [0.0; 4]),
            };
            let tint = env.sky_tint.linear();
            let ground = env.ground_color.linear();
            // Base sky colors modulated by the tint (0.5 gray = neutral, like Unity).
            // sRGB 0.5 gray (linear 0.214) is the neutral tint.
            let nt = Vector3::new(tint.r, tint.g, tint.b) / 0.214;
            let zenith = Vector3::new(0.075, 0.17, 0.46) * nt;
            let horizon = Vector3::new(0.36, 0.44, 0.56) * nt.powf(0.5);
            let (fog_mode, fog_p, fog_c) = match env.fog {
                None => (0.0, [0.0; 3], [0.0; 3]),
                Some((kind, c)) => {
                    let c = c.linear();
                    match kind {
                        FogKind::Linear { start, end } => (1.0, [0.0, start, end], [c.r, c.g, c.b]),
                        FogKind::Exponential { density } => {
                            (2.0, [density, 0.0, 0.0], [c.r, c.g, c.b])
                        }
                        FogKind::ExponentialSquared { density } => {
                            (3.0, [density, 0.0, 0.0], [c.r, c.g, c.b])
                        }
                    }
                }
            };
            let clear = match cam.clear {
                ClearMode::Color(c) => [c.r, c.g, c.b, 1.0],
                _ => [0.0, 0.0, 0.0, 1.0],
            };
            let mut cvp = [[0.0f32; 16]; CASCADES];
            for i in 0..CASCADES {
                cvp[i] = cascade_vp[i].to_cols_array();
            }
            let u = FrameUniform {
                view_proj: draw_vp.to_cols_array(),
                view: view.to_cols_array(),
                inv_view_proj: draw_vp.inverse().to_cols_array(),
                camera_pos: cam.position.extend(frame.time).to_array(),
                sun_dir: sun_dir.extend(frame.sun.is_some() as u32 as f32).to_array(),
                sun_color: [sun_color[0], sun_color[1], sun_color[2], strength],
                shadow_params: sp,
                cascade_splits: splits,
                cascade_vp: cvp,
                sky_zenith: zenith.extend(env.sky_exposure).to_array(),
                sky_horizon: horizon.extend(env.sun_size).to_array(),
                ground_color: [ground.r, ground.g, ground.b, env.ambient_intensity],
                fog_color: [fog_c[0], fog_c[1], fog_c[2], fog_mode],
                fog_params: [fog_p[0], fog_p[1], fog_p[2], env.reflection_intensity],
                misc: [
                    lights.len() as f32,
                    frame.debug_view.shader_mode() as f32,
                    w as f32,
                    h as f32,
                ],
                clear_color: clear,
                cascade_texel: texels,
                ssao: [
                    ssao.is_some() as u32 as f32,
                    ssao.map_or(0.0, |s| s.direct_lighting_strength.clamp(0.0, 1.0)),
                    0.0,
                    0.0,
                ],
                cluster_viewport: {
                    let [vx, vy, vw, vh] = cam.viewport;
                    let ph = vh * h as f32;
                    [
                        vx * w as f32,
                        h as f32 - vy * h as f32 - ph,
                        (vw * w as f32).max(1.0),
                        ph.max(1.0),
                    ]
                },
                cluster_params: [grids[ci].scale, grids[ci].bias, 0.0, 0.0],
            };
            queue.write_buffer(&self.frame_buffers[ci], 0, bytemuck::bytes_of(&u));
        }

        // ---- upload instances and lines ------------------------------------------------
        let inst_bytes = (instances.len() * std::mem::size_of::<Instance>()) as u64;
        grow_buffer(
            device,
            &mut self.instance_buffer,
            inst_bytes,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "instances",
        );
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }
        let line_bytes = (frame.lines.len() * std::mem::size_of::<LineVertex>()) as u64;
        grow_buffer(
            device,
            &mut self.line_buffer,
            line_bytes,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "lines",
        );
        if !frame.lines.is_empty() {
            queue.write_buffer(&self.line_buffer, 0, bytemuck::cast_slice(&frame.lines));
        }

        // Make sure every pipeline exists before borrowing for the passes.
        for (o, t) in &camera_batches {
            for b in o.iter().chain(t) {
                self.main_pipeline(device, b.key);
            }
        }
        for b in camera_features.iter().flatten().flat_map(|(_, list)| list) {
            self.main_pipeline(device, b.key);
        }
        for list in shadow_batches
            .iter()
            .chain(tile_batches.iter().map(|(_, l)| l))
        {
            for b in list {
                self.shadow_pipeline(device, b.key);
            }
        }

        let cam0 = frame.cameras.first().map_or(Vector3::ZERO, |c| c.position);
        let particle_ranges = self.particles.prepare(
            device,
            queue,
            &mut self.gpu_assets,
            assets,
            &frame.particles,
            cam0,
        );
        let sprite_ranges = self.sprites.prepare(
            device,
            queue,
            &mut self.gpu_assets,
            assets,
            &frame.sprites,
            cam0,
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene"),
        });

        // ---- shadow passes ------------------------------------------------------------
        if shadows.is_some() {
            for (i, list) in shadow_batches.iter().enumerate() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow cascade"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow_layers[i],
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_bind_group(0, &self.shadow_groups[i], &[]);
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                for b in list {
                    let mesh = &self.gpu_assets.meshes[&b.mesh];
                    let mat = &self.gpu_assets.materials[&b.material];
                    pass.set_pipeline(&self.shadow_pipelines[&b.key]);
                    pass.set_bind_group(1, &mat.bind_group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    if b.key.skinned {
                        let Some(skin) = &mesh.skin else { continue };
                        pass.set_vertex_buffer(2, skin.slice(..));
                    }
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(
                        mesh.submeshes[b.submesh as usize].clone(),
                        0,
                        b.instances.clone(),
                    );
                    stats.shadow_draw_calls += 1;
                }
            }
        }

        // ---- point/spot shadow atlas ------------------------------------------------------
        if !tile_batches.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow atlas"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.atlas,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            let per_row = ATLAS_SIZE / ATLAS_TILE;
            for (tile, list) in &tile_batches {
                let (tx, ty) = (*tile as u32 % per_row, *tile as u32 / per_row);
                let (x, y) = ((tx * ATLAS_TILE) as f32, (ty * ATLAS_TILE) as f32);
                pass.set_viewport(x, y, ATLAS_TILE as f32, ATLAS_TILE as f32, 0.0, 1.0);
                pass.set_scissor_rect(x as u32, y as u32, ATLAS_TILE, ATLAS_TILE);
                pass.set_bind_group(0, &self.atlas_groups[*tile], &[]);
                for b in list {
                    let mesh = &self.gpu_assets.meshes[&b.mesh];
                    let mat = &self.gpu_assets.materials[&b.material];
                    pass.set_pipeline(&self.shadow_pipelines[&b.key]);
                    pass.set_bind_group(1, &mat.bind_group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    if b.key.skinned {
                        let Some(skin) = &mesh.skin else { continue };
                        pass.set_vertex_buffer(2, skin.slice(..));
                    }
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(
                        mesh.submeshes[b.submesh as usize].clone(),
                        0,
                        b.instances.clone(),
                    );
                    stats.shadow_draw_calls += 1;
                }
            }
        }

        // ---- SSAO: depth prepass of the first camera, then AO ---------------------------
        if let (Some(settings), Some(cam)) = (ssao, frame.cameras.first()) {
            for b in &camera_batches[0].0 {
                let cutout = self.gpu_assets.materials[&b.material].cutout;
                self.prepass_pipeline(device, (cutout, b.key.double_sided, b.key.skinned));
            }
            let t = self.targets.get_mut(&(w, h, samples)).expect("targets");
            if t.ssao.is_none() {
                t.ssao = Some(self.ssao.targets(device, w, h));
            }
            let st = self.targets[&(w, h, samples)]
                .ssao
                .as_ref()
                .expect("made above");
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("depth prepass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &st.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                let [vx, vy, vw, vh] = cam.viewport;
                let px = (vx * w as f32).clamp(0.0, w as f32);
                let pw = (vw * w as f32).clamp(1.0, w as f32 - px);
                let ph = (vh * h as f32).clamp(1.0, h as f32);
                let py = (h as f32 - (vy * h as f32) - ph).clamp(0.0, h as f32 - ph);
                pass.set_viewport(px, py, pw, ph, 0.0, 1.0);
                pass.set_bind_group(0, &self.frame_groups[0], &[]);
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                if !camera_terrain[0].is_empty() {
                    pass.set_pipeline(&self.terrain_prepass);
                    for &(data, mesh_id, idx) in &camera_terrain[0] {
                        let (Some(bg), Some(mesh)) = (
                            self.terrain.bind_group(data),
                            self.gpu_assets.meshes.get(&mesh_id),
                        ) else {
                            continue;
                        };
                        pass.set_bind_group(1, bg, &[]);
                        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(mesh.submeshes[0].clone(), 0, idx..idx + 1);
                    }
                }
                for b in &camera_batches[0].0 {
                    let mesh = &self.gpu_assets.meshes[&b.mesh];
                    let mat = &self.gpu_assets.materials[&b.material];
                    pass.set_pipeline(
                        &self.prepass_pipelines[&(mat.cutout, b.key.double_sided, b.key.skinned)],
                    );
                    pass.set_bind_group(1, &mat.bind_group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    if b.key.skinned {
                        let Some(skin) = &mesh.skin else { continue };
                        pass.set_vertex_buffer(2, skin.slice(..));
                    }
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(
                        mesh.submeshes[b.submesh as usize].clone(),
                        0,
                        b.instances.clone(),
                    );
                }
            }
            self.ssao
                .run(device, queue, &mut encoder, st, cam0_proj, &settings);
        }

        // ---- camera passes -------------------------------------------------------------
        let targets = self.targets.get(&(w, h, samples)).expect("targets");
        let screen_group = match (&ssao, &targets.ssao) {
            (Some(_), Some(st)) => &st.screen_group,
            _ => &self.ssao.white_group,
        };
        let fixed = &self.fixed[&samples];
        let (color_view, resolve) = match &targets.msaa {
            Some(m) => (m, Some(&targets.resolve)),
            None => (&targets.resolve, None),
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cameras"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        // TAA and depth-based post effects read depth afterwards.
                        store: if taa || fx {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            for (ci, cam) in frame.cameras.iter().enumerate() {
                let [vx, vy, vw, vh] = cam.viewport;
                let px = (vx * w as f32).clamp(0.0, w as f32);
                let pw = (vw * w as f32).clamp(1.0, w as f32 - px);
                let ph = (vh * h as f32).clamp(1.0, h as f32);
                let py = (h as f32 - (vy * h as f32) - ph).clamp(0.0, h as f32 - ph);
                pass.set_viewport(px, py, pw, ph, 0.0, 1.0);
                pass.set_scissor_rect(px as u32, py as u32, (pw as u32).max(1), (ph as u32).max(1));
                pass.set_bind_group(0, &self.frame_groups[ci], &[]);
                pass.set_bind_group(2, screen_group, &[]);
                match cam.clear {
                    ClearMode::Sky => pass.set_pipeline(&fixed.sky),
                    ClearMode::Color(_) => pass.set_pipeline(&fixed.solid),
                    ClearMode::DepthOnly => pass.set_pipeline(&fixed.depth_clear),
                    ClearMode::Nothing => {}
                }
                if cam.clear != ClearMode::Nothing {
                    pass.draw(0..3, 0..1);
                }
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                if !camera_terrain[ci].is_empty() {
                    pass.set_pipeline(&fixed.terrain);
                    for &(data, mesh_id, idx) in &camera_terrain[ci] {
                        let (Some(bg), Some(mesh)) = (
                            self.terrain.bind_group(data),
                            self.gpu_assets.meshes.get(&mesh_id),
                        ) else {
                            continue;
                        };
                        pass.set_bind_group(1, bg, &[]);
                        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                        let range = mesh.submeshes[0].clone();
                        stats.triangles += ((range.end - range.start) / 3) as u64;
                        stats.draw_calls += 1;
                        pass.draw_indexed(range, 0, idx..idx + 1);
                    }
                }
                let draw_batches = |pass: &mut wgpu::RenderPass,
                                    list: &[Batch],
                                    stats: &mut RenderStats| {
                    for b in list {
                        let mesh = &self.gpu_assets.meshes[&b.mesh];
                        let mat = &self.gpu_assets.materials[&b.material];
                        pass.set_pipeline(&self.main_pipelines[&b.key]);
                        pass.set_bind_group(1, &mat.bind_group, &[]);
                        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                        if b.key.skinned {
                            let Some(skin) = &mesh.skin else { continue };
                            pass.set_vertex_buffer(2, skin.slice(..));
                        }
                        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                        let range = mesh.submeshes[b.submesh as usize].clone();
                        let n = b.instances.end - b.instances.start;
                        stats.triangles += ((range.end - range.start) / 3) as u64 * n as u64;
                        stats.instances += n;
                        stats.draw_calls += 1;
                        pass.draw_indexed(range, 0, b.instances.clone());
                    }
                };
                let features_at = |event: RenderPassEvent| {
                    camera_features[ci]
                        .iter()
                        .filter(move |(e, _)| *e == event)
                        .map(|(_, l)| l)
                };
                let (ob, tb) = &camera_batches[ci];
                draw_batches(&mut pass, ob, &mut stats);
                for list in features_at(RenderPassEvent::AfterRenderingOpaques) {
                    draw_batches(&mut pass, list, &mut stats);
                }
                draw_batches(&mut pass, tb, &mut stats);
                if !particle_ranges.is_empty() {
                    stats.draw_calls += self.particles.draw(&mut pass, &particle_ranges, samples);
                }
                if !sprite_ranges.is_empty() {
                    stats.draw_calls += self.sprites.draw(&mut pass, &sprite_ranges, samples);
                }
                for list in features_at(RenderPassEvent::AfterRenderingTransparents) {
                    draw_batches(&mut pass, list, &mut stats);
                }
                if !frame.lines.is_empty() {
                    pass.set_pipeline(&fixed.line);
                    pass.set_vertex_buffer(0, self.line_buffer.slice(..));
                    pass.draw(0..frame.lines.len() as u32, 0..1);
                }
            }
        }

        self.occlusion = occlusion;

        // ---- TAA: motion vectors, then the history resolve ----------------------------
        let mut taa_key = None;
        if let (true, Some((jittered, vp, planes)), Some(cam)) =
            (taa, taa_matrices, frame.cameras.first())
        {
            // Visible opaque objects that moved since the last frame.
            let mut moving = Vec::new();
            for r in &resolved {
                let d = r.item;
                if r.transparent
                    || d.shadows_only
                    || d.id == 0
                    || cam.culling_mask & (1u32 << (d.layer & 31)) == 0
                    || (r.radius.is_finite() && !sphere_visible(&planes, r.center, r.radius))
                {
                    continue;
                }
                let instance = match d.joint_offset {
                    Some(offset) => match self.motion.prev_joint_offsets.get(&d.id) {
                        Some(&prev) => crate::aa::MotionInstance {
                            model: Matrix4x4::IDENTITY.to_cols_array(),
                            prev: Matrix4x4::IDENTITY.to_cols_array(),
                            extra: [offset, prev, 0, 0],
                        },
                        None => continue,
                    },
                    None => match self.motion.prev_models.get(&(d.id, d.submesh)) {
                        Some(prev) if *prev != d.model => crate::aa::MotionInstance {
                            model: d.model.to_cols_array(),
                            prev: prev.to_cols_array(),
                            extra: [0; 4],
                        },
                        _ => continue,
                    },
                };
                moving.push(MotionDraw {
                    mesh: d.mesh,
                    submesh: d.submesh,
                    skinned: d.joint_offset.is_some(),
                    instance,
                });
            }
            let key = (aa.history_id, w, h);
            taa_key = Some(key);
            self.taa_resolve(
                gpu,
                &mut encoder,
                &moving,
                (w, h, samples),
                key,
                cam,
                (jittered, vp),
                aa,
            );
        }
        (
            encoder,
            stats,
            SceneOutput {
                samples,
                taa: taa_key,
                fxaa: mode == AntiAliasing::Fxaa,
                fx,
            },
        )
    }
}

/// Lightmaps (RGBM RGBA8, top row first, all `size`×`size`) as a texture array; a 1×1 black
/// layer when there are none.
fn lightmap_array(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &[&[u8]],
    size: u32,
) -> wgpu::TextureView {
    use wgpu::util::DeviceExt;
    let (data, size, count) = if layers.is_empty() {
        (vec![0u8; 4], 1, 1)
    } else {
        (layers.concat(), size, layers.len() as u32)
    };
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("lightmaps"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: count,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &data,
        )
        .create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        })
}

impl SceneRenderer {
    /// Rebuilds the lightmap array when the scene's lightmaps (or their files) change.
    fn update_lightmaps(&mut self, gpu: &Gpu, assets: &AssetServer, ids: &[AssetId]) {
        let sources: Vec<(AssetId, u64)> = ids
            .iter()
            .map(|id| (*id, assets.textures.version(*id).unwrap_or(0)))
            .collect();
        if sources == self.lightmap_sources {
            return;
        }
        self.lightmap_sources = sources;
        let textures: Vec<&Texture2D> = ids
            .iter()
            .filter_map(|id| assets.textures.get_by_id(*id))
            .filter(|t| !t.mips.is_empty())
            .collect();
        let size = textures.first().map_or(1, |t| t.width);
        let layers: Vec<&[u8]> = textures
            .iter()
            .take_while(|t| t.width == size && t.height == size)
            .map(|t| &t.mips[0][..])
            .collect();
        if layers.len() < ids.len() {
            oxide_core::debug_log_warning!(
                "{} of {} lightmaps are missing or differ in size; those objects use ambient light",
                ids.len() - layers.len(),
                ids.len()
            );
        }
        self.lightmap_count = layers.len();
        self.lightmap_array = lightmap_array(&gpu.device, &gpu.queue, &layers, size);
        self.groups_dirty = true;
    }
}

/// What `render_scene` produced, for post-processing.
struct SceneOutput {
    samples: u32,
    /// TAA history holding the anti-aliased HDR image (else it is the resolve target).
    taa: Option<(u64, u32, u32)>,
    fxaa: bool,
    /// Depth of field or motion blur runs.
    fx: bool,
}

/// An object in the motion-vector pass.
struct MotionDraw {
    mesh: AssetId,
    submesh: u32,
    skinned: bool,
    instance: crate::aa::MotionInstance,
}

impl MotionHistory {
    /// On a new frame, the current transforms become the previous ones and `frame`'s are
    /// recorded. Repeated renders of the same frame change nothing.
    fn advance(&mut self, frame: &RenderFrame) {
        if self.frame == Some(frame.frame_index) {
            return;
        }
        self.frame = Some(frame.frame_index);
        std::mem::swap(&mut self.prev_models, &mut self.curr_models);
        std::mem::swap(&mut self.prev_joints, &mut self.curr_joints);
        std::mem::swap(&mut self.prev_joint_offsets, &mut self.curr_joint_offsets);
        self.curr_models.clear();
        self.curr_joint_offsets.clear();
        for d in frame.draws.iter().filter(|d| d.id != 0) {
            match d.joint_offset {
                Some(o) => {
                    self.curr_joint_offsets.insert(d.id, o);
                }
                None => {
                    self.curr_models.insert((d.id, d.submesh), d.model);
                }
            }
        }
        self.curr_joints.clone_from(&frame.joints);
    }
}

impl SceneRenderer {
    /// Motion vectors for `moving`, then TAA from the scene target and the view's history
    /// into the history's other texture.
    #[allow(clippy::too_many_arguments)]
    fn taa_resolve(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        moving: &[MotionDraw],
        target_key: (u32, u32, u32),
        key: (u64, u32, u32),
        cam: &CameraView,
        (jittered, vp): (Matrix4x4, Matrix4x4),
        aa: AntiAliasingSettings,
    ) {
        let device = &gpu.device;
        let queue = &gpu.queue;
        let (w, h, _) = target_key;
        let hdr_tex = |label: &str, format: wgpu::TextureFormat| {
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
        let now = self.frame_index;
        if !self.taa_histories.contains_key(&key) {
            while self.taa_histories.len() >= 4 {
                let oldest = *self
                    .taa_histories
                    .iter()
                    .min_by_key(|(_, t)| t.last_used)
                    .expect("non-empty")
                    .0;
                self.taa_histories.remove(&oldest);
            }
            self.taa_histories.insert(
                key,
                TaaHistory {
                    textures: [
                        hdr_tex("taa history", HDR_FORMAT),
                        hdr_tex("taa history", HDR_FORMAT),
                    ],
                    current: 0,
                    valid: false,
                    prev_vp: vp,
                    position: cam.position,
                    forward: cam.rotation * Vector3::Z,
                    last_used: now,
                },
            );
        }
        let targets = self.targets.get_mut(&target_key).expect("targets");
        targets
            .motion
            .get_or_insert_with(|| hdr_tex("motion vectors", crate::aa::MOTION_FORMAT));
        let targets = &self.targets[&target_key];
        let hist = self.taa_histories.get_mut(&key).expect("inserted");
        hist.last_used = now;
        // A camera cut (teleport or sudden turn) starts over instead of smearing.
        let forward = cam.rotation * Vector3::Z;
        let cut = cam.position.distance(hist.position) > 10.0 || forward.dot(hist.forward) < 0.7;
        let valid = hist.valid && !cut;
        let prev_vp = if valid { hist.prev_vp } else { vp };

        // Motion vectors.
        let joint_bytes = (self.motion.prev_joints.len() * 64) as u64;
        grow_buffer(
            device,
            &mut self.prev_joint_buffer,
            joint_bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            "previous joints",
        );
        if !self.motion.prev_joints.is_empty() {
            let flat: Vec<f32> = self
                .motion
                .prev_joints
                .iter()
                .flat_map(|m| m.to_cols_array())
                .collect();
            queue.write_buffer(&self.prev_joint_buffer, 0, bytemuck::cast_slice(&flat));
        }
        let instances: Vec<crate::aa::MotionInstance> = moving.iter().map(|m| m.instance).collect();
        grow_buffer(
            device,
            &mut self.motion_buffer,
            (instances.len() * std::mem::size_of::<crate::aa::MotionInstance>()) as u64,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "motion instances",
        );
        if !instances.is_empty() {
            queue.write_buffer(&self.motion_buffer, 0, bytemuck::cast_slice(&instances));
        }
        let motion_view = targets.motion.as_ref().expect("made above");
        {
            let group = self.aa.motion_group(
                device,
                queue,
                jittered.to_cols_array(),
                vp.to_cols_array(),
                prev_vp.to_cols_array(),
                &self.joint_buffer,
                &self.prev_joint_buffer,
                &targets.depth,
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("motion vectors"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: motion_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &group, &[]);
            pass.set_vertex_buffer(1, self.motion_buffer.slice(..));
            for (i, m) in moving.iter().enumerate() {
                let Some(mesh) = self.gpu_assets.meshes.get(&m.mesh) else {
                    continue;
                };
                if m.skinned {
                    let Some(skin) = &mesh.skin else { continue };
                    pass.set_pipeline(&self.aa.motion_skinned);
                    pass.set_vertex_buffer(2, skin.slice(..));
                } else {
                    pass.set_pipeline(&self.aa.motion_static);
                }
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                let i = i as u32;
                pass.draw_indexed(mesh.submeshes[m.submesh as usize].clone(), 0, i..i + 1);
            }
        }

        // Resolve.
        let current = hist.current;
        self.aa.taa(
            device,
            queue,
            encoder,
            crate::aa::TaaUniform {
                inv_vp: vp.inverse().to_cols_array(),
                prev_vp: prev_vp.to_cols_array(),
                params: [
                    (1.0 - aa.taa_base_blend_factor).clamp(0.02, 1.0),
                    aa.taa_variance_clamp_scale.max(0.1),
                    valid as u32 as f32,
                    0.0,
                ],
                texel: [1.0 / w as f32, 1.0 / h as f32, w as f32, h as f32],
            },
            &targets.resolve,
            &hist.textures[current],
            motion_view,
            &targets.depth,
            &hist.textures[1 - current],
        );
        hist.current = 1 - current;
        hist.valid = true;
        hist.prev_vp = vp;
        hist.position = cam.position;
        hist.forward = forward;
    }
}

/// The six cube-face cameras of a capture at `position`, tiled 3×2 in the capture atlas
/// (face `i` at column `i % 3`, row `i / 3` from the top).
fn face_cameras(
    position: Vector3,
    near: f32,
    far: f32,
    clear: ClearMode,
    culling_mask: u32,
) -> Vec<CameraView> {
    crate::reflection::face_rotations()
        .into_iter()
        .enumerate()
        .map(|(i, rotation)| {
            let (col, row) = ((i % 3) as f32, (i / 3) as f32);
            CameraView {
                position,
                rotation,
                projection: ProjectionKind::Perspective { fov: 90.0 },
                near: near.max(0.01),
                far: far.max(near + 0.1),
                // Bottom-left origin: row 0 (top) starts halfway up.
                viewport: [col / 3.0, 0.5 - row * 0.5, 1.0 / 3.0, 0.5],
                clear,
                culling_mask,
                occlusion_culling: false,
                antialiasing: Default::default(),
            }
        })
        .collect()
}

struct Cascade {
    view_proj: Matrix4x4,
    split: f32,
    texel: f32,
    light_view: Matrix4x4,
    center: Vector3,
    radius: f32,
}

impl Cascade {
    /// Whether a caster's bounding sphere can throw a shadow into this cascade.
    fn overlaps(&self, c: Vector3, r: f32) -> bool {
        if !r.is_finite() {
            return true;
        }
        let p = self.light_view.transform_point3(c);
        (p.x - self.center.x).abs() <= self.radius + r
            && (p.y - self.center.y).abs() <= self.radius + r
            && p.z - r <= self.center.z + self.radius
    }
}

fn compute_cascades(
    cam: &CameraView,
    aspect: f32,
    sun_dir: Vector3,
    shadow_distance: f32,
    res: u32,
) -> Vec<Cascade> {
    let near = cam.near.max(0.01);
    let far = cam.far.min(shadow_distance).max(near + 1.0);
    let lambda = 0.8;
    let light_view = sun_view(sun_dir);
    let mut out = Vec::new();
    let mut prev = near;
    for i in 0..CASCADES {
        let p = (i + 1) as f32 / CASCADES as f32;
        let log = near * (far / near).powf(p);
        let lin = near + (far - near) * p;
        let split = lambda * log + (1.0 - lambda) * lin;
        // Slice corners in view space.
        let corners: Vec<Vector3> = [prev, split]
            .iter()
            .flat_map(|&d| {
                let (hw, hh) = match cam.projection {
                    ProjectionKind::Perspective { fov } => {
                        let t = (fov.to_radians() * 0.5).tan();
                        (d * t * aspect, d * t)
                    }
                    ProjectionKind::Orthographic { size } => (size * aspect, size),
                };
                [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
                    .map(|(sx, sy)| Vector3::new(sx * hw, sy * hh, d))
            })
            .map(|v| cam.position + cam.rotation * v)
            .collect();
        let center_ws = corners.iter().copied().sum::<Vector3>() / corners.len() as f32;
        let radius = corners
            .iter()
            .map(|c| c.distance(center_ws))
            .fold(0.0, f32::max);
        out.push(make_cascade(light_view, center_ws, radius, split, res));
        prev = split;
    }
    out
}

fn sun_view(sun_dir: Vector3) -> Matrix4x4 {
    let dir = sun_dir.normalize_or(Vector3::NEG_Y);
    let up = if dir.y.abs() > 0.99 {
        Vector3::Z
    } else {
        Vector3::Y
    };
    Matrix4x4::look_to(Vector3::ZERO, dir, up)
}

fn make_cascade(
    light_view: Matrix4x4,
    center_ws: Vector3,
    radius: f32,
    split: f32,
    res: u32,
) -> Cascade {
    let radius = (radius * 16.0).ceil() / 16.0;
    let texel = radius * 2.0 / res as f32;
    let mut c = light_view.transform_point3(center_ws);
    // Snap to whole texels so the shadow doesn't shimmer as the camera moves.
    c.x = (c.x / texel).floor() * texel;
    c.y = (c.y / texel).floor() * texel;
    let back = radius.max(300.0);
    let proj = Matrix4x4::ortho(
        c.x - radius,
        c.x + radius,
        c.y - radius,
        c.y + radius,
        c.z - back,
        c.z + radius,
    );
    Cascade {
        view_proj: proj * light_view,
        split,
        texel,
        light_view,
        center: c,
        radius,
    }
}

/// Cascades for a cube capture: spheres around `center`. A point at view depth `d` in a
/// 90° face is at most `d·√3` away, so each sphere is sized to cover its split that way.
fn cascades_around(
    center: Vector3,
    near: f32,
    sun_dir: Vector3,
    shadow_distance: f32,
    res: u32,
) -> Vec<Cascade> {
    let near = near.max(0.01);
    let far = shadow_distance.max(near + 1.0);
    let light_view = sun_view(sun_dir);
    (0..CASCADES)
        .map(|i| {
            let p = (i + 1) as f32 / CASCADES as f32;
            let split = 0.8 * near * (far / near).powf(p) + 0.2 * (near + (far - near) * p);
            make_cascade(light_view, center, split * 1.75, split, res)
        })
        .collect()
}

fn frustum_planes(m: &Matrix4x4) -> [Vector4; 6] {
    let r0 = m.row(0);
    let r1 = m.row(1);
    let r2 = m.row(2);
    let r3 = m.row(3);
    // Reversed-Z: 0 <= z <= w.
    [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2].map(|p| {
        let l = p.truncate().length();
        if l > 0.0 { p / l } else { p }
    })
}

fn sphere_visible(planes: &[Vector4; 6], c: Vector3, r: f32) -> bool {
    planes.iter().all(|p| p.truncate().dot(c) + p.w >= -r)
}
