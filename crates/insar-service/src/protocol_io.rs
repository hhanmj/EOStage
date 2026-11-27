use std::io::{self, BufRead, Write};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use insar_core::{SidecarOperation, SidecarRequest, SidecarResponse};
use serde_json::{json, Map, Value};

use crate::dispatcher::{error_response, safe_request_id};
use crate::{DispatchClass, Dispatcher, RequestDispatcher};

// A compact 50k-product workbench is commonly 70-100 MiB. Keep one explicit
// bounded frame large enough for that reviewed desktop case; larger/unbounded
// catalogue persistence still belongs on the paged store protocol.
pub const MAX_MESSAGE_BYTES: usize = 128 * 1024 * 1024;
const REQUEST_ID_SCAN_BYTES: usize = 8 * 1024;
pub const DEFAULT_WORKER_COUNT: usize = 4;
pub const DEFAULT_WORK_QUEUE_CAPACITY: usize = 64;

enum BoundedLine {
    Eof,
    Line(Vec<u8>),
    TooLarge(String),
}

pub fn serve<R: BufRead, W: Write + Send>(
    reader: R,
    writer: W,
    dispatcher: &Dispatcher,
) -> io::Result<()> {
    serve_with_limits(
        reader,
        writer,
        dispatcher,
        DEFAULT_WORKER_COUNT,
        DEFAULT_WORK_QUEUE_CAPACITY,
    )
}

pub fn serve_with_limits<R: BufRead, W: Write + Send, D: RequestDispatcher>(
    mut reader: R,
    mut writer: W,
    dispatcher: &D,
    worker_count: usize,
    queue_capacity: usize,
) -> io::Result<()> {
    if worker_count == 0 || queue_capacity == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker count and queue capacity must be positive",
        ));
    }
    let (jobs_tx, jobs_rx) = mpsc::sync_channel::<SidecarRequest>(queue_capacity);
    let jobs_rx = Arc::new(Mutex::new(jobs_rx));
    let (responses_tx, responses_rx) = mpsc::channel::<SidecarResponse>();

    thread::scope(|scope| {
        let writer_handle = scope.spawn(move || -> io::Result<()> {
            for response in responses_rx {
                write_response(&mut writer, &response)?;
            }
            Ok(())
        });
        let mut worker_handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let jobs_rx = Arc::clone(&jobs_rx);
            let responses_tx = responses_tx.clone();
            worker_handles.push(scope.spawn(move || loop {
                let request = {
                    let receiver = jobs_rx.lock().expect("work receiver lock is not poisoned");
                    receiver.recv()
                };
                let Ok(request) = request else {
                    break;
                };
                if responses_tx.send(dispatcher.dispatch(request)).is_err() {
                    break;
                }
            }));
        }

        loop {
            let parsed = match read_bounded_line(&mut reader)? {
                BoundedLine::Eof => break,
                BoundedLine::TooLarge(request_id) => Err(Box::new(error_response(
                    request_id,
                    "V3_PROTOCOL_MESSAGE_TOO_LARGE",
                    "Sidecar 协议消息超过大小限制",
                    Map::from_iter([("maximum_bytes".to_string(), json!(MAX_MESSAGE_BYTES))]),
                ))),
                BoundedLine::Line(line) if line.is_empty() => continue,
                BoundedLine::Line(line) => parse_request(&line),
            };
            match parsed {
                Err(response) => responses_tx.send(*response).map_err(channel_closed)?,
                Ok(request)
                    if dispatcher.classify(&request.operation) == DispatchClass::Control =>
                {
                    responses_tx
                        .send(dispatcher.dispatch(request))
                        .map_err(channel_closed)?;
                    if dispatcher.is_shutting_down() {
                        break;
                    }
                }
                Ok(request) => {
                    let request_id = request.request_id.clone();
                    match jobs_tx.try_send(request) {
                        Ok(()) => {}
                        Err(mpsc::TrySendError::Full(_)) => responses_tx
                            .send(error_response(
                                request_id,
                                "V3_SIDECAR_BUSY",
                                "Rust Sidecar 工作队列已满，请稍后重试",
                                Map::from_iter([(
                                    "queue_capacity".to_string(),
                                    json!(queue_capacity),
                                )]),
                            ))
                            .map_err(channel_closed)?,
                        Err(mpsc::TrySendError::Disconnected(_)) => {
                            return Err(io::Error::new(
                                io::ErrorKind::BrokenPipe,
                                "sidecar worker queue closed",
                            ));
                        }
                    }
                }
            }
        }
        drop(jobs_tx);
        for handle in worker_handles {
            handle
                .join()
                .map_err(|_| io::Error::other("sidecar worker panicked"))?;
        }
        drop(responses_tx);
        writer_handle
            .join()
            .map_err(|_| io::Error::other("sidecar writer panicked"))?
    })
}

fn channel_closed<T>(_error: mpsc::SendError<T>) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "sidecar response channel closed")
}

fn parse_request(line: &[u8]) -> Result<SidecarRequest, Box<SidecarResponse>> {
    let value: Value = match serde_json::from_slice(line) {
        Ok(value) => value,
        Err(_) => {
            return Err(Box::new(error_response(
                "unknown".to_string(),
                "V3_PROTOCOL_INVALID_JSON",
                "Sidecar 协议消息不是有效 JSON",
                Map::new(),
            )))
        }
    };
    let request_id = value
        .get("request_id")
        .and_then(Value::as_str)
        .map(safe_request_id)
        .unwrap_or_else(|| "unknown".to_string());
    if let Some(operation) = value.get("operation").and_then(Value::as_str) {
        if serde_json::from_value::<SidecarOperation>(json!(operation)).is_err() {
            return Err(Box::new(error_response(
                request_id,
                "V3_PROTOCOL_UNKNOWN_OPERATION",
                "Sidecar 请求操作不受支持",
                Map::new(),
            )));
        }
    }
    serde_json::from_value(value).map_err(|_| {
        Box::new(error_response(
            request_id,
            "V3_PROTOCOL_INVALID_REQUEST",
            "Sidecar 请求格式无效",
            Map::new(),
        ))
    })
}

fn write_response<W: Write>(writer: &mut W, response: &SidecarResponse) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, response).map_err(io::Error::other)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<BoundedLine> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(BoundedLine::Eof)
            } else {
                trim_line_ending(&mut line);
                Ok(BoundedLine::Line(line))
            };
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(buffer.len(), |index| index + 1);
        if line.len() + consumed > MAX_MESSAGE_BYTES {
            let request_id = oversized_request_id(&line, &buffer[..consumed]);
            reader.consume(consumed);
            if newline.is_none() {
                drain_line(reader)?;
            }
            return Ok(BoundedLine::TooLarge(request_id));
        }
        line.extend_from_slice(&buffer[..consumed]);
        reader.consume(consumed);
        if newline.is_some() {
            trim_line_ending(&mut line);
            return Ok(BoundedLine::Line(line));
        }
    }
}

fn oversized_request_id(line: &[u8], overflow: &[u8]) -> String {
    let mut prefix = Vec::with_capacity(REQUEST_ID_SCAN_BYTES);
    let from_line = line.len().min(REQUEST_ID_SCAN_BYTES);
    prefix.extend_from_slice(&line[..from_line]);
    if prefix.len() < REQUEST_ID_SCAN_BYTES {
        let remaining = REQUEST_ID_SCAN_BYTES - prefix.len();
        prefix.extend_from_slice(&overflow[..overflow.len().min(remaining)]);
    }
    request_id_from_prefix(&prefix).unwrap_or_else(|| "unknown".to_string())
}

fn request_id_from_prefix(prefix: &[u8]) -> Option<String> {
    const FIELD: &[u8] = b"\"request_id\"";
    let field_start = prefix
        .windows(FIELD.len())
        .position(|window| window == FIELD)?;
    let mut cursor = field_start + FIELD.len();
    while matches!(prefix.get(cursor), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        cursor += 1;
    }
    if prefix.get(cursor) != Some(&b':') {
        return None;
    }
    cursor += 1;
    while matches!(prefix.get(cursor), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        cursor += 1;
    }
    if prefix.get(cursor) != Some(&b'\"') {
        return None;
    }
    let value_start = cursor;
    cursor += 1;
    let mut escaped = false;
    while let Some(byte) = prefix.get(cursor).copied() {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'\"' {
            let raw = &prefix[value_start..=cursor];
            let value = serde_json::from_slice::<String>(raw).ok()?;
            return Some(safe_request_id(&value));
        }
        cursor += 1;
    }
    None
}

fn drain_line<R: BufRead>(reader: &mut R) -> io::Result<()> {
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(());
        }
        if let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
            reader.consume(index + 1);
            return Ok(());
        }
        let consumed = buffer.len();
        reader.consume(consumed);
    }
}

fn trim_line_ending(line: &mut Vec<u8>) {
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
}
