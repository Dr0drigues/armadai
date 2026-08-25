//! Minimal scripted HTTP/1.1 server, shared by the API providers' tests.
//!
//! Every test that uses it exercises the provider against **real wire
//! traffic** — a real status line, real headers, a real socket — without
//! calling any external API and without needing a key. Mocking at the
//! `reqwest::Client` level would prove nothing about parsing bytes off a
//! socket, which is exactly where hand-written SSE readers break.
//!
//! Originally written inside `retry.rs`'s test module (rate-limit Lot 2,
//! #358); lifted here when `openai_compatible.rs` needed the same thing,
//! rather than growing a second copy — two implementations of one test
//! primitive is the defect class this repo has closed repeatedly.
//!
//! What it adds over the original:
//!
//! - **Multi-write bodies** (`ScriptedResponse::streamed`): each chunk is
//!   its own `write_all` + flush, with `TCP_NODELAY` set and a short pause
//!   in between, so the client really observes several reads. That is what
//!   makes "a `data:` line split across two TCP packets" a *measured*
//!   condition instead of a hopeful one.
//! - **Unframed bodies**: a streamed response sends no `Content-Length`, so
//!   the body ends when the connection closes — how an SSE response
//!   actually arrives.
//! - **Request capture** (`request(i)`): the raw bytes the server received,
//!   so a test can assert on what was *sent* (e.g. that no `Authorization`
//!   header went out for a keyless proxy).
//!
//! Each accepted connection serves exactly one scripted response and then
//! closes (`Connection: close`, so reqwest never reuses a stale connection
//! across scripts). Requests past the end of the script repeat the last
//! entry — tests that don't know the exact attempt count (the
//! give-up-after-budget case in `retry.rs`) rely on that.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Extra response headers, as `(name, value)` pairs.
pub(crate) type ScriptedHeaders = Vec<(&'static str, String)>;

/// One scripted response.
#[derive(Clone)]
pub(crate) struct ScriptedResponse {
    pub(crate) status: u16,
    pub(crate) headers: ScriptedHeaders,
    /// Body pieces. Each is written and flushed separately, with a pause in
    /// between, so a multi-piece body genuinely crosses several packets.
    pub(crate) chunks: Vec<String>,
    /// `true` → framed by `Content-Length` (an ordinary JSON response).
    /// `false` → no framing header at all; the body ends at connection
    /// close, which is how a streaming SSE response is delimited.
    pub(crate) framed: bool,
}

impl ScriptedResponse {
    /// A whole body in one write, framed by `Content-Length`.
    pub(crate) fn body(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            chunks: vec![body.into()],
            framed: true,
        }
    }

    /// A body delivered as several separate writes and delimited by the
    /// connection closing — the shape of a streaming response.
    pub(crate) fn streamed<S: Into<String>>(status: u16, chunks: Vec<S>) -> Self {
        Self {
            status,
            headers: vec![("Content-Type", "text/event-stream".to_string())],
            chunks: chunks.into_iter().map(Into::into).collect(),
            framed: false,
        }
    }
}

/// Keeps `retry.rs`'s existing `(status, headers, body)` tuples working
/// unchanged after the extraction.
impl From<(u16, ScriptedHeaders, &'static str)> for ScriptedResponse {
    fn from((status, headers, body): (u16, ScriptedHeaders, &'static str)) -> Self {
        Self {
            status,
            headers,
            chunks: vec![body.to_string()],
            framed: true,
        }
    }
}

pub(crate) struct ScriptedServer {
    addr: std::net::SocketAddr,
    requests: Arc<AtomicUsize>,
    received: Arc<Mutex<Vec<String>>>,
}

impl ScriptedServer {
    pub(crate) fn start<R: Into<ScriptedResponse>>(responses: Vec<R>) -> Self {
        let responses: Vec<ScriptedResponse> = responses.into_iter().map(Into::into).collect();
        assert!(
            !responses.is_empty(),
            "script must have at least one response"
        );

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test server");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(AtomicUsize::new(0));
        let received = Arc::new(Mutex::new(Vec::new()));

        let requests_clone = requests.clone();
        let received_clone = received.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let idx = requests_clone.fetch_add(1, Ordering::SeqCst);
                let scripted = responses
                    .get(idx)
                    .or_else(|| responses.last())
                    .cloned()
                    .expect("script is non-empty");

                let _ = stream.set_nodelay(true);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

                // Drain the whole request (headers, then the declared body).
                // Leaving unread bytes in the socket makes close() send an
                // RST on some platforms, which the client would see instead
                // of the response.
                let raw = read_request(&mut stream);
                if let Ok(mut guard) = received_clone.lock() {
                    guard.push(raw);
                }

                let mut head = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", scripted.status);
                if scripted.framed {
                    let len: usize = scripted.chunks.iter().map(|c| c.len()).sum();
                    head.push_str(&format!("Content-Length: {len}\r\n"));
                }
                for (k, v) in &scripted.headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                if stream.write_all(head.as_bytes()).is_err() {
                    continue;
                }
                let _ = stream.flush();

                for (i, chunk) in scripted.chunks.iter().enumerate() {
                    if i > 0 {
                        // Long enough that the client's read loop wakes up
                        // between writes, short enough to keep the suite fast.
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    if stream.write_all(chunk.as_bytes()).is_err() {
                        break;
                    }
                    let _ = stream.flush();
                }
            }
        });

        Self {
            addr,
            requests,
            received,
        }
    }

    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub(crate) fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// Raw text of the request received at `index` (request line, headers,
    /// and body), or `None` if that many requests never arrived.
    pub(crate) fn request(&self, index: usize) -> Option<String> {
        self.received.lock().ok()?.get(index).cloned()
    }
}

/// Read one HTTP request: everything up to the blank line, then exactly the
/// number of body bytes its `Content-Length` declares (0 when absent).
fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut raw: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];

    // Headers, byte by byte so we stop exactly at the blank line and never
    // swallow part of a body we haven't sized yet.
    while !raw.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return String::from_utf8_lossy(&raw).into_owned(),
            Ok(_) => raw.push(byte[0]),
        }
    }

    let head = String::from_utf8_lossy(&raw).into_owned();
    let content_length = head
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);

    if content_length > 0 {
        let mut body = vec![0u8; content_length];
        if stream.read_exact(&mut body).is_ok() {
            raw.extend_from_slice(&body);
        }
    }

    String::from_utf8_lossy(&raw).into_owned()
}
