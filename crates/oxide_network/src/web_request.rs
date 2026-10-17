//! Unity's `UnityWebRequest`: HTTP(S) requests that run in the background while the game
//! keeps going — GET/POST/PUT/DELETE/HEAD, form and JSON bodies, headers, timeouts — with the
//! result read as text, bytes, a texture or a file. Poll [`WebRequestOperation::is_done`]
//! each frame, or `.await` it in a coroutine:
//!
//! ```ignore
//! ctx.start_coroutine(|co| async move {
//!     let op = UnityWebRequest::get("https://example.com/scores.json").send_web_request();
//!     op.wait().await;                       // yield return request.SendWebRequest()
//!     if op.result() == WebRequestResult::Success {
//!         let text = op.text();
//!         co.with(|ctx| { /* use it */ });
//!     }
//! });
//! ```
//! <https://docs.unity3d.com/Manual/UnityWebRequest.html>

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

/// Unity `UnityWebRequest.Result`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebRequestResult {
    InProgress,
    Success,
    /// No connection: DNS, refused, TLS, timeout.
    ConnectionError,
    /// The server answered with an error status (4xx / 5xx).
    ProtocolError,
    /// The response couldn't be read or decoded.
    DataProcessingError,
}

/// A request to build and send (Unity `UnityWebRequest`).
#[derive(Clone, Debug)]
pub struct UnityWebRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Seconds; 0 = no limit (Unity `timeout`).
    pub timeout: u32,
    /// Write the response to this file instead of memory (Unity `DownloadHandlerFile`).
    pub download_to: Option<std::path::PathBuf>,
}

impl UnityWebRequest {
    fn new(method: &str, url: &str) -> Self {
        Self {
            url: url.to_owned(),
            method: method.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout: 0,
            download_to: None,
        }
    }

    /// Unity `UnityWebRequest.Get`.
    pub fn get(url: &str) -> Self {
        Self::new("GET", url)
    }

    /// Unity `UnityWebRequest.Post(url, postData, contentType)`.
    pub fn post(url: &str, body: impl Into<Vec<u8>>, content_type: &str) -> Self {
        let mut r = Self::new("POST", url);
        r.body = body.into();
        r.set_request_header("Content-Type", content_type);
        r
    }

    /// Unity `UnityWebRequest.Post(url, form)`: `application/x-www-form-urlencoded` fields.
    pub fn post_form(url: &str, fields: &[(&str, &str)]) -> Self {
        let body = fields
            .iter()
            .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        Self::post(url, body, "application/x-www-form-urlencoded")
    }

    /// A JSON POST (Unity `UnityWebRequest.Post(url, json, "application/json")`).
    pub fn post_json(url: &str, json: &serde_json::Value) -> Self {
        Self::post(url, json.to_string(), "application/json")
    }

    /// Unity `UnityWebRequest.Put`.
    pub fn put(url: &str, body: impl Into<Vec<u8>>) -> Self {
        let mut r = Self::new("PUT", url);
        r.body = body.into();
        r
    }

    /// Unity `UnityWebRequest.Delete`.
    pub fn delete(url: &str) -> Self {
        Self::new("DELETE", url)
    }

    /// Unity `UnityWebRequest.Head`.
    pub fn head(url: &str) -> Self {
        Self::new("HEAD", url)
    }

    /// Unity `SetRequestHeader`.
    pub fn set_request_header(&mut self, name: &str, value: &str) -> &mut Self {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Unity `SendWebRequest()`: starts it in the background.
    pub fn send_web_request(self) -> WebRequestOperation {
        let state = Arc::new(Mutex::new(Response::default()));
        let s = state.clone();
        let spawned = std::thread::Builder::new()
            .name("web request".into())
            .spawn(move || {
                let r = perform(&self);
                *s.lock().unwrap_or_else(|e| e.into_inner()) = r;
            });
        if let Err(e) = spawned {
            *state.lock().unwrap_or_else(|e| e.into_inner()) = Response {
                result: WebRequestResult::ConnectionError,
                error: e.to_string(),
                ..Default::default()
            };
        }
        WebRequestOperation { state }
    }
}

/// `application/x-www-form-urlencoded` escaping (Unity `UnityWebRequest.EscapeURL`).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Clone, Debug)]
struct Response {
    result: WebRequestResult,
    code: u16,
    headers: Vec<(String, String)>,
    data: Vec<u8>,
    error: String,
}

impl Default for Response {
    fn default() -> Self {
        Self {
            result: WebRequestResult::InProgress,
            code: 0,
            headers: Vec::new(),
            data: Vec::new(),
            error: String::new(),
        }
    }
}

fn perform(r: &UnityWebRequest) -> Response {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global((r.timeout > 0).then(|| Duration::from_secs(r.timeout as u64)));
    let agent: ureq::Agent = config.build().into();
    let mut builder = ureq::http::Request::builder()
        .method(r.method.as_str())
        .uri(r.url.as_str());
    for (k, v) in &r.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let request = match builder.body(r.body.clone()) {
        Ok(req) => req,
        Err(e) => {
            return Response {
                result: WebRequestResult::ConnectionError,
                error: format!("bad request: {e}"),
                ..Default::default()
            };
        }
    };
    let mut resp = match agent.run(request) {
        Ok(resp) => resp,
        Err(e) => {
            return Response {
                result: WebRequestResult::ConnectionError,
                error: e.to_string(),
                ..Default::default()
            };
        }
    };
    let code = resp.status().as_u16();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
        .collect();
    let mut data = Vec::new();
    let mut result = if (400..600).contains(&code) {
        WebRequestResult::ProtocolError
    } else {
        WebRequestResult::Success
    };
    let mut error = if result == WebRequestResult::ProtocolError {
        format!(
            "HTTP/1.1 {code} {}",
            resp.status().canonical_reason().unwrap_or("")
        )
    } else {
        String::new()
    };
    if r.method != "HEAD" {
        let read = resp.body_mut().with_config().limit(u64::MAX).read_to_vec();
        match read {
            Ok(bytes) => data = bytes,
            Err(e) => {
                result = WebRequestResult::DataProcessingError;
                error = e.to_string();
            }
        }
    }
    if let (Some(path), WebRequestResult::Success) = (&r.download_to, result) {
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(path, &data));
        if let Err(e) = written {
            result = WebRequestResult::DataProcessingError;
            error = format!("{}: {e}", path.display());
        }
        data.clear();
    }
    Response {
        result,
        code,
        headers,
        data,
        error,
    }
}

/// A sent request (Unity's `UnityWebRequestAsyncOperation` with its finished request).
#[derive(Clone)]
pub struct WebRequestOperation {
    state: Arc<Mutex<Response>>,
}

impl WebRequestOperation {
    fn with<R>(&self, f: impl FnOnce(&Response) -> R) -> R {
        f(&self.state.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Unity `isDone`.
    pub fn is_done(&self) -> bool {
        self.with(|r| r.result != WebRequestResult::InProgress)
    }

    /// Unity `progress` (0 while running, 1 when done).
    pub fn progress(&self) -> f32 {
        if self.is_done() { 1.0 } else { 0.0 }
    }

    /// Unity `result`.
    pub fn result(&self) -> WebRequestResult {
        self.with(|r| r.result)
    }

    /// Unity `responseCode` (0 without a response).
    pub fn response_code(&self) -> u16 {
        self.with(|r| r.code)
    }

    /// Unity `error` (empty on success).
    pub fn error(&self) -> String {
        self.with(|r| r.error.clone())
    }

    /// Unity `GetResponseHeader` (case-insensitive).
    pub fn response_header(&self, name: &str) -> Option<String> {
        self.with(|r| {
            r.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        })
    }

    /// Unity `downloadHandler.data`.
    pub fn data(&self) -> Vec<u8> {
        self.with(|r| r.data.clone())
    }

    /// Unity `downloadHandler.text` (UTF-8, invalid bytes replaced).
    pub fn text(&self) -> String {
        self.with(|r| String::from_utf8_lossy(&r.data).into_owned())
    }

    /// The body parsed as JSON.
    pub fn json(&self) -> Result<serde_json::Value, String> {
        self.with(|r| serde_json::from_slice(&r.data).map_err(|e| e.to_string()))
    }

    /// Unity `DownloadHandlerTexture.GetContent`: the body decoded as an image.
    pub fn texture(&self, name: &str) -> Result<oxide_assets::Texture2D, String> {
        self.with(|r| oxide_assets::Texture2D::decode(name, &r.data, true))
    }

    /// Waits in a coroutine: `op.wait().await` (Unity `yield return request.SendWebRequest()`).
    pub fn wait(&self) -> WaitForRequest {
        WaitForRequest { op: self.clone() }
    }

    /// Blocks until done (tools and tests; games should poll or await).
    pub fn block(&self) -> &Self {
        while !self.is_done() {
            std::thread::sleep(Duration::from_millis(2));
        }
        self
    }
}

/// [`WebRequestOperation::wait`].
pub struct WaitForRequest {
    op: WebRequestOperation,
}

impl Future for WaitForRequest {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if self.op.is_done() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
