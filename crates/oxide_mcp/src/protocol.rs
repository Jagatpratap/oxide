//! MCP over JSON-RPC 2.0: lifecycle (`initialize`, `ping`) and tools (`tools/list`, `tools/call`).

use base64::Engine as _;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Newest first. The newest is offered when a client asks for a version we don't know.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// A tool's public description, returned by `tools/list`.
#[derive(Clone, Debug)]
pub struct ToolDef {
    /// `[A-Za-z0-9_-]`, unique per server.
    pub name: String,
    pub title: String,
    pub description: String,
    /// JSON Schema (an `"type": "object"` schema) for the arguments.
    pub input_schema: Value,
    /// Tool only reads state.
    pub read_only: bool,
    /// Tool can destroy data or end the session.
    pub destructive: bool,
}

impl ToolDef {
    pub fn new(name: &str, title: &str, description: &str, input_schema: Value) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            description: description.into(),
            input_schema,
            read_only: false,
            destructive: false,
        }
    }

    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": {
                "title": self.title,
                "readOnlyHint": self.read_only,
                "destructiveHint": self.destructive,
                "openWorldHint": false,
            },
        })
    }
}

#[derive(Clone, Debug)]
pub enum Content {
    Text(String),
    Image { data: Vec<u8>, mime_type: String },
}

impl Content {
    fn to_json(&self) -> Value {
        match self {
            Content::Text(text) => json!({ "type": "text", "text": text }),
            Content::Image { data, mime_type } => json!({
                "type": "image",
                "data": base64::engine::general_purpose::STANDARD.encode(data),
                "mimeType": mime_type,
            }),
        }
    }
}

/// What a tool returns. `is_error` marks a failure the model should see and can recover from.
#[derive(Clone, Debug, Default)]
pub struct ToolResult {
    pub content: Vec<Content>,
    pub structured: Option<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(text.into())],
            ..Default::default()
        }
    }

    /// JSON result, sent both as `structuredContent` and as pretty text for older clients.
    pub fn json(value: Value) -> Self {
        let text = serde_json::to_string_pretty(&value).unwrap_or_default();
        let structured = value.is_object().then_some(value);
        Self {
            content: vec![Content::Text(text)],
            structured,
            is_error: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(message.into())],
            structured: None,
            is_error: true,
        }
    }

    pub fn png(data: Vec<u8>) -> Self {
        Self {
            content: vec![Content::Image {
                data,
                mime_type: "image/png".into(),
            }],
            ..Default::default()
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.content.push(Content::Text(text.into()));
        self
    }

    fn to_json(&self) -> Value {
        let mut out = json!({
            "content": self.content.iter().map(Content::to_json).collect::<Vec<_>>(),
            "isError": self.is_error,
        });
        if let Some(s) = &self.structured {
            out["structuredContent"] = s.clone();
        }
        out
    }
}

/// Runs a tool. Called on a server thread; marshal onto the engine thread if needed.
pub type Handler = Arc<dyn Fn(&Map<String, Value>) -> ToolResult + Send + Sync>;

struct Tool {
    def: ToolDef,
    handler: Handler,
}

/// Transport-independent MCP server: feed it JSON-RPC messages, get responses back.
pub struct McpServer {
    name: String,
    version: String,
    instructions: String,
    tools: Vec<Tool>,
    by_name: HashMap<String, usize>,
}

impl McpServer {
    pub fn new(name: &str, version: &str, instructions: &str) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            instructions: instructions.into(),
            tools: Vec::new(),
            by_name: HashMap::new(),
        }
    }

    /// Registers a tool. Panics on a duplicate or invalid name (a programming error).
    pub fn add_tool(
        &mut self,
        def: ToolDef,
        handler: impl Fn(&Map<String, Value>) -> ToolResult + Send + Sync + 'static,
    ) {
        assert!(
            !def.name.is_empty()
                && def.name.len() <= 64
                && def
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "invalid MCP tool name `{}`",
            def.name
        );
        assert!(
            !self.by_name.contains_key(&def.name),
            "duplicate MCP tool `{}`",
            def.name
        );
        self.by_name.insert(def.name.clone(), self.tools.len());
        self.tools.push(Tool {
            def,
            handler: Arc::new(handler),
        });
    }

    pub fn tool_names(&self) -> impl Iterator<Item = &str> {
        self.tools.iter().map(|t| t.def.name.as_str())
    }

    /// Handles raw message text. Returns the response text, or `None` if nothing should be
    /// sent back (notifications and responses).
    pub fn handle_text(&self, body: &str) -> Option<String> {
        let response = match serde_json::from_str::<Value>(body) {
            Ok(message) => self.handle(message)?,
            Err(e) => error_response(Value::Null, PARSE_ERROR, &format!("parse error: {e}")),
        };
        Some(response.to_string())
    }

    /// Handles one message or a batch (array) of messages.
    pub fn handle(&self, message: Value) -> Option<Value> {
        match message {
            Value::Array(batch) if batch.is_empty() => {
                Some(error_response(Value::Null, INVALID_REQUEST, "empty batch"))
            }
            Value::Array(batch) => {
                let out: Vec<Value> = batch
                    .into_iter()
                    .filter_map(|m| self.handle_one(m))
                    .collect();
                (!out.is_empty()).then_some(Value::Array(out))
            }
            m => self.handle_one(m),
        }
    }

    fn handle_one(&self, message: Value) -> Option<Value> {
        let Value::Object(msg) = message else {
            return Some(error_response(
                Value::Null,
                INVALID_REQUEST,
                "message must be an object",
            ));
        };
        let id = msg.get("id").cloned();
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            // A response from the client (we never send requests yet), or garbage.
            return match id {
                Some(id) if !msg.contains_key("result") && !msg.contains_key("error") => {
                    Some(error_response(id, INVALID_REQUEST, "missing method"))
                }
                _ => None,
            };
        };
        // Notifications (no id) never get a response.
        let id = id?;
        let empty = Map::new();
        let params = msg
            .get("params")
            .and_then(Value::as_object)
            .unwrap_or(&empty);

        let result = match method {
            "initialize" => Ok(self.initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": self.tools.iter().map(|t| t.def.to_json()).collect::<Vec<_>>()
            })),
            "tools/call" => self.call_tool(params),
            _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    fn initialize(&self, params: &Map<String, Value>) -> Value {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version = SUPPORTED_PROTOCOL_VERSIONS
            .iter()
            .find(|v| **v == requested)
            .unwrap_or(&SUPPORTED_PROTOCOL_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": self.name, "title": "Oxide Engine", "version": self.version },
            "instructions": self.instructions,
        })
    }

    fn call_tool(&self, params: &Map<String, Value>) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "tools/call needs a `name`".to_owned()))?;
        let &index = self
            .by_name
            .get(name)
            .ok_or((INVALID_PARAMS, format!("unknown tool: {name}")))?;
        let empty = Map::new();
        let args = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(args)) => args,
            Some(_) => return Err((INVALID_PARAMS, "`arguments` must be an object".to_owned())),
        };
        let handler = &self.tools[index].handler;
        let result = catch_unwind(AssertUnwindSafe(|| handler(args))).unwrap_or_else(|panic| {
            let why = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            ToolResult::error(format!("tool `{name}` panicked: {why}"))
        });
        Ok(result.to_json())
    }
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> McpServer {
        let mut s = McpServer::new("oxide", "0.1.0", "test");
        s.add_tool(
            ToolDef::new("echo", "Echo", "Echoes `text`", json!({"type": "object"})).read_only(),
            |args| match args.get("text").and_then(Value::as_str) {
                Some(t) => ToolResult::text(t),
                None => ToolResult::error("missing text"),
            },
        );
        s.add_tool(
            ToolDef::new("boom", "Boom", "Panics", json!({"type": "object"})),
            |_| panic!("kaboom"),
        );
        s
    }

    fn call(s: &McpServer, msg: Value) -> Value {
        s.handle(msg).expect("response")
    }

    #[test]
    fn initialize_negotiates_version() {
        let s = server();
        let r = call(
            &s,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}),
        );
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "oxide");
        let r = call(
            &s,
            json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}),
        );
        assert_eq!(
            r["result"]["protocolVersion"],
            SUPPORTED_PROTOCOL_VERSIONS[0]
        );
    }

    #[test]
    fn notifications_get_no_response() {
        let s = server();
        assert!(
            s.handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
                .is_none()
        );
    }

    #[test]
    fn list_and_call_tools() {
        let s = server();
        let r = call(&s, json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
        assert_eq!(r["result"]["tools"][0]["name"], "echo");
        assert_eq!(r["result"]["tools"][0]["annotations"]["readOnlyHint"], true);

        let r = call(
            &s,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}),
        );
        assert_eq!(r["result"]["content"][0]["text"], "hi");
        assert_eq!(r["result"]["isError"], false);

        let r = call(
            &s,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo"}}),
        );
        assert_eq!(r["result"]["isError"], true);

        let r = call(
            &s,
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope"}}),
        );
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn panicking_tool_is_reported_not_fatal() {
        let s = server();
        let r = call(
            &s,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"boom"}}),
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("kaboom")
        );
    }

    #[test]
    fn errors() {
        let s = server();
        let r = call(
            &s,
            json!({"jsonrpc":"2.0","id":1,"method":"resources/list"}),
        );
        assert_eq!(r["error"]["code"], METHOD_NOT_FOUND);
        let r: Value = serde_json::from_str(&s.handle_text("{not json").unwrap()).unwrap();
        assert_eq!(r["error"]["code"], PARSE_ERROR);
        let r = call(
            &s,
            json!([{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"}]),
        );
        assert_eq!(r.as_array().unwrap().len(), 1);
    }
}
