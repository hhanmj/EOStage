//! Test-only loopback server that answers sequential STAC JSON pages, one
//! request per connection. It lets #22 B slice 3(a) fixtures generate
//! 10,000-record catalogues programmatically instead of committing JSON.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

pub(crate) struct JsonPageServer {
    pub(crate) origin: String,
    calls: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl JsonPageServer {
    /// Serve `pages` in order; any request beyond them gets HTTP 500.
    pub(crate) fn new(pages: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let expected = pages.len();
        let thread = thread::spawn(move || {
            let mut pages = pages.into_iter();
            // One extra accept answers an unexpected request with a failure.
            for _ in 0..=expected {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                if !read_request(&mut stream) {
                    return;
                }
                counted.fetch_add(1, Ordering::SeqCst);
                let (status, body) = match pages.next() {
                    Some(page) => (200, serde_json::to_vec(&page).unwrap()),
                    None => (500, b"{}".to_vec()),
                };
                let header = format!(
                    "HTTP/1.1 {status} Test\r\nConnection: close\r\nContent-Type: application/geo+json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        Self {
            origin,
            calls,
            thread: Some(thread),
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Drop for JsonPageServer {
    fn drop(&mut self) {
        // Unblock a pending accept so the thread can finish.
        let _ = std::net::TcpStream::connect(
            self.origin
                .trim_start_matches("http://")
                .trim_end_matches('/'),
        );
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Read one HTTP/1.1 request (headers plus a Content-Length body).
fn read_request(stream: &mut std::net::TcpStream) -> bool {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let Ok(read) = stream.read(&mut buffer) else {
            return false;
        };
        if read == 0 {
            return false;
        }
        bytes.extend_from_slice(&buffer[..read]);
        let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if bytes.len() >= end + 4 + length {
            return true;
        }
    }
}
