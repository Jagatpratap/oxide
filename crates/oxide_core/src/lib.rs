//! Engine-wide foundations shared by every other Oxide crate.
//!
//! - [`time`]: Unity's `Time` (scaled/unscaled delta, fixed timestep, time scale).
//! - [`debug`]: Unity's `Debug.Log` family feeding an in-memory Console buffer.
//! - [`reflect`]: runtime type info used by the Inspector, serialization and MCP tools.

pub mod debug;
pub mod reflect;
pub mod time;

pub use debug::{LogEntry, LogType};
pub use oxide_derive::Reflect;
pub use time::Time;

/// Engine version reported to tools and the MCP server.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
