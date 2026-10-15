//! MCP Streamable HTTP transport, localhost only.
//!
//! `POST /mcp` with a JSON-RPC message → `200 application/json` with the response, or
//! `202 Accepted` for notifications. No server-initiated SSE stream yet (`GET` → 405).
//!
//! DNS-rebinding protection (required by the MCP spec): the socket binds to loopback only,
//! and requests whose `Host` or `Origin` isn't a loopback name are rejected with 403.
//!
//! Spec: <https://modelcontextprotocol.io/specification/2025-06-18/basic/transports>

use crate::McpServer;
use std::io::{self, Read};
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use tiny_http::{Header, Method, Request, Response, StatusCode};

const MAX_BODY: u64 = 16 * 1024 * 1024;

/// A running HTTP endpoint. Dropping it stops the server.
pub struct HttpServer {
    server: Arc<tiny_http::Server>,
    addr: SocketAddr,
    thread: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Starts serving `mcp` at `http://{addr}/mcp`. `addr` must be a loopback address.
    pub fn start(mcp: Arc<McpServer>, addr: &str) -> io::Result<Self> {
        let requested: SocketAddr = addr
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("{addr}: {e}")))?;
        if !requested.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("refusing to expose MCP on non-loopback address {addr}"),
            ));
        }
        let server = Arc::new(tiny_http::Server::http(requested).map_err(io::Error::other)?);
        let addr = server.server_addr().to_ip().unwrap_or(requested);
        let accept = Arc::clone(&server);
        let thread = thread::Builder::new()
            .name("oxide-mcp-http".into())
            .spawn(move || {
                for request in accept.incoming_requests() {
                    let mcp = Arc::clone(&mcp);
                    // One thread per request: tool calls can block waiting on the engine thread.
                    let _ = thread::Builder::new()
                        .name("oxide-mcp-request".into())
                        .spawn(move || respond(&mcp, request));
                }
            })?;
        Ok(Self {
            server,
            addr,
            thread: Some(thread),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

/// `true` if `host` (optionally with a port) names the local machine.
fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        host.rsplit_once(':').map_or(host, |(h, _)| h)
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

fn is_allowed_origin(origin: &str) -> bool {
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .unwrap_or(origin);
    is_loopback_host(rest.split('/').next().unwrap_or_default())
}

fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json").expect("static header")
}

fn respond(mcp: &McpServer, mut request: Request) {
    let path = request.url().split('?').next().unwrap_or_default();
    let reply = |request: Request, code: u16, body: &str| {
        let _ = request.respond(
            Response::from_string(body)
                .with_status_code(StatusCode(code))
                .with_header(json_header()),
        );
    };

    if path != "/mcp" {
        return reply(
            request,
            404,
            r#"{"error":"not found; the MCP endpoint is /mcp"}"#,
        );
    }
    if !header(&request, "Host").is_some_and(is_loopback_host)
        || header(&request, "Origin").is_some_and(|o| !is_allowed_origin(o))
    {
        return reply(request, 403, r#"{"error":"forbidden origin"}"#);
    }
    match request.method() {
        Method::Post => {}
        Method::Get | Method::Delete => {
            let allow = Header::from_bytes("Allow", "POST").expect("static header");
            let _ = request.respond(Response::empty(405).with_header(allow));
            return;
        }
        _ => return reply(request, 405, r#"{"error":"method not allowed"}"#),
    }

    let mut body = String::new();
    if let Err(e) = request.as_reader().take(MAX_BODY).read_to_string(&mut body) {
        return reply(request, 400, &format!(r#"{{"error":"bad body: {e}"}}"#));
    }
    match mcp.handle_text(&body) {
        Some(response) => reply(request, 200, &response),
        None => {
            let _ = request.respond(Response::empty(202));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;

    fn post(addr: SocketAddr, host: &str, origin: Option<&str>, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(addr).unwrap();
        let origin = origin
            .map(|o| format!("Origin: {o}\r\n"))
            .unwrap_or_default();
        write!(
            s,
            "POST /mcp HTTP/1.1\r\nHost: {host}\r\n{origin}Content-Type: application/json\r\n\
             Accept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let code = out[9..12].parse().unwrap();
        let body = out.split("\r\n\r\n").nth(1).unwrap_or_default().to_owned();
        (code, body)
    }

    #[test]
    fn serves_and_guards() {
        let mcp = Arc::new(McpServer::new("t", "0", ""));
        let http = HttpServer::start(mcp, "127.0.0.1:0").unwrap();
        let addr = http.addr();
        let host = format!("127.0.0.1:{}", addr.port());

        let (code, body) = post(
            addr,
            &host,
            None,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
        );
        assert_eq!(code, 200);
        assert!(body.contains(r#""result":{}"#), "{body}");

        let (code, _) = post(
            addr,
            &host,
            None,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        );
        assert_eq!(code, 202);

        let (code, _) = post(addr, "evil.example:80", None, "{}");
        assert_eq!(code, 403);
        let (code, _) = post(addr, &host, Some("https://evil.example"), "{}");
        assert_eq!(code, 403);
        let (code, _) = post(
            addr,
            &host,
            Some("http://localhost:3000"),
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
        );
        assert_eq!(code, 200);
    }

    #[test]
    fn refuses_public_bind() {
        let mcp = Arc::new(McpServer::new("t", "0", ""));
        assert!(HttpServer::start(mcp, "0.0.0.0:0").is_err());
    }

    #[test]
    fn host_parsing() {
        assert!(is_loopback_host("localhost:47474"));
        assert!(is_loopback_host("[::1]:80"));
        assert!(!is_loopback_host("127.0.0.1.evil.com"));
    }
}
