//! Rendering for Oxide, on top of `wgpu` (Metal / Vulkan / DX12 / WebGPU).
//!
//! P0 scope: GPU device + window surface ([`gpu`]), a test renderer ([`triangle`]) and
//! readback of rendered frames to PNG ([`capture`]) for screenshots and MCP tools.
//! The render graph and Scriptable Render Pipeline arrive in P4.

pub mod capture;
pub mod gpu;
pub mod triangle;

pub use capture::{CapturedImage, capture_frame};
pub use gpu::{Gpu, GpuError, WindowSurface};
pub use triangle::TriangleRenderer;
pub use wgpu;
