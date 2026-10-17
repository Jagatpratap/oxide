//! egui integration: window input → egui, one egui pass per frame shared by game UI (and
//! later the editor), and drawing the result over the 3D frame.

use oxide_assets::AssetServer;
use oxide_render::{Gpu, SceneRenderer, wgpu};
use oxide_scene::World;
use oxide_ui::Gui;
use winit::window::Window;

/// System fonts with wide script coverage (CJK, Devanagari and other Indic scripts, Arabic,
/// Thai), added after egui's built-in fonts so localized text doesn't show as boxes.
const FALLBACK_FONTS: &[&str] = &[
    // macOS
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    // Windows
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\Nirmala.ttf",
    // Linux
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansDevanagari-Regular.ttf",
];

/// Adds the first system fallback fonts found (egui's own fonts are Latin, Greek and
/// Cyrillic only).
pub fn install_fallback_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut added = 0;
    for (k, path) in FALLBACK_FONTS.iter().enumerate() {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let name = format!("oxide-fallback-{k}");
        fonts.font_data.insert(
            name.clone(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
        added += 1;
    }
    if added > 0 {
        ctx.set_fonts(fonts);
    }
}

pub struct GuiLayer {
    pub state: egui_winit::State,
    pub renderer: egui_wgpu::Renderer,
    paint: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
    free: Vec<egui::TextureId>,
}

impl GuiLayer {
    pub fn new(window: &Window, gpu: &Gpu, format: wgpu::TextureFormat, world: &mut World) -> Self {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        install_fallback_fonts(&ctx);
        let max_side = gpu.device.limits().max_texture_dimension_2d as usize;
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(max_side),
        );
        let mut renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            format.remove_srgb_suffix(),
            egui_wgpu::RendererOptions::default(),
        );
        // Canvas text draws with its own pipeline inside egui's pass.
        renderer
            .callback_resources
            .insert(oxide_ui::UiTargetFormat(format.remove_srgb_suffix()));

        world.resources.insert(Gui::new(ctx));
        Self {
            state,
            renderer,
            paint: Vec::new(),
            pixels_per_point: window.scale_factor() as f32,
            free: Vec::new(),
        }
    }

    /// Feeds a window event to egui. Returns true if egui used it (the game should not).
    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    pub fn begin(&mut self, window: &Window, world: &mut World) {
        let mut raw = self.state.take_egui_input(window);
        let gui = world.resource_mut::<Gui>();
        if let Some(events) = gui.injected_input.pop_front() {
            raw.events.extend(events);
        }
        gui.ctx().begin_pass(raw);
        gui.in_frame = true;
    }

    /// Ends the egui pass, applies platform output and uploads textures.
    pub fn end(
        &mut self,
        window: &Window,
        gpu: &Gpu,
        renderer: &mut SceneRenderer,
        world: &mut World,
    ) {
        let gui = world.resource_mut::<Gui>();
        if !gui.in_frame {
            return;
        }
        gui.in_frame = false;
        let ctx = gui.ctx().clone();
        let mut full = ctx.end_pass();
        gui.wants_pointer = ctx.egui_wants_pointer_input();
        gui.wants_keyboard = ctx.egui_wants_keyboard_input();
        let requests = std::mem::take(&mut gui.texture_requests);
        if let Some(update) = &full.platform_output.accesskit_update {
            world
                .resources
                .insert(oxide_ui::accessibility::AccessKitSnapshot::record(update));
        }
        self.state
            .handle_platform_output(window, full.platform_output);
        self.pixels_per_point = full.pixels_per_point;
        self.paint = ctx.tessellate(full.shapes, full.pixels_per_point);
        for id in self.free.drain(..) {
            self.renderer.free_texture(&id);
        }
        for (id, deltas) in &full.textures_delta.set {
            for delta in deltas {
                self.renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        self.free = full.textures_delta.free.iter().copied().collect();
        full.textures_delta.clear();

        // Register textures that UI components asked for.
        for h in requests {
            if world.resource::<Gui>().textures.contains_key(&h.id()) {
                continue;
            }
            let assets = world.resource::<AssetServer>();
            if let Some(view) =
                renderer
                    .gpu_assets
                    .ui_texture_view(&gpu.device, &gpu.queue, assets, h.id())
            {
                let tid = self.renderer.register_native_texture(
                    &gpu.device,
                    view,
                    wgpu::FilterMode::Linear,
                );
                world.resource_mut::<Gui>().textures.insert(h.id(), tid);
            }
        }
    }

    /// Draws the last egui frame over `target` (`width`×`height` pixels). For captures at a
    /// different size the UI is scaled uniformly. `clear` clears the target first (when the UI
    /// covers the whole window, as in the editor).
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        window_width: u32,
        clear: bool,
    ) {
        if self.paint.is_empty() && !clear {
            return;
        }
        let ppp = self.pixels_per_point * width as f32 / window_width.max(1) as f32;
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: ppp,
        };
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("egui"),
            });
        let extra = self.renderer.update_buffers(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &self.paint,
            &screen,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if clear {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.renderer
                .render(&mut pass.forget_lifetime(), &self.paint, &screen);
        }
        gpu.queue
            .submit(extra.into_iter().chain([encoder.finish()]));
    }
}
