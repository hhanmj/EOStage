use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const MAX_REQUEST_HEADER_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseMode {
    Normal,
    IgnoreRange,
    BadContentRange,
    WrongTotal,
    WrongContentLength,
    Truncate,
    Corrupt,
    MissingEtag,
    WeakEtag,
    ChangedStrongEtag,
    Encoded,
    Status416,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedRangeRequest {
    pub range: Option<String>,
    pub if_range: Option<String>,
    pub accept_encoding: Option<String>,
    pub status: u16,
}

struct ServerState {
    body: Vec<u8>,
    etag: String,
    mode: ResponseMode,
}

pub struct RangeServer {
    address: SocketAddr,
    url: String,
    state: Arc<Mutex<ServerState>>,
    requests: Arc<Mutex<Vec<CapturedRangeRequest>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl RangeServer {
    pub fn new(body: Vec<u8>, etag: &str, mode: ResponseMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback Range fixture");
        listener
            .set_nonblocking(true)
            .expect("configure nonblocking Range fixture");
        let address = listener.local_addr().expect("Range fixture address");
        let state = Arc::new(Mutex::new(ServerState {
            body,
            etag: etag.to_owned(),
            mode,
        }));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_requests = Arc::clone(&requests);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("configure accepted Range fixture stream");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .expect("Range fixture read timeout");
                        let headers = read_request_headers(&mut stream);
                        let range = header(&headers, "Range").map(str::to_owned);
                        let if_range = header(&headers, "If-Range").map(str::to_owned);
                        let accept_encoding =
                            header(&headers, "Accept-Encoding").map(str::to_owned);
                        let snapshot = worker_state.lock().expect("Range fixture state");
                        let response = build_response(
                            &snapshot.body,
                            &snapshot.etag,
                            snapshot.mode,
                            range.as_deref(),
                            if_range.as_deref(),
                        );
                        worker_requests.lock().expect("Range captures").push(
                            CapturedRangeRequest {
                                range,
                                if_range,
                                accept_encoding,
                                status: response.status,
                            },
                        );
                        drop(snapshot);
                        let _ = stream.write_all(&response.bytes);
                        let _ = stream.flush();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("Range fixture accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            url: format!("http://{address}/synthetic-rslc.h5"),
            state,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn set_mode(&self, mode: ResponseMode) {
        self.state.lock().expect("Range fixture state").mode = mode;
    }

    pub fn replace_object(&self, body: Vec<u8>, etag: &str) {
        let mut state = self.state.lock().expect("Range fixture state");
        state.body = body;
        state.etag = etag.to_owned();
        state.mode = ResponseMode::Normal;
    }

    pub fn requests(&self) -> Vec<CapturedRangeRequest> {
        self.requests.lock().expect("Range captures").clone()
    }
}

impl Drop for RangeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("join Range fixture worker");
        }
    }
}

struct FixtureResponse {
    status: u16,
    bytes: Vec<u8>,
}

fn build_response(
    object: &[u8],
    etag: &str,
    mode: ResponseMode,
    range_header: Option<&str>,
    if_range: Option<&str>,
) -> FixtureResponse {
    let parsed = range_header.and_then(parse_range);
    let mut status = if mode == ResponseMode::Status416 {
        416
    } else if parsed.is_none() || if_range != Some(etag) || mode == ResponseMode::IgnoreRange {
        200
    } else {
        206
    };
    let (start, end) = if status == 206 {
        let (start, end) = parsed.expect("206 requires parsed range");
        if start > end || end >= object.len() {
            status = 416;
            (0, 0)
        } else {
            (start, end)
        }
    } else {
        (0, object.len().saturating_sub(1))
    };

    let mut body = if status == 416 || object.is_empty() {
        Vec::new()
    } else {
        object[start..=end].to_vec()
    };
    if mode == ResponseMode::Corrupt && !body.is_empty() {
        body[0] ^= 0xff;
    }
    let declared_width = body.len();
    if mode == ResponseMode::Truncate && body.len() > 1 {
        body.truncate(body.len() / 2);
    }

    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        416 => "Range Not Satisfiable",
        _ => "Fixture",
    };
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nConnection: close\r\n");
    if mode != ResponseMode::MissingEtag {
        let value = match mode {
            ResponseMode::WeakEtag => format!("W/{etag}"),
            ResponseMode::ChangedStrongEtag => "\"unexpected-strong-etag\"".to_owned(),
            _ => etag.to_owned(),
        };
        head.push_str(&format!("ETag: {value}\r\n"));
    }
    if status == 206 {
        let header_start = if mode == ResponseMode::BadContentRange {
            start + 1
        } else {
            start
        };
        let total = if mode == ResponseMode::WrongTotal {
            object.len() + 1
        } else {
            object.len()
        };
        head.push_str(&format!(
            "Content-Range: bytes {header_start}-{end}/{total}\r\n"
        ));
    }
    if mode == ResponseMode::Encoded {
        head.push_str("Content-Encoding: gzip\r\n");
    }
    let content_length = if mode == ResponseMode::WrongContentLength {
        declared_width + 1
    } else {
        declared_width
    };
    head.push_str(&format!("Content-Length: {content_length}\r\n\r\n"));
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(&body);
    FixtureResponse { status, bytes }
}

fn read_request_headers(stream: &mut TcpStream) -> Vec<(String, String)> {
    let mut bytes = Vec::new();
    let mut block = [0_u8; 512];
    loop {
        let count = stream.read(&mut block).expect("read Range request");
        assert!(count > 0, "Range request ended before headers");
        bytes.extend_from_slice(&block[..count]);
        assert!(
            bytes.len() <= MAX_REQUEST_HEADER_BYTES,
            "Range request headers exceed fixture bound"
        );
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&bytes).expect("Range request is UTF-8");
    request
        .split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line.split_once(':').expect("Range request header syntax");
            (name.trim().to_owned(), value.trim().to_owned())
        })
        .collect()
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn parse_range(value: &str) -> Option<(usize, usize)> {
    let value = value.strip_prefix("bytes=")?;
    let (start, end) = value.split_once('-')?;
    if start.is_empty() || end.is_empty() || end.contains(',') {
        return None;
    }
    Some((start.parse().ok()?, end.parse().ok()?))
}
