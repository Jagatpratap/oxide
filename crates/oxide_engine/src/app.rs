//! Window, main loop and engine state.

use crate::bus::{Bus, Envelope, Wake};
use crate::mcp_tools;
use crate::scene::{CameraSettings, TriangleSettings};
use oxide_core::reflect::TypeRegistry;
use oxide_core::{Time, debug, debug_log, debug_log_error, debug_log_warning};
use oxide_mcp::http::HttpServer;
use oxide_render::{CapturedImage, Gpu, TriangleRenderer, WindowSurface, capture_frame, wgpu};
use std::error::Error;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// Loopback address for the MCP endpoint; `None` disables MCP.
    pub mcp_addr: Option<String>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            title: "Oxide".into(),
            width: 1280,
            height: 720,
            mcp_addr: Some(
                std::env::var("OXIDE_MCP_ADDR").unwrap_or_else(|_| oxide_mcp::DEFAULT_ADDR.into()),
            ),
        }
    }
}

/// Everything that lives on the main thread. MCP tools get `&mut Engine` via the bus.
pub struct Engine {
    pub window: Arc<Window>,
    pub gpu: Gpu,
    pub surface: WindowSurface,
    pub triangle_renderer: TriangleRenderer,
    pub time: Time,
    pub types: TypeRegistry,
    pub camera: CameraSettings,
    pub triangle: TriangleSettings,
    pub paused: bool,
    /// Frames still to run while paused (Unity's "Step" button).
    pub pending_steps: u32,
    pub update_count: u64,
    pub late_update_count: u64,
    /// Smoothed frames per second (wall clock).
    pub fps: f32,
    pub mcp_url: Option<String>,
    pub quit_requested: bool,
    last_frame: Instant,
}

impl Engine {
    fn new(
        window: Arc<Window>,
        instance: wgpu::Instance,
        mcp_url: Option<String>,
    ) -> Result<Self, Box<dyn Error>> {
        let surface = instance.create_surface(Arc::clone(&window))?;
        let gpu = Gpu::new(instance, &surface)?;
        let size = window.inner_size();
        let surface = WindowSurface::new(&gpu, surface, size.width, size.height)?;
        let triangle_renderer = TriangleRenderer::new(&gpu.device, surface.format());
        let mut types = TypeRegistry::new();
        types.register::<CameraSettings>();
        types.register::<TriangleSettings>();
        Ok(Self {
            window,
            triangle_renderer,
            surface,
            gpu,
            time: Time::new(),
            types,
            camera: CameraSettings::default(),
            triangle: TriangleSettings::default(),
            paused: false,
            pending_steps: 0,
            update_count: 0,
            late_update_count: 0,
            fps: 0.0,
            mcp_url,
            quit_requested: false,
            last_frame: Instant::now(),
        })
    }

    /// One iteration of the player loop.
    fn frame(&mut self) {
        let now = Instant::now();
        let real_dt = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        if real_dt > 0.0 {
            let instant_fps = (1.0 / real_dt) as f32;
            self.fps = if self.fps == 0.0 {
                instant_fps
            } else {
                self.fps * 0.95 + instant_fps * 0.05
            };
        }

        let step = self.paused && self.pending_steps > 0;
        if !self.paused || step {
            if step {
                self.pending_steps -= 1;
            }
            // A paused step advances by one fixed-length frame, independent of wall time.
            let dt = if step { 1.0 / 60.0 } else { real_dt };
            self.time.begin_frame(dt);
            while self.time.next_fixed_step() {
                self.fixed_update();
            }
            self.update();
            self.late_update();
        }
        self.render();
    }

    fn fixed_update(&mut self) {
        // Physics and fixed-rate scripts run here from P6/P7 on.
    }

    fn update(&mut self) {
        self.update_count += 1;
        if self.triangle.active {
            let z = self.triangle.rotation.z + self.triangle.spin_speed * self.time.delta_time();
            self.triangle.rotation.z = z.rem_euclid(360.0);
        }
    }

    fn late_update(&mut self) {
        self.late_update_count += 1;
    }

    fn mvp(&self, width: u32, height: u32) -> oxide_math::Matrix4x4 {
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        let model = if self.triangle.active {
            self.triangle.local_to_world()
        } else {
            // Inactive objects aren't drawn; collapse the triangle to nothing.
            oxide_math::Matrix4x4::ZERO
        };
        self.camera.view_projection(aspect) * model
    }

    fn render(&mut self) {
        let frame = match self.surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.reconfigure(&self.gpu);
                frame
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.reconfigure(&self.gpu);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                debug_log_error!("surface validation error; skipping frame");
                return;
            }
        };
        let (w, h) = self.surface.size();
        self.triangle_renderer
            .set_mvp(&self.gpu.queue, self.mvp(w, h));
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.triangle_renderer
            .draw(&mut encoder, &view, self.camera.background_color.linear());
        self.gpu.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
    }

    /// Renders the current scene offscreen at `width`×`height` and reads it back.
    pub fn capture(&self, width: u32, height: u32) -> Result<CapturedImage, String> {
        self.triangle_renderer
            .set_mvp(&self.gpu.queue, self.mvp(width, height));
        let clear = self.camera.background_color.linear();
        let result = capture_frame(
            &self.gpu,
            self.surface.format(),
            width,
            height,
            |encoder, view| {
                self.triangle_renderer.draw(encoder, view, clear);
            },
        );
        // Restore the window-sized matrix for the next on-screen frame.
        let (w, h) = self.surface.size();
        self.triangle_renderer
            .set_mvp(&self.gpu.queue, self.mvp(w, h));
        result
    }
}

struct App {
    config: EngineConfig,
    engine: Option<Engine>,
    jobs: Receiver<Envelope>,
    mcp_url: Option<String>,
    failed: Option<String>,
}

impl App {
    fn drain_jobs(&mut self) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        while let Ok((job, reply)) = self.jobs.try_recv() {
            let _ = reply.send(job(engine));
        }
    }
}

impl ApplicationHandler<Wake> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.engine.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(self.config.width, self.config.height));
        let result = event_loop
            .create_window(attrs)
            .map_err(|e| Box::new(e) as Box<dyn Error>)
            .and_then(|window| {
                let instance = wgpu::Instance::new(
                    wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
                        event_loop.owned_display_handle(),
                    )),
                );
                Engine::new(Arc::new(window), instance, self.mcp_url.clone())
            });
        match result {
            Ok(engine) => {
                debug_log!("Oxide {} running", oxide_core::ENGINE_VERSION);
                engine.window.request_redraw();
                self.engine = Some(engine);
            }
            Err(e) => {
                debug_log_error!("failed to start: {e}");
                self.failed = Some(e.to_string());
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, _: Wake) {
        self.drain_jobs();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                engine.surface.resize(&engine.gpu, size.width, size.height)
            }
            WindowEvent::RedrawRequested => engine.frame(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.drain_jobs();
        if let Some(engine) = &self.engine {
            if engine.quit_requested {
                event_loop.exit();
                return;
            }
            engine.window.request_redraw();
        }
    }
}

/// Opens the window and runs until it is closed (or an MCP client calls `quit`).
pub fn run(config: EngineConfig) -> Result<(), Box<dyn Error>> {
    debug::install_log_bridge(log::LevelFilter::Warn);
    let event_loop = EventLoop::<Wake>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let (bus, jobs) = Bus::new(event_loop.create_proxy());
    let http = match &config.mcp_addr {
        Some(addr) => {
            match HttpServer::start(Arc::new(mcp_tools::build_server(Arc::new(bus))), addr) {
                Ok(http) => {
                    debug_log!("MCP server listening on {}", http.url());
                    Some(http)
                }
                Err(e) => {
                    debug_log_warning!("MCP server disabled: could not listen on {addr}: {e}");
                    None
                }
            }
        }
        None => None,
    };

    let mut app = App {
        config,
        engine: None,
        jobs,
        mcp_url: http.as_ref().map(HttpServer::url),
        failed: None,
    };
    event_loop.run_app(&mut app)?;
    drop(http);
    match app.failed {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}
