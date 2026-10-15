//! Model Context Protocol (MCP) server built into Oxide.
//!
//! Every Oxide process (editor, player, sandbox) can expose its state to AI agents as MCP
//! *tools*. This crate is engine-agnostic: it knows JSON-RPC and MCP, and runs handlers that
//! the engine registers. The engine decides what each tool does (see `oxide_engine::mcp_tools`).
//!
//! Transport: Streamable HTTP on `127.0.0.1` (`POST /mcp`, JSON responses), see [`http`].
//! Clients that only speak stdio use the `oxide-mcp-stdio` bridge binary.
//!
//! Spec: <https://modelcontextprotocol.io/specification>

pub mod http;
mod protocol;

pub use protocol::{Content, Handler, McpServer, SUPPORTED_PROTOCOL_VERSIONS, ToolDef, ToolResult};
pub use serde_json::{Value, json};

/// Default address for the engine's MCP endpoint (override with `OXIDE_MCP_ADDR`).
pub const DEFAULT_ADDR: &str = "127.0.0.1:47474";
