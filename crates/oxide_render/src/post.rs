//! Post-processing stack: bloom (13-tap downsample / tent upsample chain with URP's
//! scatter) and the uber pass (lens distortion, chromatic aberration, vignette, color
//! grading, tonemapping, film grain) driven by the resolved [`PostSettings`].
//! Reference: <https://docs.unity3d.com/Manual/urp/integration-with-post-processing.html>

use crate::frame::{PostSettings, TonemapMode};
use crate::renderer::HDR_FORMAT;
use std::collections::HashMap;

/// Mirrors `Post` in post.wgsl (32 vec4s).
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniform {
    params: [f32; 4],
    params2: [f32; 4],
    texel: [f32; 4],
    bloom_tint: [f32; 4],
    vignette_color: [f32; 4],
    vignette_params: [f32; 4],
    color: [f32; 4],
    color_filter: [f32; 4],
    white_balance: [f32; 4],
    grain: [f32; 4],
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
    shadows: [f32; 4],
    midtones: [f32; 4],
    highlights: [f32; 4],
    smh_ranges: [f32; 4],
    split_shadows: [f32; 4],
    split_highlights: [f32; 4],
    mixer_r: [f32; 4],
    mixer_g: [f32; 4],
    mixer_b: [f32; 4],
    distortion_amount: [f32; 4],
    distortion_center_scale: [f32; 4],
    /// Screen Space Lens Flare (see `Post.sslf*` in post.wgsl).
    sslf: [[f32; 4]; 5],
    _pad: [[f32; 4]; 3],
}

fn gamma_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(v: [f32; 3]) -> f32 {
    0.212_672_9 * v[0] + 0.715_152_2 * v[1] + 0.072_175 * v[2]
}

/// Unity `ColorUtils.ColorBalanceToLMSCoeffs`.
fn white_balance_coeffs(temperature: f32, tint: f32) -> [f32; 3] {
    let t1 = temperature / 65.0;
    let t2 = tint / 65.0;
    let x = 0.31271 - t1 * if t1 < 0.0 { 0.1 } else { 0.05 };
    let y = 2.87 * x - 3.0 * x * x - 0.275_095_07 + t2 * 0.05;
    let big_x = x / y;
    let big_z = (1.0 - x - y) / y;
    let l = 0.7328 * big_x + 0.4296 - 0.1624 * big_z;
    let m = -0.7036 * big_x + 1.6975 + 0.0061 * big_z;
    let s = 0.0030 * big_x + 0.0136 + 0.9834 * big_z;
    [0.949_237 / l, 1.035_42 / m, 1.087_28 / s]
}

/// Unity `ColorUtils.PrepareLiftGammaGain`.
fn prepare_lgg(lift: [f32; 4], gamma: [f32; 4], gain: [f32; 4]) -> [[f32; 4]; 3] {
    let lin = |v: [f32; 4], k: f32| [0, 1, 2].map(|i| gamma_to_linear(v[i]) * k);
    let l = lin(lift, 0.15);
    let lum = luminance(l);
    let lift = [
        l[0] - lum + lift[3],
        l[1] - lum + lift[3],
        l[2] - lum + lift[3],
        0.0,
    ];
    let g = lin(gamma, 0.8);
    let lum = luminance(g);
    let w = gamma[3] + 1.0;
    let gamma = [0, 1, 2].map(|i| 1.0 / (g[i] - lum + w).max(1e-3));
    let g2 = lin(gain, 0.8);
    let lum = luminance(g2);
    let w = gain[3] + 1.0;
    let gain = [0, 1, 2].map(|i| g2[i] - lum + w);
    [
        lift,
        [gamma[0], gamma[1], gamma[2], 0.0],
        [gain[0], gain[1], gain[2], 0.0],
    ]
}

/// Unity `ColorUtils.PrepareShadowsMidtonesHighlights` for one trackball.
fn prepare_smh(v: [f32; 4]) -> [f32; 4] {
    let offset = v[3] * if v[3] < 0.0 { 1.0 } else { 4.0 };
    let c = [0, 1, 2].map(|i| (gamma_to_linear(v[i]) + offset).max(0.0));
    [c[0], c[1], c[2], 0.0]
}

const SLOT: u64 = 512;
const SLOTS: u64 = 32;

pub struct PostProcessor {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    prefilter: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    sslf: wgpu::RenderPipeline,
    /// Screen Space Lens Flare output per size.
    sslf_targets: HashMap<(u32, u32), wgpu::TextureView>,
    composite: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    dummy: wgpu::TextureView,
}

impl PostProcessor {
    pub fn new(device: &wgpu::Device) -> Self {
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(SLOT),
                    },
                    count: None,
                },
                tex(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(3),
                tex(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let make = |entry: &str, format, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // Upsampling blends the lower mip over the larger one by the scatter (blend
        // constant): URP's lerp(high, low, scatter).
        let scatter = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Constant,
                dst_factor: wgpu::BlendFactor::OneMinusConstant,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let prefilter = make("fs_prefilter", HDR_FORMAT, None);
        let down = make("fs_down", HDR_FORMAT, None);
        let up = make("fs_up", HDR_FORMAT, Some(scatter));
        let sslf = make("fs_sslf", HDR_FORMAT, None);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post uniforms"),
            size: SLOT * SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("post dummy"),
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
            layout,
            pipeline_layout,
            module,
            prefilter,
            down,
            up,
            sslf,
            sslf_targets: HashMap::new(),
            composite: HashMap::new(),
            uniforms,
            sampler,
            dummy,
        }
    }

    /// Bloom mip chain for a `w`×`h` target (half resolution down to 6 levels).
    pub fn bloom_chain(
        device: &wgpu::Device,
        w: u32,
        h: u32,
    ) -> Vec<(wgpu::TextureView, u32, u32)> {
        let mut chain = Vec::new();
        let (mut bw, mut bh) = ((w / 2).max(1), (h / 2).max(1));
        while chain.len() < 6 && bw >= 4 && bh >= 4 {
            let view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("bloom"),
                    size: wgpu::Extent3d {
                        width: bw,
                        height: bh,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            chain.push((view, bw, bh));
            bw /= 2;
            bh /= 2;
        }
        chain
    }

    fn group(
        &self,
        device: &wgpu::Device,
        src: &wgpu::TextureView,
        bloom: &wgpu::TextureView,
        flare: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(SLOT),
                    }),
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
                    resource: wgpu::BindingResource::TextureView(bloom),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(flare),
                },
            ],
        })
    }

    fn pass(
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        blend_constant: Option<f64>,
        pipeline: &wgpu::RenderPipeline,
        group: &wgpu::BindGroup,
        slot: u32,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if blend_constant.is_some() {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        if let Some(k) = blend_constant {
            pass.set_blend_constant(wgpu::Color {
                r: k,
                g: k,
                b: k,
                a: k,
            });
        }
        pass.set_bind_group(0, group, &[slot * SLOT as u32]);
        pass.draw(0..3, 0..1);
    }

    /// The composite pass's parameters for `post`.
    fn composite_uniform(
        post: &PostSettings,
        encode_srgb: bool,
        aspect: f32,
        time: f32,
    ) -> PostUniform {
        let grading = post.enabled;
        let b = &post.bloom;
        let bloom_on = post.enabled && b.intensity > 0.0;
        let v = &post.vignette;
        let row = |r: [f32; 3]| [r[0] / 100.0, r[1] / 100.0, r[2] / 100.0, 0.0];
        let [lift, gamma, gain] = prepare_lgg(post.lift, post.gamma, post.gain);
        let wb = white_balance_coeffs(post.white_balance[0], post.white_balance[1]);
        let ld = &post.lens_distortion;
        let (amount, center_scale) = if post.enabled && ld.intensity != 0.0 {
            let a = 1.6 * (ld.intensity * 100.0).abs().max(1.0);
            let theta = a.min(160.0).to_radians();
            let sigma = 2.0 * (theta * 0.5).tan();
            (
                [
                    if ld.intensity >= 0.0 {
                        theta
                    } else {
                        1.0 / theta
                    },
                    sigma,
                    1.0 / ld.scale.max(0.01),
                    ld.intensity * 100.0,
                ],
                [
                    ld.center[0] * 2.0 - 1.0,
                    ld.center[1] * 2.0 - 1.0,
                    ld.multiplier[0].max(1e-4),
                    ld.multiplier[1].max(1e-4),
                ],
            )
        } else {
            ([0.0; 4], [0.0; 4])
        };
        let on = |c: bool| c as u32 as f32;
        PostUniform {
            params: [
                gamma_to_linear(b.threshold),
                0.5,
                b.intensity,
                post.exposure,
            ],
            params2: [
                0.0,
                on(encode_srgb),
                on(bloom_on),
                match (post.enabled, post.tonemapping) {
                    (false, _) | (_, TonemapMode::None) => 0.0,
                    (_, TonemapMode::Neutral) => 1.0,
                    (_, TonemapMode::Aces) => 2.0,
                },
            ],
            texel: [1.0, 1.0, 0.0, b.clamp],
            bloom_tint: [b.tint[0], b.tint[1], b.tint[2], 0.0],
            vignette_color: [
                v.color[0],
                v.color[1],
                v.color[2],
                if post.enabled { v.intensity * 3.0 } else { 0.0 },
            ],
            vignette_params: [
                v.center[0],
                1.0 - v.center[1],
                v.smoothness * 5.0,
                if v.rounded { aspect } else { 1.0 },
            ],
            color: [
                post.contrast / 100.0 + 1.0,
                post.saturation / 100.0 + 1.0,
                post.hue_shift / 360.0,
                if post.enabled {
                    post.chromatic_aberration
                } else {
                    0.0
                },
            ],
            color_filter: [
                post.color_filter[0],
                post.color_filter[1],
                post.color_filter[2],
                on(grading),
            ],
            white_balance: [wb[0], wb[1], wb[2], on(post.white_balance != [0.0, 0.0])],
            grain: [
                if post.enabled {
                    post.film_grain[0]
                } else {
                    0.0
                },
                post.film_grain[1],
                time,
                0.0,
            ],
            lift,
            gamma,
            gain,
            shadows: prepare_smh(post.shadows),
            midtones: prepare_smh(post.midtones),
            highlights: prepare_smh(post.highlights),
            smh_ranges: post.smh_ranges,
            split_shadows: [
                post.split_shadows[0],
                post.split_shadows[1],
                post.split_shadows[2],
                post.split_balance / 100.0,
            ],
            split_highlights: [
                post.split_highlights[0],
                post.split_highlights[1],
                post.split_highlights[2],
                0.0,
            ],
            mixer_r: row(post.channel_mixer[0]),
            mixer_g: row(post.channel_mixer[1]),
            mixer_b: row(post.channel_mixer[2]),
            distortion_amount: amount,
            distortion_center_scale: center_scale,
            sslf: {
                let f = &post.screen_space_lens_flare;
                let on = post.enabled && f.intensity > 0.0;
                [
                    [
                        if on { f.intensity } else { 0.0 },
                        f.first_flare_intensity,
                        f.secondary_flare_intensity,
                        f.warped_flare_intensity,
                    ],
                    [
                        f.tint[0],
                        f.tint[1],
                        f.tint[2],
                        f.samples.clamp(1, 3) as f32,
                    ],
                    [
                        f.warped_flare_scale[0],
                        f.warped_flare_scale[1],
                        f.sample_dimmer,
                        f.vignette_effect,
                    ],
                    [
                        f.starting_position,
                        f.scale,
                        f.chromatic_aberration,
                        f.streaks_intensity,
                    ],
                    [
                        f.streaks_length,
                        f.streaks_orientation.to_radians(),
                        f.streaks_threshold,
                        aspect,
                    ],
                ]
            },
            _pad: [[0.0; 4]; 3],
        }
    }

    /// Bloom, then the uber pass from `hdr` into `target` (`format`, `aspect` = w / h).
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        chain: &[(wgpu::TextureView, u32, u32)],
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        post: &PostSettings,
        aspect: f32,
        time: f32,
    ) {
        if !self.composite.contains_key(&format) {
            let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("composite"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some("fs_composite"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            self.composite.insert(format, p);
        }
        let composite = Self::composite_uniform(post, !format.is_srgb(), aspect, time);
        let bloom_on = composite.params2[2] > 0.5 && !chain.is_empty();
        let sslf_on = composite.sslf[0][0] > 0.0 && !chain.is_empty();
        let sslf = &post.screen_space_lens_flare;
        let sslf_src = (sslf.bloom_mip as usize).min(chain.len().saturating_sub(1));
        let sslf_size = chain
            .get((sslf.resolution_mip as usize).min(chain.len().saturating_sub(1)))
            .map(|c| (c.1, c.2));
        let mut slots: Vec<PostUniform> = vec![composite];
        let mut u = |texel: (u32, u32)| {
            let mut p = composite;
            p.texel[0] = 1.0 / texel.0 as f32;
            p.texel[1] = 1.0 / texel.1 as f32;
            slots.push(p);
            slots.len() as u32 - 1
        };
        // Slot plan: composite first, then bloom passes (slot, dst, src (None = hdr), up).
        // Screen Space Lens Flare needs the bright mips too, before upsampling changes them.
        let mut plan: Vec<(u32, usize, Option<usize>, bool)> = Vec::new();
        let sslf_slot = sslf_on.then(|| u((chain[sslf_src].1, chain[sslf_src].2)));
        if bloom_on || sslf_on {
            let first = chain[0].clone();
            let s = u((first.1 * 2, first.2 * 2));
            plan.push((s, 0, None, false));
            for i in 1..chain.len() {
                let (_, sw, sh) = &chain[i - 1];
                let s = u((*sw, *sh));
                plan.push((s, i, Some(i - 1), false));
            }
            for i in (0..chain.len() - 1).rev().filter(|_| bloom_on) {
                let (_, sw, sh) = &chain[i + 1];
                let s = u((*sw, *sh));
                plan.push((s, i, Some(i + 1), true));
            }
        }
        if let (true, Some(size)) = (sslf_on, sslf_size)
            && !self.sslf_targets.contains_key(&size)
        {
            let view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("screen space lens flare"),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            if self.sslf_targets.len() > 4 {
                self.sslf_targets.clear();
            }
            self.sslf_targets.insert(size, view);
        }
        queue.write_buffer(&self.uniforms, 0, bytemuck::cast_slice(&slots));
        // URP maps scatter 0..1 to a blend of 0.05..0.95.
        let scatter = 0.05 + 0.9 * post.bloom.scatter.clamp(0.0, 1.0) as f64;
        let mut sslf_done = false;
        let sslf_view = sslf_size
            .filter(|_| sslf_on)
            .and_then(|s| self.sslf_targets.get(&s));
        for (slot, dst, src, up) in plan {
            // Lens flare ghosts from the bright mips, between down- and upsampling.
            if up
                && !sslf_done
                && let (Some(view), Some(s)) = (sslf_view, sslf_slot)
            {
                let g = self.group(device, &chain[sslf_src].0, &self.dummy, &self.dummy);
                Self::pass(encoder, view, None, &self.sslf, &g, s);
                sslf_done = true;
            }
            let src_view = match src {
                None => hdr,
                Some(i) => &chain[i].0,
            };
            let g = self.group(device, src_view, &self.dummy, &self.dummy);
            let pipeline = match (src, up) {
                (None, _) => &self.prefilter,
                (Some(_), false) => &self.down,
                (Some(_), true) => &self.up,
            };
            Self::pass(
                encoder,
                &chain[dst].0,
                up.then_some(scatter),
                pipeline,
                &g,
                slot,
            );
        }
        if !sslf_done && let (Some(view), Some(s)) = (sslf_view, sslf_slot) {
            let g = self.group(device, &chain[sslf_src].0, &self.dummy, &self.dummy);
            Self::pass(encoder, view, None, &self.sslf, &g, s);
        }
        let bloom_view = if bloom_on { &chain[0].0 } else { &self.dummy };
        let g = self.group(device, hdr, bloom_view, sslf_view.unwrap_or(&self.dummy));
        Self::pass(encoder, target, None, &self.composite[&format], &g, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_grading_inputs_are_identity() {
        let n = [1.0, 1.0, 1.0, 0.0];
        let [lift, gamma, gain] = prepare_lgg(n, n, n);
        for i in 0..3 {
            assert!(lift[i].abs() < 1e-5, "lift {lift:?}");
            assert!((gamma[i] - 1.0).abs() < 1e-5, "gamma {gamma:?}");
            assert!((gain[i] - 1.0).abs() < 1e-5, "gain {gain:?}");
        }
        let smh = prepare_smh(n);
        assert!(smh[..3].iter().all(|v| (v - 1.0).abs() < 1e-5));
        let wb = white_balance_coeffs(0.0, 0.0);
        assert!(wb.iter().all(|v| (v - 1.0).abs() < 0.02), "{wb:?}");
        // Warmer: more long-wave (L) than short-wave (S) gain.
        let warm = white_balance_coeffs(50.0, 0.0);
        assert!(warm[0] / warm[2] > wb[0] / wb[2]);
    }
}
