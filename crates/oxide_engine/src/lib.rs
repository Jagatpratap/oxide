//! The Oxide runtime: opens the window, runs the main loop in Unity's execution order and
//! serves the engine over MCP from the first frame.
//!
//! Frame order (<https://docs.unity3d.com/Manual/execution-order.html>):
//! `FixedUpdate` ×N → `Update` → `LateUpdate` → render → present.

mod app;
mod bus;
pub mod mcp_tools;
mod scene;

pub use app::{EngineConfig, run};
pub use scene::{CameraSettings, TriangleSettings};
