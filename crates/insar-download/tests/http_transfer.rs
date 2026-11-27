use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insar_core::DownloadOutcome;
use insar_download::{
    part_path, BurstAuthEndpoints, HttpTransfer, TransferAuth, TransferBackend,
    TransferBackendKind, TransferOptions, TransferRequest, TransferUpdate,
};

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "insar-download-http-{}-{nonce}-{seq}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct LocalServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl LocalServer {
    fn new(responses: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let worker = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                captured.lock().unwrap().push(request);
                stream.write_all(&response).unwrap();
                stream.flush().unwrap();
            }
        });
        Self {
            url: format!("http://{address}/product"),
            requests,
            worker: Some(worker),
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.worker.take().unwrap().join().unwrap();
        Arc::try_unwrap(self.requests)
            .unwrap()
            .into_inner()
            .unwrap()
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        request.push_str(&line);
    }
    request
}

fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut value = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for (name, content) in headers {
        value.push_str(&format!("{name}: {content}\r\n"));
    }
    value.push_str("\r\n");
    let mut bytes = value.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn request(url: &str, destination: &Path, expected_size: Option<u64>) -> TransferRequest {
    TransferRequest::new(
        "task-http",
        "item-http",
        url,
        destination.to_path_buf(),
        expected_size,
    )
    .unwrap()
}

fn transfer(max_attempts: u32) -> HttpTransfer {
    HttpTransfer::new(TransferOptions {
        max_attempts,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(2),
        chunk_size: 2,
        ..TransferOptions::default()
    })
    .unwrap()
}

fn local_burst_endpoints(url: &str) -> BurstAuthEndpoints {
    BurstAuthEndpoints::new(url, url, url).unwrap()
}

#[test]
fn streams_200_to_part_then_atomically_finalizes() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("scene.zip");
    let server = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"abcde",
    )]);

    let transfer = transfer(1);
    let backend: &dyn TransferBackend = &transfer;
    assert_eq!(backend.kind(), TransferBackendKind::NativeSingleStream);
    let result = backend.download_legacy(
        &request(&server.url, &destination, Some(5)),
        &TransferAuth::None,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(result.bytes_written, 5);
    assert_eq!(fs::read(destination).unwrap(), b"abcde");
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn admitted_body_stream_brackets_progress_with_activity_updates() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("activity.zip");
    let server = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"abcde",
    )]);
    let mut updates = Vec::new();

    let result = transfer(1).download_legacy(
        &request(&server.url, &destination, Some(5)),
        &TransferAuth::None,
        &|| false,
        &|| false,
        &mut |update| updates.push(update),
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(
        updates.first(),
        Some(&TransferUpdate::BodyStarted {
            attempt: 1,
            bytes_completed: 0,
        })
    );
    assert!(updates
        .iter()
        .any(|update| matches!(update, TransferUpdate::Progress(_))));
    assert_eq!(
        updates.last(),
        Some(&TransferUpdate::BodyStopped { attempt: 1 })
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn pausing_an_open_body_stops_and_restarts_transfer_activity() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("paused-activity.zip");
    let server = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"abcde",
    )]);
    let paused = Arc::new(AtomicBool::new(false));
    let pause_scheduled = Arc::new(AtomicBool::new(false));
    let mut updates = Vec::new();

    let paused_for_check = Arc::clone(&paused);
    let paused_for_update = Arc::clone(&paused);
    let scheduled_for_update = Arc::clone(&pause_scheduled);
    let result = transfer(1).download_legacy(
        &request(&server.url, &destination, Some(5)),
        &TransferAuth::None,
        &|| false,
        &move || paused_for_check.load(Ordering::SeqCst),
        &mut |update| {
            if matches!(update, TransferUpdate::Progress(_))
                && !scheduled_for_update.swap(true, Ordering::SeqCst)
            {
                paused_for_update.store(true, Ordering::SeqCst);
                let paused = Arc::clone(&paused_for_update);
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(100));
                    paused.store(false, Ordering::SeqCst);
                });
            }
            updates.push(update);
        },
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(
        updates
            .iter()
            .filter(|update| matches!(update, TransferUpdate::BodyStarted { .. }))
            .count(),
        2
    );
    assert_eq!(
        updates
            .iter()
            .filter(|update| matches!(update, TransferUpdate::BodyStopped { .. }))
            .count(),
        2
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn progress_panic_still_closes_body_activity_before_unwinding() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("panic-activity.zip");
    let server = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"abcde",
    )]);
    let mut updates = Vec::new();

    let panicked = catch_unwind(AssertUnwindSafe(|| {
        transfer(1).download_legacy(
            &request(&server.url, &destination, Some(5)),
            &TransferAuth::None,
            &|| false,
            &|| false,
            &mut |update| {
                updates.push(update);
                if matches!(update, TransferUpdate::Progress(_)) {
                    panic!("fixture progress publication panic");
                }
            },
        )
    }));

    assert!(panicked.is_err());
    assert_eq!(
        updates.last(),
        Some(&TransferUpdate::BodyStopped { attempt: 1 })
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn long_transfer_uses_idle_read_timeout_instead_of_total_file_deadline() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("large-scene.zip");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\n")
            .unwrap();
        for byte in [b'a', b'b', b'c'] {
            stream.write_all(&[byte]).unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(70));
        }
    });
    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_millis(120),
        chunk_size: 1,
        ..TransferOptions::default()
    })
    .unwrap();
    let result = transfer.download(
        &request(
            &format!("http://{address}/large-scene.zip"),
            &destination,
            Some(3),
        ),
        &|| false,
        &|| false,
        &mut |_| {},
    );

    worker.join().unwrap();
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(destination).unwrap(), b"abc");
}

#[test]
fn resumes_206_and_preserves_partial_when_server_ignores_range() {
    let temp = TempDirectory::new();
    let resumed = temp.0.join("resume.zip");
    fs::write(part_path(&resumed), b"abc").unwrap();
    let server = LocalServer::new(vec![response(
        "206 Partial Content",
        &[("Content-Length", "2"), ("Content-Range", "bytes 3-4/5")],
        b"de",
    )]);
    let result = transfer(1).download(
        &request(&server.url, &resumed, Some(5)),
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(&resumed).unwrap(), b"abcde");
    assert!(server.finish()[0]
        .to_ascii_lowercase()
        .contains("range: bytes=3-"));

    let restarted = temp.0.join("restart.zip");
    fs::write(part_path(&restarted), b"abc").unwrap();
    let server = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"12345",
    )]);
    let result = transfer(1).download(
        &request(&server.url, &restarted, Some(5)),
        &|| false,
        &|| false,
        &mut |_| {},
    );
    // Owner 2026-09-30: a response ignoring Range is not permission to erase.
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert!(!restarted.exists());
    assert_eq!(fs::read(part_path(&restarted)).unwrap(), b"abc");
    assert!(server.finish()[0]
        .to_ascii_lowercase()
        .contains("range: bytes=3-"));
}

#[test]
fn retries_transient_and_416_without_leaking_extra_concurrency() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("retry.zip");
    let server = LocalServer::new(vec![
        response("503 Service Unavailable", &[("Content-Length", "0")], b""),
        response("200 OK", &[("Content-Length", "5")], b"abcde"),
    ]);
    let result = transfer(2).download(
        &request(&server.url, &destination, Some(5)),
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(result.attempt_count, 2);
    assert_eq!(server.finish().len(), 2);

    let ranged = temp.0.join("range-416.zip");
    fs::write(part_path(&ranged), b"abc").unwrap();
    let server = LocalServer::new(vec![
        response("416 Range Not Satisfiable", &[("Content-Length", "0")], b""),
        response(
            "206 Partial Content",
            &[("Content-Length", "2"), ("Content-Range", "bytes 3-4/5")],
            b"de",
        ),
    ]);
    let result = transfer(2).download(
        &request(&server.url, &ranged, Some(5)),
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    let requests = server.finish();
    assert!(requests[0].to_ascii_lowercase().contains("range: bytes=3-"));
    assert!(requests[1].to_ascii_lowercase().contains("range: bytes=3-"));
    assert_eq!(fs::read(&ranged).unwrap(), b"abcde");
}

#[test]
fn accepted_response_honors_retry_after_without_opening_part() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("burst.tiff");
    let server = LocalServer::new(vec![
        response(
            "202 Accepted",
            &[("Retry-After", "0"), ("Content-Length", "0")],
            b"",
        ),
        response("200 OK", &[("Content-Length", "5")], b"abcde"),
    ]);

    let result = transfer(1).download(
        &request(&server.url, &destination, Some(5)),
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(result.attempt_count, 1);
    assert_eq!(fs::read(&destination).unwrap(), b"abcde");
    assert!(!part_path(&destination).exists());
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn accepted_response_stops_at_the_independent_poll_limit() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("bounded-accepted.bin");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = thread::spawn(move || loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                let request = read_request(&mut stream);
                let index = {
                    let mut requests = captured.lock().unwrap();
                    requests.push(request);
                    requests.len()
                };
                let reply = if index <= 3 {
                    response(
                        "202 Accepted",
                        &[("Retry-After", "0"), ("Content-Length", "0")],
                        b"",
                    )
                } else {
                    response("200 OK", &[("Content-Length", "5")], b"abcde")
                };
                stream.write_all(&reply).unwrap();
                stream.flush().unwrap();
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if worker_stop.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("poll fixture failed: {error}"),
        }
    });
    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(2),
        chunk_size: 2,
        max_accepted_polls: 2,
        accepted_poll_timeout: Duration::from_secs(2),
    })
    .unwrap();

    let result = transfer.download(
        &request(
            &format!("http://{address}/bounded-accepted.bin"),
            &destination,
            Some(5),
        ),
        &|| false,
        &|| false,
        &mut |_| {},
    );

    stop.store(true, Ordering::Release);
    worker.join().unwrap();
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.attempt_count, 1);
    assert_eq!(requests.lock().unwrap().len(), 3);
    assert!(!part_path(&destination).exists());
}

#[test]
fn trusted_style_307_preserves_transfer_and_untrusted_target_never_gets_auth() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("burst.xml");
    let target = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"abcde",
    )]);
    let source = LocalServer::new(vec![response(
        "307 Temporary Redirect",
        &[("Location", &target.url), ("Content-Length", "0")],
        b"",
    )]);
    let auth = TransferAuth::basic("earthdata-user", "SECRET_TEST_PASSWORD").unwrap();

    let result = transfer(1).download_authenticated(
        &request(&source.url, &destination, Some(5)),
        &auth,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert!(source.finish()[0]
        .to_ascii_lowercase()
        .contains("authorization: basic"));
    assert!(!target.finish()[0]
        .to_ascii_lowercase()
        .contains("authorization:"));
}

#[test]
fn complete_file_and_precancel_never_open_a_socket() {
    let temp = TempDirectory::new();
    let complete = temp.0.join("complete.zip");
    fs::write(&complete, b"abcde").unwrap();
    let unreachable = "http://127.0.0.1:9/product";
    let skipped = transfer(1).download(
        &request(unreachable, &complete, Some(5)),
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert_eq!(skipped.outcome, DownloadOutcome::Skipped);

    let cancelled = temp.0.join("cancelled.zip");
    fs::write(part_path(&cancelled), b"abc").unwrap();
    let interrupted = transfer(1).download(
        &request(unreachable, &cancelled, Some(5)),
        &|| true,
        &|| false,
        &mut |_| {},
    );
    assert_eq!(interrupted.outcome, DownloadOutcome::Interrupted);
    assert_eq!(fs::read(part_path(&cancelled)).unwrap(), b"abc");
}

#[test]
fn credential_rejection_is_chinese_bounded_and_not_retried() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("auth.zip");
    let server = LocalServer::new(vec![response(
        "403 Forbidden",
        &[("Content-Length", "0")],
        b"",
    )]);
    let result = transfer(3).download(
        &request(&server.url, &destination, None),
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    let failure = result.failure.unwrap();
    assert_eq!(failure.code, "DL004");
    assert!(failure.reason_zh.contains("Earthdata"));
    assert!(!failure.reason_zh.contains(&server.url));
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn burst_session_rejects_token_and_requires_asf_urs_cookie() {
    let token = TransferAuth::bearer("TOKEN_ONLY_SECRET").unwrap();
    let unreachable = local_burst_endpoints("http://127.0.0.1:9/auth");
    let error = transfer(1)
        .establish_burst_session(&token, &unreachable)
        .unwrap_err();
    assert_eq!(error.code(), "ASF_BURST_COOKIE_REQUIRED");
    assert!(!format!("{error:?}").contains("TOKEN_ONLY_SECRET"));

    let missing_cookie = LocalServer::new(vec![
        response("200 OK", &[("Content-Length", "2")], b"{}"),
        response("200 OK", &[("Content-Length", "0")], b""),
    ]);
    let account = TransferAuth::basic("earth-user", "ACCOUNT_SECRET").unwrap();
    let error = transfer(1)
        .establish_burst_session(&account, &local_burst_endpoints(&missing_cookie.url))
        .unwrap_err();
    assert_eq!(error.code(), "ASF_BURST_COOKIE_REQUIRED");
    assert_eq!(missing_cookie.finish().len(), 2);
    assert!(!format!("{error:?}").contains("ACCOUNT_SECRET"));

    let cookie_session = LocalServer::new(vec![
        response("200 OK", &[("Content-Length", "2")], b"{}"),
        response(
            "200 OK",
            &[
                ("Set-Cookie", "asf-urs=COOKIE_SECRET; Path=/; HttpOnly"),
                ("Content-Length", "0"),
            ],
            b"",
        ),
    ]);
    transfer(1)
        .establish_burst_session(&account, &local_burst_endpoints(&cookie_session.url))
        .unwrap();
    assert_eq!(cookie_session.finish().len(), 2);
}

#[test]
fn truncated_body_keeps_part_and_retries_from_the_written_offset() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("truncated.zip");
    let server = LocalServer::new(vec![
        response("200 OK", &[("Content-Length", "5")], b"abc"),
        response(
            "206 Partial Content",
            &[("Content-Length", "2"), ("Content-Range", "bytes 3-4/5")],
            b"de",
        ),
    ]);
    let result = transfer(2).download(
        &request(&server.url, &destination, Some(5)),
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(destination).unwrap(), b"abcde");
    let requests = server.finish();
    assert!(!requests[0].to_ascii_lowercase().contains("range:"));
    assert!(requests[1].to_ascii_lowercase().contains("range: bytes=3-"));
}

#[test]
fn authorization_is_redacted_and_removed_on_cross_origin_redirect() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("redirect.zip");
    let target = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"abcde",
    )]);
    let source = LocalServer::new(vec![response(
        "302 Found",
        &[("Location", &target.url), ("Content-Length", "0")],
        b"",
    )]);
    let auth = TransferAuth::bearer("SECRET_TEST_TOKEN").unwrap();
    assert!(!format!("{auth:?}").contains("SECRET_TEST_TOKEN"));
    let transfer_request = request(&source.url, &destination, Some(5));
    assert!(!format!("{transfer_request:?}").contains(&source.url));

    let result = transfer(1).download_authenticated(
        &transfer_request,
        &auth,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    let source_requests = source.finish();
    let target_requests = target.finish();
    assert!(source_requests[0]
        .to_ascii_lowercase()
        .contains("authorization: bearer secret_test_token"));
    assert!(!target_requests[0]
        .to_ascii_lowercase()
        .contains("authorization:"));
}

#[test]
fn unauthorized_external_and_userinfo_redirects_are_rejected_without_writing_file() {
    for loc in [
        "https://evil.example.invalid/payload.zip",
        "https://user:pass@127.0.0.1:19999/secret.zip",
    ] {
        let temp = TempDirectory::new();
        let destination = temp.0.join("blocked-redirect.zip");
        let source = LocalServer::new(vec![response(
            "302 Found",
            &[("Location", loc), ("Content-Length", "0")],
            b"",
        )]);
        let req = request(&source.url, &destination, None);
        let result = transfer(1).download(&req, &|| false, &|| false, &mut |_| {});
        let source_requests = source.finish();

        assert_eq!(source_requests.len(), 1);
        assert!(matches!(result.outcome, DownloadOutcome::Failed));
        assert!(!destination.exists());
    }
}

#[test]
fn fresh_segmented_probe_416_falls_back_to_single_stream_and_completes() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("cdse-dynamic.SAFE.zip");
    let body: Vec<u8> = (0..(128 * 1024))
        .map(|i| ((i * 13 + 5) & 0xff) as u8)
        .collect();
    let server = LocalServer::new(vec![
        // 1st request is speculative segmented probe (Range: bytes=0-...) -> 416
        response("416 Range Not Satisfiable", &[("Content-Length", "0")], b""),
        // 2nd request is single-stream fallback GET without Range -> 200 OK full archive
        response(
            "200 OK",
            &[("Content-Length", &body.len().to_string())],
            &body,
        ),
    ]);
    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 16 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(64 * 1024)
    .with_ssd_staging(false, false, None);

    let result = transfer.download(
        &request(&server.url, &destination, Some(body.len() as u64)),
        &|| false,
        &|| false,
        &mut |_| {},
    );
    let requests = server.finish();

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(result.bytes_written, body.len() as u64);
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert_eq!(requests.len(), 2);
    assert!(requests[0].to_ascii_lowercase().contains("range: bytes=0-"));
    assert!(!requests[1].to_ascii_lowercase().contains("range:"));
}

#[test]
fn segmented_unauthorized_external_and_userinfo_redirects_are_rejected_without_writing_file() {
    for loc in [
        "https://evil.example.invalid/payload.zip",
        "https://user:pass@127.0.0.1:19999/secret.zip",
    ] {
        let temp = TempDirectory::new();
        let destination = temp.0.join("blocked-segmented-redirect.zip");
        let source = LocalServer::new(vec![response(
            "302 Found",
            &[("Location", loc), ("Content-Length", "0")],
            b"",
        )]);
        let transfer = HttpTransfer::new(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(5),
            chunk_size: 16 * 1024,
            ..TransferOptions::default()
        })
        .unwrap()
        .with_per_file_segments(4)
        .with_segment_threshold_bytes(64 * 1024)
        .with_ssd_staging(false, false, None);

        let result = transfer.download(
            &request(&source.url, &destination, Some(128 * 1024)),
            &|| false,
            &|| false,
            &mut |_| {},
        );
        let source_requests = source.finish();

        assert_eq!(source_requests.len(), 1);
        assert!(source_requests[0]
            .to_ascii_lowercase()
            .contains("range: bytes=0-"));
        assert!(matches!(result.outcome, DownloadOutcome::Failed));
        assert!(!destination.exists());
    }
}
