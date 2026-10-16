//! Full Screen Pass renderer feature: a user WGSL fragment shader run over the camera image
//! (custom post effects). The user file defines `fn fs_main(in: FullscreenIn) -> vec4f`; the
//! header (bindings, vertex shader, helpers) is prepended. Shaders are validated with naga
//! first so mistakes show in the Console instead of breaking the frame.
//! <https://docs.unity3d.com/Manual/urp/renderer-features/renderer-feature-full-screen-pass.html>

use oxide_assets::{AssetId, AssetServer};
use std::collections::HashMap;

const HEADER: &str = include_str!("shaders/fullscreen_header.wgsl");
/// Used when a shader is missing or invalid, so the image still gets through.
const PASSTHROUGH: &str = "fn fs_main(in: FullscreenIn) -> vec4f { return sample_source(in.uv); }";

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    params0: [f32; 4],
    params1: [f32; 4],
    texel: [f32; 4],
    time: [f32; 4],
    flags: [f32; 4],
}

type PipelineKey = (Option<AssetId>, u64, wgpu::TextureFormat);

pub(crate) struct FullscreenGpu {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    /// `None` for shaders that failed validation (reported once per version).
    pipelines: HashMap<PipelineKey, Option<wgpu::RenderPipeline>>,
    uniforms: Vec<wgpu::Buffer>,
    sampler: wgpu::Sampler,
    no_depth: wgpu::TextureView,
}

/// Parses and validates WGSL; the error text is naga's report.
pub fn validate(source: &str) -> Result<(), String> {
    use wgpu::naga;
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| e.emit_to_string(source))?;
    Ok(())
}

impl FullscreenGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        use wgpu::util::DeviceExt;
        let frag = wgpu::ShaderStages::FRAGMENT;
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
            label: Some("full screen pass layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: frag,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(3, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("full screen pass pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("full screen pass sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let no_depth = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("no depth"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R32Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &0f32.to_le_bytes(),
            )
            .create_view(&Default::default());
        Self {
            layout,
            pipeline_layout,
            pipelines: HashMap::new(),
            uniforms: Vec::new(),
            sampler,
            no_depth,
        }
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        assets: &AssetServer,
        shader: Option<AssetId>,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let version = shader
            .and_then(|id| assets.shaders.version(id))
            .unwrap_or(0);
        let key = (shader, version, format);
        if !self.pipelines.contains_key(&key) {
            let user = shader
                .and_then(|id| assets.shaders.get_by_id(id))
                .map(|s| (s.name.clone(), s.source.clone()));
            let pipeline = match &user {
                None => {
                    if shader.is_some() {
                        oxide_core::debug_log_warning!(
                            "Full Screen Pass: shader not loaded; passing the image through"
                        );
                    }
                    None
                }
                Some((name, src)) => {
                    let full = format!("{HEADER}\n{src}");
                    match validate(&full) {
                        Ok(()) => Some(self.create(device, name, &full, format)),
                        Err(e) => {
                            oxide_core::debug_log_error!("Shader error in {name}:\n{e}");
                            None
                        }
                    }
                }
            };
            self.pipelines.insert(key, pipeline);
        }
        if self.pipelines[&key].is_none() {
            let fallback = (None, 0, format);
            if !self.pipelines.contains_key(&fallback) {
                let full = format!("{HEADER}\n{PASSTHROUGH}");
                let p = self.create(device, "passthrough", &full, format);
                self.pipelines.insert(fallback, Some(p));
            }
            return self.pipelines[&fallback].as_ref().expect("passthrough");
        }
        self.pipelines[&key].as_ref().expect("checked")
    }

    fn create(
        &self,
        device: &wgpu::Device,
        name: &str,
        source: &str,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(name),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("oxide_fullscreen_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("oxide_fullscreen_fs"),
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

    /// Runs `shader` (the `slot`-th full screen pass of this render) from `src` into `dst`.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        assets: &AssetServer,
        slot: usize,
        shader: AssetId,
        params: [[f32; 4]; 2],
        time: f32,
        src: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        dst: &wgpu::TextureView,
        dst_format: wgpu::TextureFormat,
        (w, h): (u32, u32),
    ) {
        while self.uniforms.len() <= slot {
            self.uniforms
                .push(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("full screen pass uniform"),
                    size: std::mem::size_of::<Uniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }
        queue.write_buffer(
            &self.uniforms[slot],
            0,
            bytemuck::bytes_of(&Uniform {
                params0: params[0],
                params1: params[1],
                texel: [1.0 / w as f32, 1.0 / h as f32, w as f32, h as f32],
                time: [time, 0.0, 0.0, 0.0],
                // Only 8-bit display targets without hardware sRGB need encoding; HDR
                // intermediates stay linear.
                flags: [
                    matches!(
                        dst_format,
                        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
                    ) as u32 as f32,
                    0.0,
                    0.0,
                    0.0,
                ],
            }),
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("full screen pass"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms[slot].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(depth.unwrap_or(&self.no_depth)),
                },
            ],
        });
        let pipeline = self.pipeline(device, assets, Some(shader), dst_format);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("full screen pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dst,
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
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn header_plus_user_code_validates_and_errors_are_reported() {
        let ok = format!(
            "{}\nfn fs_main(in: FullscreenIn) -> vec4f {{ return vec4f(1.0) - sample_source(in.uv); }}",
            super::HEADER
        );
        super::validate(&ok).unwrap();
        let bad = format!(
            "{}\nfn fs_main(in: FullscreenIn) -> vec4f {{ return nope; }}",
            super::HEADER
        );
        let err = super::validate(&bad).unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }
}
