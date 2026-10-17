//! UnityWebRequest against a local HTTP server.

use oxide_network::{UnityWebRequest, WebRequestResult};

fn png_2x2() -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 2, 2);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .unwrap()
        .write_image_data(&[255, 0, 0, 255].repeat(4))
        .unwrap();
    out
}

/// Serves the test routes on a free port; returns its base URL.
fn server() -> String {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr());
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let header = |name: &str| {
                req.headers()
                    .iter()
                    .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
                    .map(|h| h.value.as_str().to_owned())
                    .unwrap_or_default()
            };
            let resp = match req.url() {
                "/text" => tiny_http::Response::from_string("hello")
                    .with_header("X-Answer: 42".parse::<tiny_http::Header>().unwrap())
                    .boxed(),
                "/echo" => tiny_http::Response::from_string(format!(
                    "{} {} {} {}",
                    req.method(),
                    body,
                    header("X-Test"),
                    header("Content-Type")
                ))
                .boxed(),
                "/png" => tiny_http::Response::from_data(png_2x2()).boxed(),
                "/slow" => {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    tiny_http::Response::from_string("late").boxed()
                }
                _ => tiny_http::Response::from_string("nope")
                    .with_status_code(404)
                    .boxed(),
            };
            let _ = req.respond(resp);
        }
    });
    base
}

#[test]
fn requests_succeed_fail_and_decode() {
    let base = server();
    // GET: text and headers.
    let op = UnityWebRequest::get(&format!("{base}/text")).send_web_request();
    op.block();
    assert_eq!(op.result(), WebRequestResult::Success, "{}", op.error());
    assert_eq!((op.response_code(), op.text().as_str()), (200, "hello"));
    assert_eq!(op.response_header("x-answer").as_deref(), Some("42"));
    assert_eq!(op.progress(), 1.0);
    // POST with a header; form fields are escaped.
    let mut post = UnityWebRequest::post_form(
        &format!("{base}/echo"),
        &[("name", "Ace Pilot"), ("x", "a&b")],
    );
    post.set_request_header("X-Test", "yes");
    let op = post.send_web_request();
    op.block();
    assert_eq!(
        op.text(),
        "POST name=Ace+Pilot&x=a%26b yes application/x-www-form-urlencoded"
    );
    // JSON, PUT, DELETE.
    let op = UnityWebRequest::post_json(&format!("{base}/echo"), &serde_json::json!({"a": 1}))
        .send_web_request();
    assert_eq!(op.block().text(), "POST {\"a\":1}  application/json");
    let op = UnityWebRequest::put(&format!("{base}/echo"), "data").send_web_request();
    assert!(op.block().text().starts_with("PUT data"));
    let op = UnityWebRequest::delete(&format!("{base}/echo")).send_web_request();
    assert!(op.block().text().starts_with("DELETE"));
    // HEAD: status only.
    let op = UnityWebRequest::head(&format!("{base}/text")).send_web_request();
    assert_eq!((op.block().response_code(), op.data().len()), (200, 0));
    // 404: a protocol error, with the body still readable.
    let op = UnityWebRequest::get(&format!("{base}/missing")).send_web_request();
    op.block();
    assert_eq!(op.result(), WebRequestResult::ProtocolError);
    assert_eq!((op.response_code(), op.text().as_str()), (404, "nope"));
    assert!(op.error().contains("404"), "{}", op.error());
    // A texture.
    let op = UnityWebRequest::get(&format!("{base}/png")).send_web_request();
    let tex = op.block().texture("remote").unwrap();
    assert_eq!((tex.width, tex.height, tex.mips[0][0]), (2, 2, 255));
    // Download to a file.
    let path = std::env::temp_dir().join(format!("oxide_dl_{}.txt", std::process::id()));
    let mut dl = UnityWebRequest::get(&format!("{base}/text"));
    dl.download_to = Some(path.clone());
    assert_eq!(
        dl.send_web_request().block().result(),
        WebRequestResult::Success
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    let _ = std::fs::remove_file(&path);
    // Timeout and refused connections are connection errors.
    let mut slow = UnityWebRequest::get(&format!("{base}/slow"));
    slow.timeout = 1;
    let op = slow.send_web_request();
    assert!(!op.is_done(), "runs in the background");
    assert_eq!(op.block().result(), WebRequestResult::ConnectionError);
    let op = UnityWebRequest::get("http://127.0.0.1:1/nothing").send_web_request();
    assert_eq!(op.block().result(), WebRequestResult::ConnectionError);
}

/// Needs the internet: `cargo test -p oxide_network --test web_request -- --ignored`.
#[test]
#[ignore]
fn https_over_the_internet() {
    let op = UnityWebRequest::get("https://example.com/").send_web_request();
    op.block();
    assert_eq!(op.result(), WebRequestResult::Success, "{}", op.error());
    assert!(op.text().contains("Example Domain"));
}
