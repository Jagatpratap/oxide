//! The running engine: GPU, window surface, the World and its systems, and one frame of
//! Unity's player loop.

use crate::extract;
use crate::input_map::Gamepads;
use oxide_assets::AssetServer;
use oxide_core::{Time, debug_log_error};
use oxide_input::{CursorLockMode, Input};
use oxide_render::{
    CameraView, CapturedImage, Gpu, RenderStats, SceneRenderer, WindowSurface, capture_frame, wgpu,
};
use oxide_scene::{DebugDraw, Phase, RenderSettings, System, Timers, World};
use std::sync::Arc;
use std::time::Instant;
use winit::window::Window;

/// Registers components/resources/systems on a fresh World (physics, audio, game code...).
pub trait Plugin {
    fn build(&self, world: &mut World, systems: &mut Vec<Box<dyn System>>);
}

/// Takes over the window: draws its own UI around the game and decides when the game runs.
/// The editor is a `Shell`; a shipped game runs without one.
pub trait Shell: std::any::Any {
    /// Called once on the App's World, before scene setup runs.
    fn install(&mut self, world: &mut World) {
        let _ = world;
    }
    /// Builds this frame's UI (inside the egui pass, after the game's frame ran).
    fn ui(&mut self, engine: &mut Engine);
    /// Renders the 3D views the UI shows, before the UI itself is drawn over the window.
    fn render(&mut self, engine: &mut Engine);
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// Where the game's screen is while a [`Shell`] shows it in a panel (physical pixels).
/// Mouse input is made relative to it and keyboard input only reaches the game when
/// `focused`.
#[derive(Clone, Copy, Debug, Default)]
pub struct GameViewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub focused: bool,
    /// Game pixels per window pixel (a fixed Game view resolution shown scaled); 0 means 1.
    pub scale: f32,
}

impl GameViewport {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

/// Installs everything a World needs before scripts run: resources and built-in components.
pub fn init_world(world: &mut World, project_root: std::path::PathBuf) {
    oxide_scene::register_builtin(&mut world.registry);
    world.resources.insert(Time::new());
    world.resources.insert(Input::new());
    let mut assets = AssetServer::new(project_root);
    crate::builtin_assets::register(&mut assets);
    world.resources.insert(assets);
    world.resources.insert(Timers::default());
    world.resources.insert(DebugDraw::default());
    world.resources.insert(RenderSettings::default());
    world.resources.insert(crate::AppControl::default());
}

/// Background-loaded scenes activated per frame at most (spreads streaming hitches).
const MAX_SCENE_ACTIVATIONS: usize = 1;

pub struct Engine {
    pub gui: crate::gui::GuiLayer,
    pub window: Arc<Window>,
    pub gpu: Gpu,
    pub surface: WindowSurface,
    pub renderer: SceneRenderer,
    pub world: World,
    pub systems: Vec<Box<dyn System>>,
    pub paused: bool,
    /// Frames still to run while paused (the editor's Step button).
    pub pending_steps: u32,
    pub fps: f32,
    pub mcp_url: Option<String>,
    pub quit_requested: bool,
    pub stats: RenderStats,
    pub(crate) gamepads: Gamepads,
    /// The editor (or another shell) around the game, if any.
    pub shell: Option<Box<dyn Shell>>,
    /// Whether the player loop runs (false in the editor's Edit mode).
    pub simulate: bool,
    /// Set by a shell that shows the game in a panel.
    pub game_viewport: Option<GameViewport>,
    cursor: (CursorLockMode, bool),
    last_frame: Instant,
    /// Vsync the surface is configured with (follows the active quality level).
    vsync: bool,
    /// Reimport asset files edited outside the engine (Preferences → Auto Refresh).
    pub auto_refresh: bool,
}

impl Engine {
    pub(crate) fn new(
        window: Arc<Window>,
        instance: wgpu::Instance,
        mcp_url: Option<String>,
        world: World,
        systems: Vec<Box<dyn System>>,
        shell: Option<Box<dyn Shell>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let surface = instance.create_surface(Arc::clone(&window))?;
        let gpu = Gpu::new(instance, &surface)?;
        let size = window.inner_size();
        let surface = WindowSurface::new(&gpu, surface, size.width, size.height)?;
        let renderer = SceneRenderer::new(&gpu);
        let mut world = world;
        let gui = crate::gui::GuiLayer::new(&window, &gpu, surface.format(), &mut world);
        let mut e = Self {
            gui,
            window,
            gpu,
            surface,
            renderer,
            world,
            systems,
            paused: false,
            pending_steps: 0,
            fps: 0.0,
            mcp_url,
            quit_requested: false,
            stats: RenderStats::default(),
            gamepads: Gamepads::new(),
            simulate: shell.is_none(),
            shell,
            game_viewport: None,
            cursor: (CursorLockMode::None, true),
            last_frame: Instant::now(),
            vsync: true,
            auto_refresh: true,
        };
        e.world.resource_mut::<Input>().screen_height = size.height as f32;
        e.start_systems();
        Ok(e)
    }

    pub fn start_systems(&mut self) {
        for s in &mut self.systems {
            s.start(&mut self.world);
        }
    }

    pub fn time(&self) -> &Time {
        self.world.resource::<Time>()
    }

    /// One iteration of the player loop, then render.
    pub(crate) fn frame(&mut self) {
        let now = Instant::now();
        let real_dt = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        if real_dt > 0.0 {
            let inst = (1.0 / real_dt) as f32;
            self.fps = if self.fps == 0.0 {
                inst
            } else {
                self.fps * 0.95 + inst * 0.05
            };
        }
        self.gamepads.poll(self.world.resource_mut::<Input>());

        // Debug lines live until the next frame starts (so screenshots taken between
        // frames still see them), or until their `until` time.
        let now = self.world.resource::<Time>().time();
        if let Some(d) = self.world.try_resource_mut::<DebugDraw>() {
            d.lines.retain(|l| l.until > now);
        }
        self.gui.begin(&self.window, &mut self.world);
        let step = self.paused && self.pending_steps > 0;
        if self.simulate && (!self.paused || step) {
            if step {
                self.pending_steps -= 1;
            }
            let dt = if step { 1.0 / 60.0 } else { real_dt };
            self.tick(dt);
        }
        for s in &mut self.systems {
            s.always(&mut self.world);
        }
        if let Some(mut shell) = self.shell.take() {
            shell.ui(self);
            self.shell = Some(shell);
        }
        self.gui
            .end(&self.window, &self.gpu, &mut self.renderer, &mut self.world);
        self.render();
        self.end_frame();
    }

    /// Advances the simulation by `dt` seconds of real time (one frame).
    pub fn tick(&mut self, dt: f64) {
        let w = &mut self.world;
        w.resource_mut::<Time>().begin_frame(dt);
        let unscaled = w.resource::<Time>().unscaled_delta_time();
        w.resource_mut::<Input>().update_axes(unscaled);
        crate::builtin_assets::update_input_actions(w, unscaled);
        w.run_start();
        while w.resource_mut::<Time>().next_fixed_step() {
            w.run_phase(Phase::FixedUpdate);
            for s in &mut self.systems {
                s.fixed_update(w);
            }
        }
        w.run_phase(Phase::Update);
        let t = w.resource::<Time>().time();
        Timers::run(w, t);
        for s in &mut self.systems {
            s.update(w);
        }
        w.run_phase(Phase::LateUpdate);
        for s in &mut self.systems {
            s.late_update(w);
        }
    }

    fn end_frame(&mut self) {
        if let Some(result) = crate::lighting::poll(&mut self.world) {
            match result {
                Ok(msg) => oxide_core::debug_log!("{msg}"),
                Err(e) => oxide_core::debug_log_error!("Lighting bake failed: {e}"),
            }
        }
        // Pick up asset files edited outside the engine (twice a second).
        if self.auto_refresh
            && self
                .world
                .resource::<Time>()
                .frame_count()
                .is_multiple_of(30)
        {
            let reloaded = self.world.resource_mut::<AssetServer>().reload_changed();
            for p in reloaded {
                oxide_core::debug_log!("Reloaded {p}");
            }
        }
        let vsync = oxide_scene::QualitySettings::of(&self.world).vsync;
        if vsync != self.vsync {
            self.vsync = vsync;
            self.surface.set_vsync(&self.gpu, vsync);
        }
        let input = self.world.resource_mut::<Input>();
        input.end_frame();
        let want = (input.cursor_lock, input.cursor_visible);
        if want != self.cursor {
            self.cursor = want;
            use winit::window::CursorGrabMode;
            let grab = match want.0 {
                CursorLockMode::None => CursorGrabMode::None,
                CursorLockMode::Locked => CursorGrabMode::Locked,
                CursorLockMode::Confined => CursorGrabMode::Confined,
            };
            if self.window.set_cursor_grab(grab).is_err() && grab == CursorGrabMode::Locked {
                let _ = self.window.set_cursor_grab(CursorGrabMode::Confined);
            }
            self.window.set_cursor_visible(want.1);
        }
        // Application.Quit ends a player; in the editor (a shell) it is ignored, like Unity.
        if std::mem::take(&mut self.world.resource_mut::<crate::AppControl>().quit)
            && self.shell.is_none()
        {
            self.quit_requested = true;
        }
        if let Some(scene) = self
            .world
            .resource_mut::<crate::AppControl>()
            .pending_scene
            .take()
        {
            self.load_requested_scene(&scene);
        }
        self.pump_scenes();
    }

    /// Activates finished background scene loads (LoadSceneAsync, streaming) and runs
    /// pending unloads. Called at the end of every frame.
    pub fn pump_scenes(&mut self) {
        let run = self.world.run_scripts;
        if self
            .world
            .pump_scene_operations(MAX_SCENE_ACTIVATIONS)
            .replaced
        {
            self.world.run_scripts = run;
            self.start_systems();
        }
    }

    /// Applies a `SceneManager.LoadScene` request from game code.
    fn load_requested_scene(&mut self, scene: &str) {
        let all: Vec<String> = self
            .world
            .resource::<AssetServer>()
            .list_files()
            .into_iter()
            .filter(|f| f.ends_with(".scene"))
            .collect();
        let Some(path) = self
            .world
            .resource::<crate::AppControl>()
            .resolve_scene(scene, &all)
        else {
            debug_log_error!("LoadScene: no scene `{scene}` (add it to the build's scenes)");
            return;
        };
        let run = self.world.run_scripts;
        match crate::load_scene(&mut self.world, &path, false) {
            Ok(_) => {
                self.world.run_scripts = run;
                self.start_systems();
            }
            Err(e) => debug_log_error!("LoadScene: {e}"),
        }
    }

    fn render(&mut self) {
        let frame = match self.surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                self.surface.reconfigure(&self.gpu);
                f
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                // Hidden/minimized windows don't present, so nothing paces the loop.
                std::thread::sleep(std::time::Duration::from_millis(16));
                return;
            }

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
        let ui_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.surface.format().remove_srgb_suffix()),
            ..Default::default()
        });
        if let Some(mut shell) = self.shell.take() {
            shell.render(self);
            self.shell = Some(shell);
            self.gui.draw(&self.gpu, &ui_view, w, h, w, true);
        } else {
            let rf = extract::extract(&self.world, None);
            let view = frame.texture.create_view(&Default::default());
            let assets = self.world.resource::<AssetServer>();
            self.stats =
                self.renderer
                    .render(&self.gpu, assets, &rf, &view, self.surface.format(), w, h);
            self.gui.draw(&self.gpu, &ui_view, w, h, w, false);
        }
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
    }

    /// Renders the whole window (a shell's UI and its views) offscreen.
    pub fn capture_window(&mut self) -> Result<CapturedImage, String> {
        if let Some(mut shell) = self.shell.take() {
            shell.render(self);
            self.shell = Some(shell);
        }
        let (w, h) = self.surface.size();
        let format = self.surface.format();
        let has_shell = self.shell.is_some();
        let world = &self.world;
        let renderer = &mut self.renderer;
        let gpu = &self.gpu;
        let gui = &mut self.gui;
        capture_frame(gpu, format, w, h, |view, texture| {
            if !has_shell {
                let rf = extract::extract(world, None);
                let assets = world.resource::<AssetServer>();
                renderer.render(gpu, assets, &rf, view, format, w, h);
            }
            let ui_view = texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(format.remove_srgb_suffix()),
                ..Default::default()
            });
            gui.draw(gpu, &ui_view, w, h, w, has_shell);
        })
    }

    /// The installed shell, if it is a `T`.
    pub fn shell_mut<T: Shell>(&mut self) -> Option<&mut T> {
        self.shell.as_mut()?.as_any_mut().downcast_mut::<T>()
    }

    /// Renders offscreen at `width`×`height` (scene cameras, or `cameras` if given).
    pub fn capture(
        &mut self,
        width: u32,
        height: u32,
        cameras: Option<Vec<CameraView>>,
    ) -> Result<CapturedImage, String> {
        let rf = extract::extract(&self.world, cameras);
        let format = self.surface.format();
        let assets = self.world.resource::<AssetServer>();
        let renderer = &mut self.renderer;
        let gpu = &self.gpu;
        let gui = &mut self.gui;
        let window_width = self.surface.size().0;
        // With a shell the UI is the editor's, not the game's, so it is left out.
        let game_ui = self.shell.is_none();
        capture_frame(gpu, format, width, height, |view, texture| {
            renderer.render(gpu, assets, &rf, view, format, width, height);
            if game_ui {
                let ui_view = texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(format.remove_srgb_suffix()),
                    ..Default::default()
                });
                gui.draw(gpu, &ui_view, width, height, window_width, false);
            }
        })
    }
}
