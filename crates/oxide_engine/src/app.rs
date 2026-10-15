//! [`App`]: build a game (register scripts, set up the scene) and run it in a window.
//!
//! ```ignore
//! oxide_engine::App::new("My Racer")
//!     .register::<CarController>("Vehicles/Car Controller")
//!     .setup(|world| { /* build or load the scene */ })
//!     .run();
//! ```

use crate::bus::{Bus, Envelope, Wake};
use crate::engine::{Engine, Plugin, Shell, init_world};
use crate::{input_map, mcp_tools};
use oxide_core::{debug, debug_log, debug_log_error, debug_log_warning};
use oxide_input::Input;
use oxide_mcp::http::HttpServer;
use oxide_render::wgpu;
use oxide_scene::{Component, System, World};
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

/// Requests from game code to the application (Unity `Application.Quit`, scene loading).
pub type AppControl = oxide_scene::Application;

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// Loopback address for the MCP endpoint; `None` disables MCP.
    pub mcp_addr: Option<String>,
    /// Project folder containing `Assets/`.
    pub project_root: PathBuf,
    /// Borderless fullscreen window (a player's Build Settings).
    pub fullscreen: bool,
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
            project_root: std::env::current_dir().unwrap_or_default(),
            fullscreen: false,
        }
    }
}

type SetupFn = Box<dyn FnOnce(&mut World)>;
/// Adds MCP tools to the server (see [`App::mcp_tools`]).
pub type McpToolsFn = fn(&mut oxide_mcp::McpServer, &Arc<Bus>);

pub struct App {
    pub config: EngineConfig,
    world: World,
    systems: Vec<Box<dyn System>>,
    setup: Vec<SetupFn>,
    shell: Option<Box<dyn Shell>>,
    mcp_tools: Vec<McpToolsFn>,
}

impl App {
    pub fn new(title: &str) -> Self {
        Self::with_config(EngineConfig {
            title: title.into(),
            ..Default::default()
        })
    }

    pub fn with_config(config: EngineConfig) -> Self {
        let mut world = World::new();
        init_world(&mut world, config.project_root.clone());
        let mut app = Self {
            config,
            world,
            systems: Vec::new(),
            setup: Vec::new(),
            shell: None,
            mcp_tools: Vec::new(),
        };
        for p in crate::default_plugins() {
            app = app.plugin(p.as_ref());
        }
        app
    }

    /// Makes a component available to Add Component, scene files and MCP.
    pub fn register<T: Component + Default>(mut self, menu: &str) -> Self {
        self.world.registry.register::<T>(menu);
        self
    }

    pub fn plugin(mut self, p: &dyn Plugin) -> Self {
        p.build(&mut self.world, &mut self.systems);
        self
    }

    pub fn system(mut self, s: impl System + 'static) -> Self {
        self.systems.push(Box::new(s));
        self
    }

    /// Scene setup, run once before the first frame (in order of registration).
    pub fn setup(mut self, f: impl FnOnce(&mut World) + 'static) -> Self {
        self.setup.push(Box::new(f));
        self
    }

    /// Runs the game inside a [`Shell`] (the editor).
    pub fn shell(mut self, mut shell: Box<dyn Shell>) -> Self {
        shell.install(&mut self.world);
        self.shell = Some(shell);
        self
    }

    /// Registers extra MCP tools (e.g. the editor's).
    pub fn mcp_tools(mut self, f: McpToolsFn) -> Self {
        self.mcp_tools.push(f);
        self
    }

    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.config.width = width;
        self.config.height = height;
        self
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Opens the window and runs until it is closed or the game quits.
    pub fn run(self) -> Result<(), Box<dyn Error>> {
        run_app(self)
    }

    /// Builds the World with setup applied, without a window (tests, servers).
    pub fn into_world(mut self) -> (World, Vec<Box<dyn System>>) {
        for f in std::mem::take(&mut self.setup) {
            f(&mut self.world);
        }
        (self.world, self.systems)
    }
}

/// What the window needs to start the engine: the World, its systems and the shell.
type Pending = (World, Vec<Box<dyn System>>, Option<Box<dyn Shell>>);

struct Runner {
    config: EngineConfig,
    pending: Option<Pending>,
    /// Last cursor position in window pixels.
    cursor: (f32, f32),
    engine: Option<Engine>,
    jobs: Receiver<Envelope>,
    mcp_url: Option<String>,
    failed: Option<String>,
}

impl Runner {
    fn drain_jobs(&mut self) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        while let Ok((job, reply)) = self.jobs.try_recv() {
            let _ = reply.send(job(engine));
        }
    }
}

impl ApplicationHandler<Wake> for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.engine.is_some() {
            return;
        }
        let Some((world, systems, shell)) = self.pending.take() else {
            return;
        };
        let mut attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(self.config.width, self.config.height));
        if self.config.fullscreen {
            attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }
        let result = event_loop
            .create_window(attrs)
            .map_err(|e| Box::new(e) as Box<dyn Error>)
            .and_then(|window| {
                let instance = wgpu::Instance::new(
                    wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
                        event_loop.owned_display_handle(),
                    )),
                );
                Engine::new(
                    Arc::new(window),
                    instance,
                    self.mcp_url.clone(),
                    world,
                    systems,
                    shell,
                )
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

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        let unfocused = engine.game_viewport.is_some_and(|v| !v.focused);
        if let DeviceEvent::MouseMotion { delta } = event
            && !unfocused
        {
            engine
                .world
                .resource_mut::<Input>()
                .mouse_motion(delta.0 as f32, delta.1 as f32);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        // UI gets events first; ones it uses (clicks on buttons, typing in fields) don't
        // reach the game. Releases always do, so keys can't get stuck.
        let ui_used = engine.gui.on_window_event(&engine.window, &event);
        let release = matches!(
            &event,
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Released
        ) || matches!(
            &event,
            WindowEvent::MouseInput {
                state: ElementState::Released,
                ..
            }
        );
        let locked =
            engine.world.resource::<Input>().cursor_lock != oxide_input::CursorLockMode::None;
        if let WindowEvent::CursorMoved { position, .. } = &event {
            self.cursor = (position.x as f32, position.y as f32);
        }
        let to_game = match engine.game_viewport {
            // Shown in a panel: the game gets the keyboard while its view is focused and the
            // mouse while the cursor is over it.
            Some(vp) => {
                release
                    || locked
                    || match &event {
                        WindowEvent::KeyboardInput { .. } => vp.focused,
                        WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } => {
                            vp.focused && vp.contains(self.cursor.0, self.cursor.1)
                        }
                        _ => true,
                    }
            }
            None => !ui_used || release || locked,
        };
        let viewport = engine.game_viewport;
        let input = engine.world.resource_mut::<Input>();
        match event {
            WindowEvent::KeyboardInput { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
                if !to_game => {}

            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if viewport.is_none() {
                    input.screen_height = size.height as f32;
                }
                engine.surface.resize(&engine.gpu, size.width, size.height);
            }
            WindowEvent::RedrawRequested => engine.frame(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key
                    && let Some(k) = input_map::key(code)
                {
                    input.key_event(k, event.state == ElementState::Pressed);
                }
                if event.state == ElementState::Pressed
                    && let Some(t) = &event.text
                {
                    input.text_event(t);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(b) = input_map::mouse(button) {
                    input.mouse_button_event(b, state == ElementState::Pressed);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (ox, oy) = viewport.map_or((0.0, 0.0), |v| (v.x, v.y));
                input.mouse_move(position.x as f32 - ox, position.y as f32 - oy)
            }
            WindowEvent::MouseWheel { delta, .. } => match delta {
                MouseScrollDelta::LineDelta(x, y) => input.scroll_event(x, y),
                MouseScrollDelta::PixelDelta(p) => {
                    input.scroll_event(p.x as f32 / 40.0, p.y as f32 / 40.0)
                }
            },
            WindowEvent::Focused(false) => input.release_all(),
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

fn run_app(mut app: App) -> Result<(), Box<dyn Error>> {
    debug::install_log_bridge(log::LevelFilter::Warn);
    let event_loop = EventLoop::<Wake>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let (bus, jobs) = Bus::new(event_loop.create_proxy());
    let bus = Arc::new(bus);
    let http = match &app.config.mcp_addr {
        Some(addr) => {
            let mut server = mcp_tools::build_server(bus.clone());
            for f in &app.mcp_tools {
                f(&mut server, &bus);
            }
            match HttpServer::start(Arc::new(server), addr) {
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

    let setup = std::mem::take(&mut app.setup);
    let had_setup = !setup.is_empty();
    for f in setup {
        f(&mut app.world);
    }
    if !had_setup {
        crate::objects::create_default_scene(&mut app.world);
    }
    let mut runner = Runner {
        config: app.config,
        pending: Some((app.world, app.systems, app.shell)),
        cursor: (0.0, 0.0),
        engine: None,
        jobs,
        mcp_url: http.as_ref().map(HttpServer::url),
        failed: None,
    };
    event_loop.run_app(&mut runner)?;
    drop(http);
    match runner.failed {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}
