//! Rendering for Oxide, on top of `wgpu` (Metal / Vulkan / DX12 / WebGPU).
//!
//! - [`gpu`]: device and window surface.
//! - [`frame`]: the plain-data [`RenderFrame`] the engine extracts from the scene.
//! - [`renderer`]: the built-in forward renderer (PBR, cascaded shadows, sky, fog, instancing).
//! - [`post`]: bloom and tonemapping.
//! - [`capture`]: offscreen rendering and readback (screenshots, MCP, tests).

pub mod capture;
pub mod frame;
pub mod gpu;
pub mod gpu_assets;
pub mod post;
pub mod renderer;
pub mod terrain;

pub use capture::{CapturedImage, capture_frame};
pub use frame::*;
pub use gpu::{Gpu, GpuError, WindowSurface};
pub use renderer::SceneRenderer;
pub use wgpu;
