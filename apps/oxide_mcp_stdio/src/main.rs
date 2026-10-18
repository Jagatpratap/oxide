//! `oxide-mcp-stdio` — for MCP clients that can only launch stdio servers.
//!
//! Reads newline-delimited JSON-RPC from stdin, forwards each message to the running engine's
//! HTTP endpoint (`OXIDE_MCP_ADDR`, default `127.0.0.1:47474`) and writes replies to stdout.
//! A project's editor listens on the project's own port, written in the project's `.mcp.json`.
//! If the engine isn't running, requests get a JSON-RPC error explaining how to start it.

use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn post(addr: &str, body: &str) -> io::Result<(u16, String)> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| io::Error::other("malformed HTTP response"))?;
    Ok((status, body.to_owned()))
}

fn main() {
    let addr = std::env::var("OXIDE_MCP_ADDR").unwrap_or_else(|_| oxide_mcp::DEFAULT_ADDR.into());
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match post(&addr, &line) {
            Ok((200, body)) => Some(body),
            Ok((202, _)) => None,
            Ok((status, body)) => {
                error_for(&line, &format!("engine returned HTTP {status}: {body}"))
            }
            Err(e) => error_for(
                &line,
                &format!(
                    "Oxide editor is not reachable at {addr} ({e}). Open the project in the Oxide editor, and set OXIDE_MCP_ADDR to the address in the project's .mcp.json."
                ),
            ),
        };
        if let Some(reply) = reply {
            // Replies must be single-line on stdio.
            let reply = reply.replace('\n', " ");
            if writeln!(stdout, "{reply}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    }
}

/// A JSON-RPC error for each request in `line` (notifications get nothing).
fn error_for(line: &str, message: &str) -> Option<String> {
    eprintln!("oxide-mcp-stdio: {message}");
    let msg: Value = serde_json::from_str(line).ok()?;
    let one = |m: &Value| {
        m.get("id").filter(|_| m.get("method").is_some()).map(|id| {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": message } })
        })
    };
    let out = match &msg {
        Value::Array(batch) => {
            let errs: Vec<Value> = batch.iter().filter_map(one).collect();
            (!errs.is_empty()).then_some(Value::Array(errs))?
        }
        m => one(m)?,
    };
    Some(out.to_string())
}
