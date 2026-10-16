//! MCP tools for multiplayer.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_network::{NetworkExt, NetworkObject};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "network",
            "Network session",
            "Multiplayer status: mode, local client id, connected clients, round-trip time and spawned network objects. \
         Optional `start` = host | server | client (with `address` host:port for clients, `port` and `max_clients` for servers) \
         or `stop` = true. Sessions only run in Play mode.",
            schema(
                json!({
                    "start": { "type": "string", "enum": ["host", "server", "client"] },
                    "stop": { "type": "boolean" },
                    "address": { "type": "string" },
                    "port": { "type": "integer", "minimum": 0, "maximum": 65535 },
                    "max_clients": { "type": "integer", "minimum": 1 }
                }),
                &[],
            ),
        ),
        |args, e| {
            let port = args.get("port").and_then(Json::as_u64).unwrap_or(7777) as u16;
            let max = args.get("max_clients").and_then(Json::as_u64).unwrap_or(64) as usize;
            if args.get("stop").and_then(Json::as_bool) == Some(true) {
                e.world.network_mut().shutdown();
            }
            if let Some(mode) = args.get("start").and_then(Json::as_str) {
                if !e.world.run_scripts {
                    return ToolResult::error("enter Play mode first (sessions end when it stops)");
                }
                let net = e.world.network_mut();
                let r = match mode {
                    "host" => net.start_host(port, max),
                    "server" => net.start_server(port, max),
                    _ => {
                        let addr = args
                            .get("address")
                            .and_then(Json::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("127.0.0.1:{port}"));
                        net.start_client(&addr)
                    }
                };
                if let Err(err) = r {
                    return ToolResult::error(err);
                }
            }
            let objects: Vec<Json> = e
                .world
                .query::<NetworkObject>()
                .filter(|(_, n)| n.network_id != 0)
                .map(|(id, n)| {
                    json!({ "network_id": n.network_id, "owner": n.owner, "prefab": n.prefab,
                        "name": e.world.get(id).map(|o| o.name.clone()), "id": id.to_string() })
                })
                .collect();
            let net = e.world.network();
            ToolResult::json(json!({
                "mode": format!("{:?}", net.mode),
                "port": net.port,
                "local_client_id": net.local_client_id,
                "connected": net.is_connected_client(),
                "clients": net.client_count(),
                "rtt_ms": net.rtt_ms(),
                "objects": objects,
            }))
        },
    );
}
