//! Rendering for Oxide, on top of `wgpu` (Metal / Vulkan / DX12 / WebGPU).
//!
//! - [`gpu`]: device and window surface.
//! - [`frame`]: the plain-data [`RenderFrame`] the engine extracts from the scene.
//! - [`renderer`]: the built-in forward renderer (PBR, cascaded shadows, sky, fog, instancing).
//! - [`post`]: bloom and tonemapping.
//! - [`occlusion`]: CPU occlusion culling (occluder depth buffer + min-depth pyramid).
//! - [`capture`]: offscreen rendering and readback (screenshots, MCP, tests).

pub mod aa;
pub mod capture;
pub mod clusters;
pub mod frame;
pub mod fullscreen;
pub mod gpu;
pub mod gpu_assets;
pub mod occlusion;
pub mod particles;
pub mod post;
pub mod postfx;
pub mod reflection;
pub mod render_graph;
pub mod renderer;
pub mod ssao;
pub mod terrain;

pub use capture::{CapturedImage, capture_frame};
pub use frame::*;
pub use gpu::{Gpu, GpuError, WindowSurface};
pub use renderer::SceneRenderer;
pub use wgpu;
