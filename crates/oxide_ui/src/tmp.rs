//! TextMeshPro - Text (UI): SDF text on the Canvas. Layout comes from `oxide_text`; the
//! glyphs are drawn by a wgpu paint callback inside egui's pass, so they stay sharp at any
//! size and get TMP's outline and underlay.

use crate::{Anchor, canvas_scale};
use egui_wgpu::wgpu;
use oxide_assets::{AssetServer, Handle};
use oxide_core::Reflect;
use oxide_math::{Color, Vector2};
use oxide_scene::{Component, GameObjectId, World};
use oxide_text::{
    FontAsset, FontLibrary, FontStyles, HorizontalAlignment, LayoutSettings, TextInput,
    TextMaterial, TextMesh, TextOverflowModes, VerticalAlignment,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Unity `TextMeshProUGUI`.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct TextMeshProUGUI {
    pub text: String,
    /// Empty uses the default Font Asset.
    pub font_asset: Option<Handle<FontAsset>>,
    pub font_style: FontStyles,
    /// Reference pixels.
    pub font_size: f32,
    pub auto_size: bool,
    pub font_size_min: f32,
    pub font_size_max: f32,
    pub color: Color,
    pub character_spacing: f32,
    pub word_spacing: f32,
    pub line_spacing: f32,
    pub paragraph_spacing: f32,
    pub horizontal_alignment: HorizontalAlignment,
    pub vertical_alignment: VerticalAlignment,
    pub wrapping: bool,
    pub overflow: TextOverflowModes,
    pub rich_text: bool,
    pub material: TextMaterial,
    pub anchor: Anchor,
    /// Offset from the anchor in reference pixels (+X right, +Y up).
    pub position: Vector2,
    /// Rectangle size (0 = fit the text).
    pub size: Vector2,
}

impl Default for TextMeshProUGUI {
    fn default() -> Self {
        Self {
            text: "New Text".into(),
            font_asset: None,
            font_style: FontStyles::default(),
            font_size: 36.0,
            auto_size: false,
            font_size_min: 18.0,
            font_size_max: 72.0,
            color: Color::WHITE,
            character_spacing: 0.0,
            word_spacing: 0.0,
            line_spacing: 0.0,
            paragraph_spacing: 0.0,
            horizontal_alignment: HorizontalAlignment::Left,
            vertical_alignment: VerticalAlignment::Top,
            wrapping: true,
            overflow: TextOverflowModes::Overflow,
            rich_text: true,
            material: TextMaterial::default(),
            anchor: Anchor::Center,
            position: Vector2::ZERO,
            size: Vector2::new(200.0, 50.0),
        }
    }
}

impl Component for TextMeshProUGUI {}

impl TextMeshProUGUI {
    fn input(&self, k: f32) -> TextInput<'_> {
        TextInput {
            text: &self.text,
            font_asset: self.font_asset.as_ref(),
            rich_text: self.rich_text,
            layout: LayoutSettings {
                font_size: self.font_size * k,
                unit_scale: 1.0,
                width: (self.size.x * k).max(0.0),
                height: (self.size.y * k).max(0.0),
                wrap: self.wrapping,
                overflow: self.overflow,
                horizontal: self.horizontal_alignment,
                vertical: self.vertical_alignment,
                character_spacing: self.character_spacing,
                word_spacing: self.word_spacing,
                line_spacing: self.line_spacing,
                paragraph_spacing: self.paragraph_spacing,
                color: self.color,
                style: self.font_style.to_style(),
            },
            auto_size: self
                .auto_size
                .then_some((self.font_size_min * k, self.font_size_max * k)),
        }
    }
}

/// Layouts of every TextMeshProUGUI, rebuilt when its settings, scale or font change.
#[derive(Default)]
pub struct UiTextCache {
    /// By object and slot (0 = drawn, 1 = measured for layout).
    entries: HashMap<(GameObjectId, u8), (TextMeshProUGUI, f32, u64, TextMesh)>,
}

/// The text's layout for canvas scale `k` (cached).
pub fn layout_ui_text(
    world: &mut World,
    id: GameObjectId,
    t: &TextMeshProUGUI,
    k: f32,
) -> TextMesh {
    layout_ui_text_slot(world, id, 0, t, k)
}

/// The text's unwrapped size in canvas units (its preferred size for auto layout).
pub fn measure(world: &mut World, id: GameObjectId, t: &TextMeshProUGUI) -> Vector2 {
    let mut free = t.clone();
    free.size = Vector2::ZERO;
    let b = layout_ui_text_slot(world, id, 1, &free, 1.0).bounds;
    Vector2::new((b[2] - b[0]).max(0.0), (b[3] - b[1]).max(0.0))
}

fn layout_ui_text_slot(
    world: &mut World,
    id: GameObjectId,
    slot: u8,
    t: &TextMeshProUGUI,
    k: f32,
) -> TextMesh {
    if world.try_resource::<FontLibrary>().is_none() {
        world.resources.insert(FontLibrary::default());
    }
    if world.try_resource::<UiTextCache>().is_none() {
        world.resources.insert(UiTextCache::default());
    }
    let version = {
        let assets = world.resource::<AssetServer>();
        t.font_asset
            .as_ref()
            .and_then(|h| assets.data.version(h.path()))
            .unwrap_or(0)
    };
    if let Some((old, ok, v, mesh)) = world.resource::<UiTextCache>().entries.get(&(id, slot))
        && old == t
        && *ok == k
        && *v == version
    {
        return mesh.clone();
    }
    let mut lib = world.resources.remove::<FontLibrary>().expect("inserted");
    let mesh = oxide_text::build(&mut lib, world.resource_mut::<AssetServer>(), &t.input(k));
    world.resources.insert(lib);
    world
        .resource_mut::<UiTextCache>()
        .entries
        .insert((id, slot), (t.clone(), k, version, mesh.clone()));
    mesh
}

/// One glyph for the UI text shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct UiGlyph {
    /// bl.xy, br.xy and tr.xy, tl.xy in points (relative to the callback rect, y down).
    c01: [f32; 4],
    c23: [f32; 4],
    uv: [f32; 4],
    face: [f32; 4],
    outline: [f32; 4],
    underlay: [f32; 4],
    shape: [f32; 4],
    under: [f32; 4],
    atlas: [f32; 4],
    /// Callback rect size in points (xy).
    rect: [f32; 4],
}

/// An atlas page as the callback sees it.
#[derive(Clone)]
struct PageRef {
    key: String,
    version: u64,
    size: [u32; 2],
    pixels: Arc<Vec<u8>>,
}

/// The target format of egui's pass, stored in the egui renderer's callback resources by
/// the engine so text pipelines match it.
#[derive(Clone, Copy, Debug)]
pub struct UiTargetFormat(pub wgpu::TextureFormat);

struct GpuPage {
    version: u64,
    texture: wgpu::Texture,
    group: wgpu::BindGroup,
}

/// Pipeline and atlas textures kept in egui's callback resources.
struct UiTextGpu {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pages: HashMap<String, GpuPage>,
}

impl UiTextGpu {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui text"),
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
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui_text.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    oxide_render::SDF_WGSL,
                    include_str!("ui_text.wgsl")
                )
                .into(),
            ),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui text"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui text"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<UiGlyph>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
                        4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
                        8 => Float32x4, 9 => Float32x4
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui text"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            pages: HashMap::new(),
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, p: &PageRef) {
        if self
            .pages
            .get(&p.key)
            .is_some_and(|g| g.version == p.version)
        {
            return;
        }
        let size = wgpu::Extent3d {
            width: p.size[0],
            height: p.size[1],
            depth_or_array_layers: 1,
        };
        let texture = match self.pages.remove(&p.key) {
            Some(g) if g.texture.width() == p.size[0] && g.texture.height() == p.size[1] => {
                g.texture
            }
            _ => device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ui text atlas"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &p.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(p.size[0] * 4),
                rows_per_image: Some(p.size[1]),
            },
            size,
        );
        let view = texture.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui text atlas"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.pages.insert(
            p.key.clone(),
            GpuPage {
                version: p.version,
                texture,
                group,
            },
        );
    }
}

/// A prepared instance buffer and its draw ranges per atlas page.
type Prepared = (wgpu::Buffer, Vec<(usize, std::ops::Range<u32>)>);

/// Draws one text's glyphs inside egui's pass.
struct TextCallback {
    pages: Vec<PageRef>,
    /// (page index into `pages`, glyph) in draw order.
    glyphs: Vec<(usize, UiGlyph)>,
    buffer: Mutex<Option<Prepared>>,
}

impl egui_wgpu::CallbackTrait for TextCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<UiTextGpu>().is_none() {
            let Some(format) = resources.get::<UiTargetFormat>().map(|f| f.0) else {
                return Vec::new();
            };
            resources.insert(UiTextGpu::new(device, format));
        }
        let gpu = resources.get_mut::<UiTextGpu>().expect("inserted");
        for p in &self.pages {
            gpu.upload(device, queue, p);
        }
        if self.glyphs.is_empty() {
            return Vec::new();
        }
        let mut data = Vec::with_capacity(self.glyphs.len());
        let mut ranges: Vec<(usize, std::ops::Range<u32>)> = Vec::new();
        for (page, g) in &self.glyphs {
            let at = data.len() as u32;
            data.push(*g);
            match ranges.last_mut() {
                Some((p, r)) if p == page && r.end == at => r.end = at + 1,
                _ => ranges.push((*page, at..at + 1)),
            }
        }
        use wgpu::util::DeviceExt;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui text"),
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::VERTEX,
        });
        *self.buffer.lock().expect("ui text buffer") = Some((buffer, ranges));
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(gpu) = resources.get::<UiTextGpu>() else {
            return;
        };
        let guard = self.buffer.lock().expect("ui text buffer");
        let Some((buffer, ranges)) = guard.as_ref() else {
            return;
        };
        pass.set_pipeline(&gpu.pipeline);
        pass.set_vertex_buffer(0, buffer.slice(..));
        for (page, r) in ranges {
            let Some(g) = gpu.pages.get(&self.pages[*page].key) else {
                continue;
            };
            pass.set_bind_group(0, &g.group, &[]);
            pass.draw(0..6, r.clone());
        }
    }
}

/// Paints a text mesh with its top-left at `origin` (points) inside `clip`.
fn paint_text(
    painter: &egui::Painter,
    lib: &FontLibrary,
    mesh: &TextMesh,
    material: &TextMaterial,
    origin: egui::Pos2,
    clip: egui::Rect,
) {
    let mut pages: Vec<PageRef> = Vec::new();
    let mut page_index: HashMap<(u32, u32), usize> = HashMap::new();
    let srgb = |c: Color| [c.r, c.g, c.b, c.a];
    let m = material;
    let mut faces = Vec::new();
    let mut unders = Vec::new();
    for q in &mesh.quads {
        let Some(atlas) = mesh.fonts.get(q.font as usize).and_then(|k| lib.get(k)) else {
            continue;
        };
        let Some(page) = atlas.pages.get(q.page as usize) else {
            continue;
        };
        let idx = *page_index.entry((q.font, q.page)).or_insert_with(|| {
            pages.push(PageRef {
                key: atlas.texture_path(q.page),
                version: page.version,
                size: [atlas.settings.atlas_width, atlas.settings.atlas_height],
                pixels: page.pixels.clone(),
            });
            pages.len() - 1
        });
        // Text space (+y up from the top) → callback rect points (+y down).
        let p = |c: [f32; 2]| [origin.x - clip.min.x + c[0], origin.y - clip.min.y - c[1]];
        let (bl, br, tr, tl) = (
            p(q.corners[0]),
            p(q.corners[1]),
            p(q.corners[2]),
            p(q.corners[3]),
        );
        let face = srgb(m.face_color);
        let bold = if q.bold {
            atlas.settings.bold_style
        } else {
            0.0
        };
        let g = UiGlyph {
            c01: [bl[0], bl[1], br[0], br[1]],
            c23: [tr[0], tr[1], tl[0], tl[1]],
            uv: q.uv,
            face: [
                face[0] * q.color.r,
                face[1] * q.color.g,
                face[2] * q.color.b,
                face[3] * q.color.a,
            ],
            outline: {
                let o = srgb(m.outline_color);
                [o[0], o[1], o[2], o[3] * q.color.a]
            },
            underlay: {
                let u = srgb(m.underlay_color);
                [u[0], u[1], u[2], u[3] * q.color.a]
            },
            shape: [
                (m.face_dilate + bold).clamp(-1.0, 1.0),
                m.softness,
                m.outline_width,
                atlas.spread(),
            ],
            under: [
                m.underlay_offset.x,
                m.underlay_offset.y,
                m.underlay_dilate,
                m.underlay_softness,
            ],
            atlas: [
                atlas.settings.atlas_width as f32,
                atlas.settings.atlas_height as f32,
                if m.underlay { 1.0 } else { 0.0 },
                if m.underlay { 2.0 } else { 0.0 },
            ],
            rect: [clip.width(), clip.height(), 0.0, 0.0],
        };
        if m.underlay {
            let mut u = g;
            u.atlas[3] = 1.0;
            unders.push((idx, u));
        }
        faces.push((idx, g));
    }
    if faces.is_empty() {
        return;
    }
    unders.extend(faces);
    painter.add(egui_wgpu::Callback::new_paint_callback(
        clip,
        TextCallback {
            pages,
            glyphs: unders,
            buffer: Mutex::new(None),
        },
    ));
}

/// Draws one TextMeshProUGUI (called by the UI system).
pub(crate) fn draw(
    world: &mut World,
    id: GameObjectId,
    ctx: &egui::Context,
    screen_rect: egui::Rect,
    placed: Option<egui::Rect>,
) {
    let Some(mut t) = world.get_component::<TextMeshProUGUI>(id).cloned() else {
        return;
    };
    let screen = screen_rect.size();
    let k = canvas_scale(world, id, screen);
    // A RectTransform tree sets the text box.
    if let Some(r) = placed {
        t.size = Vector2::new(r.width() / k, r.height() / k);
    }
    let mesh = layout_ui_text(world, id, &t, k);
    let Some(lib) = world.try_resource::<FontLibrary>() else {
        return;
    };
    let align = t.anchor.align();
    let (fx, fy) = (align.x().to_factor(), align.y().to_factor());
    let anchor_point = screen_rect.min
        + egui::vec2(screen.x * fx, screen.y * fy)
        + egui::vec2(t.position.x * k, -t.position.y * k);
    let b = mesh.bounds;
    let (w, h, shift) = (
        if t.size.x > 0.0 {
            t.size.x * k
        } else {
            b[2] - b[0]
        },
        if t.size.y > 0.0 {
            t.size.y * k
        } else {
            b[3] - b[1]
        },
        egui::vec2(
            if t.size.x > 0.0 { 0.0 } else { -b[0] },
            if t.size.y > 0.0 { 0.0 } else { b[3] },
        ),
    );
    let top_left = placed.map_or(anchor_point - egui::vec2(w * fx, h * fy), |r| r.min);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::new(("oxide-tmp", id.to_bits())),
    ));
    paint_text(
        &painter.with_clip_rect(screen_rect),
        lib,
        &mesh,
        &t.material,
        top_left + shift,
        screen_rect,
    );
}
