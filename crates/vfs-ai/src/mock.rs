//! A local mock provider server that replays canned responses (VFS-28: provider calls in CI go
//! to a local mock; no test needs a real key or the network).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// A response the mock sends, in order, one per request.
#[derive(Debug, Clone)]
pub struct MockResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl MockResponse {
    pub fn json(status: u16, body: impl Into<String>) -> MockResponse {
        MockResponse {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.into(),
        }
    }

    pub fn sse(body: impl Into<String>) -> MockResponse {
        MockResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: body.into(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> MockResponse {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// A request the mock received.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    /// Header names in lowercase.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

pub struct MockServer {
    port: u16,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Starts a server on 127.0.0.1 that answers the requests with `responses` in order, then
    /// with 599 «mock exhausted».
    pub fn start(responses: Vec<MockResponse>) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let port = listener.local_addr().expect("mock address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, flag) = (requests.clone(), stop.clone());
        let handle = thread::spawn(move || {
            let mut queue = responses.into_iter();
            for stream in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let response = queue
                    .next()
                    .unwrap_or_else(|| MockResponse::json(599, "{\"mock\":\"exhausted\"}"));
                if let Some(request) = serve(stream, &response) {
                    log.lock().unwrap().push(request);
                }
            }
        });
        MockServer {
            port,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    /// The base URL to give [`crate::Endpoint::local`].
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(stream: TcpStream, response: &MockResponse) -> Option<RecordedRequest> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut headers = Vec::new();
    let mut length = 0usize;
    loop {
        line.clear();
        reader.read_line(&mut line).ok()?;
        let text = line.trim_end();
        if text.is_empty() {
            break;
        }
        if let Some((name, value)) = text.split_once(':') {
            let name = name.trim().to_lowercase();
            let value = value.trim().to_owned();
            if name == "content-length" {
                length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    let mut out = stream;
    let mut head = format!("HTTP/1.1 {} Mock\r\n", response.status);
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "content-length: {}\r\nconnection: close\r\n\r\n",
        response.body.len()
    ));
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(response.body.as_bytes());
    let _ = out.flush();
    Some(RecordedRequest {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into(),
    })
}

/// An Anthropic rewrite response whose structured output is `rewrite`, in the shape of
/// `tests/fixtures/anthropic_rewrite.json`.
pub fn anthropic_rewrite(rewrite: &crate::Rewrite) -> MockResponse {
    let text = serde_json::to_string(rewrite).unwrap_or_default();
    let body = serde_json::json!({
        "id": "msg_mock_rewrite", "type": "message", "role": "assistant",
        "model": crate::cost::CLAUDE_HAIKU,
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 800, "output_tokens": 60,
                  "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0},
    });
    MockResponse::json(200, body.to_string())
}

/// An Anthropic streamed answer of `blocks`, each citation sent as a `citations_delta` with its
/// `search_result_index`, in the shape of `tests/fixtures/anthropic_answer.sse`.
pub fn anthropic_answer(blocks: &[crate::AnswerBlock]) -> MockResponse {
    let event = |name: &str, data: serde_json::Value| format!("event: {name}\ndata: {data}\n\n");
    let mut sse = event(
        "message_start",
        serde_json::json!({"type": "message_start", "message": {
            "id": "msg_mock_answer", "type": "message", "role": "assistant",
            "model": crate::cost::CLAUDE_SONNET, "content": [], "stop_reason": null,
            "usage": {"input_tokens": 9000, "output_tokens": 1,
                      "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}}}),
    );
    for (i, b) in blocks.iter().enumerate() {
        sse += &event(
            "content_block_start",
            serde_json::json!({"type": "content_block_start", "index": i,
                               "content_block": {"type": "text", "text": "", "citations": []}}),
        );
        for c in &b.citations {
            sse += &event(
                "content_block_delta",
                serde_json::json!({"type": "content_block_delta", "index": i, "delta": {
                    "type": "citations_delta", "citation": {
                        "type": "search_result_location",
                        "source": format!("vfs://passage/{}", c.passage),
                        "title": "", "cited_text": c.quote,
                        "search_result_index": c.passage,
                        "start_block_index": 0, "end_block_index": 0}}}),
            );
        }
        sse += &event(
            "content_block_delta",
            serde_json::json!({"type": "content_block_delta", "index": i,
                               "delta": {"type": "text_delta", "text": b.text}}),
        );
        sse += &event(
            "content_block_stop",
            serde_json::json!({"type": "content_block_stop", "index": i}),
        );
    }
    sse += &event(
        "message_delta",
        serde_json::json!({"type": "message_delta",
                           "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                           "usage": {"output_tokens": 300}}),
    );
    sse += &event("message_stop", serde_json::json!({"type": "message_stop"}));
    MockResponse::sse(sse)
}
