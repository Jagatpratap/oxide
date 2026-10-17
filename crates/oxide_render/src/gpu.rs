//! GPU device and window surface.

use oxide_core::{debug_log, debug_log_error};
use std::fmt;
use std::sync::Arc;

#[derive(Debug)]
pub enum GpuError {
    Surface(wgpu::CreateSurfaceError),
    NoAdapter(wgpu::RequestAdapterError),
    Device(wgpu::RequestDeviceError),
    UnsupportedSurface,
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::Surface(e) => write!(f, "could not create window surface: {e}"),
            GpuError::NoAdapter(e) => write!(f, "no compatible GPU found: {e}"),
            GpuError::Device(e) => write!(f, "could not open GPU device: {e}"),
            GpuError::UnsupportedSurface => write!(f, "GPU cannot present to this window"),
        }
    }
}

impl std::error::Error for GpuError {}

/// Block-compressed texture formats the adapter supports (BC on desktop and Apple GPUs,
/// ASTC and ETC2 on mobile and Apple GPUs).
fn compression_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    adapter.features()
        & (wgpu::Features::TEXTURE_COMPRESSION_BC
            | wgpu::Features::TEXTURE_COMPRESSION_ASTC
            | wgpu::Features::TEXTURE_COMPRESSION_ETC2
            // Scene view wireframe draw modes.
            | wgpu::Features::POLYGON_MODE_LINE
            // The Profiler's GPU pass times.
            | wgpu::Features::TIMESTAMP_QUERY
            // MSAA 2× and 8× (only 1 and 4 samples are guaranteed without it).
            | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
}

/// The GPU: adapter, logical device and submission queue.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Picks a high-performance adapter able to present to `surface` and opens a device.
    pub fn new(instance: wgpu::Instance, surface: &wgpu::Surface<'_>) -> Result<Self, GpuError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(surface),
            ..Default::default()
        }))
        .map_err(GpuError::NoAdapter)?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("oxide device"),
            required_features: compression_features(&adapter),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(GpuError::Device)?;
        device.on_uncaptured_error(Arc::new(|e: wgpu::Error| {
            debug_log_error!("GPU error: {e}")
        }));
        let info = adapter.get_info();
        debug_log!(
            "GPU: {} ({:?}, {:?})",
            info.name,
            info.backend,
            info.device_type
        );
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// A device without a window (tests, offline rendering). `None` when no GPU is present.
    pub fn headless() -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("oxide headless device"),
            required_features: compression_features(&adapter),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .ok()?;
        Some(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }
}

/// A window's swapchain.
pub struct WindowSurface {
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
}

impl WindowSurface {
    /// Configures `surface` with an sRGB format (so shaders output linear color) and vsync.
    pub fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuError> {
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or(GpuError::UnsupportedSurface)?;
        let mut config = surface
            .get_default_config(&gpu.adapter, width.max(1), height.max(1))
            .ok_or(GpuError::UnsupportedSurface)?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // A linear view of the same texture for UI drawn in gamma space (egui).
        config.view_formats = vec![format.remove_srgb_suffix()];

        surface.configure(&gpu.device, &config);
        Ok(Self { surface, config })
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return; // minimized
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn reconfigure(&self, gpu: &Gpu) {
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn set_vsync(&mut self, gpu: &Gpu, vsync: bool) {
        self.config.present_mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        self.reconfigure(gpu);
    }
}
