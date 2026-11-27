use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insar_core::DownloadOutcome;
use insar_download::disk_guard::{FreeSpaceProbe, VolumeSpace};
use insar_download::{
    active_default_ssd_staging_dir, load_and_activate_transfer_engine_prefs_in_dir,
    part_completed_bytes, part_path, save_and_activate_transfer_engine_prefs_in_dir,
    set_active_default_ssd_staging_dir, set_active_transfer_engine_prefs, AuthRefreshPolicyId,
    BandwidthLimiter, HostConcurrencyTracker, HttpTransfer, RefreshDecision, RelativeSpeedConfig,
    RemoteValidator, ResolvedAsset, ResolvedChecksum, ResolvedMethod, ResolvedTransferResult,
    SmartConcurrencyInputs, SmartConcurrencyTuner, TransferEnginePrefs, TransferOptions,
    TransferRequest,
};
use reqwest::Url;
use sha2::{Digest, Sha256};

static PREFS_LOCK: Mutex<()> = Mutex::new(());

struct ActivePrefsGuard;

impl Drop for ActivePrefsGuard {
    fn drop(&mut self) {
        let _ = set_active_transfer_engine_prefs(TransferEnginePrefs::default());
        set_active_default_ssd_staging_dir(None);
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "insar-dl-upgrade-{tag}-{}-{nonce}-{seq}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct MockFreeSpaceProbe(Option<VolumeSpace>);

impl FreeSpaceProbe for MockFreeSpaceProbe {
    fn volume_space(&self, _path: &Path) -> Option<VolumeSpace> {
        self.0
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn deterministic_payload(len: usize) -> Vec<u8> {
    (0..len)
        .map(|idx| ((idx * 31 + (idx >> 5) * 17 + 7) & 0xff) as u8)
        .collect()
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                request.push_str(&line);
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
        }
    }
    request
}

fn parse_range_header(request: &str, total_len: usize) -> Option<(usize, usize)> {
    for line in request.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(spec) = lower.strip_prefix("range:") {
            let spec = spec.trim();
            if let Some(bytes_spec) = spec.strip_prefix("bytes=") {
                let mut parts = bytes_spec.split('-');
                let start: usize = parts.next()?.trim().parse().ok()?;
                let end_str = parts.next()?.trim();
                let end_inclusive = if end_str.is_empty() {
                    total_len.saturating_sub(1)
                } else {
                    end_str
                        .parse::<usize>()
                        .ok()?
                        .min(total_len.saturating_sub(1))
                };
                if start <= end_inclusive && end_inclusive < total_len {
                    return Some((start, end_inclusive));
                }
            }
        }
    }
    None
}

struct RangeHttpServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RangeHttpServer {
    fn start(
        payload: Arc<Vec<u8>>,
        etag: &'static str,
        supports_range: bool,
        throttle_after_first_range: Option<Duration>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            let mut active = Vec::new();
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let req = read_http_request(&mut stream);
                        if req.is_empty() {
                            continue;
                        }
                        let req_idx = {
                            let mut guard = captured.lock().unwrap();
                            let idx = guard.len();
                            guard.push(req.clone());
                            idx
                        };
                        let body = Arc::clone(&payload);
                        let worker = thread::spawn(move || {
                            let total = body.len();
                            if supports_range {
                                if let Some((start, end_inclusive)) =
                                    parse_range_header(&req, total)
                                {
                                    if let Some(delay) = throttle_after_first_range {
                                        if (1..=3).contains(&req_idx) {
                                            thread::sleep(delay);
                                        }
                                    }
                                    let slice = &body[start..=end_inclusive];
                                    let hdr = format!(
                                        "HTTP/1.1 206 Partial Content\r\n\
                                         Connection: close\r\n\
                                         Accept-Ranges: bytes\r\n\
                                         ETag: \"{etag}\"\r\n\
                                         Content-Range: bytes {start}-{end_inclusive}/{total}\r\n\
                                         Content-Length: {}\r\n\r\n",
                                        slice.len()
                                    );
                                    let _ = stream.write_all(hdr.as_bytes());
                                    let _ = stream.write_all(slice);
                                    let _ = stream.flush();
                                    return;
                                }
                            }
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\n\
                                 Connection: close\r\n\
                                 ETag: \"{etag}\"\r\n\
                                 Content-Length: {total}\r\n\r\n"
                            );
                            let _ = stream.write_all(hdr.as_bytes());
                            let _ = stream.write_all(&body);
                            let _ = stream.flush();
                        });
                        active.push(worker);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
            for worker in active {
                let _ = worker.join();
            }
        });

        Self {
            url: format!("http://{addr}/scene.bin"),
            requests,
            stop,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for RangeHttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[test]
fn multi_segment_range_206_downloads_and_verifies_exact_bytes() {
    let temp = TempDir::new("seg206");
    let destination = temp.0.join("product_seg4.bin");
    let payload = Arc::new(deterministic_payload(512 * 1024));
    let expected_sha = sha256_hex(&payload);
    let server = RangeHttpServer::start(Arc::clone(&payload), "etag-seg-v1", true, None);

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

    let req = TransferRequest::new(
        "task-seg",
        "item-seg",
        &server.url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();

    let result = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    let requests = server.finish();

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(result.bytes_written, payload.len() as u64);
    assert_eq!(transfer.segmented_transfers_count(), 1);
    assert!(
        requests.len() >= 4,
        "expected >= 4 HTTP Range requests, got {}",
        requests.len()
    );
    assert!(requests
        .iter()
        .all(|r| r.to_ascii_lowercase().contains("range: bytes=")));
    let disk_bytes = fs::read(&destination).unwrap();
    assert_eq!(sha256_hex(&disk_bytes), expected_sha);
    assert!(!part_path(&destination).exists());
    let meta_path = PathBuf::from(format!(
        "{}.meta.json",
        part_path(&destination).to_string_lossy()
    ));
    assert!(!meta_path.exists());
}

#[test]
fn segmented_pause_resumes_from_part_meta_same_url_and_finishes_cleanly() {
    let temp = TempDir::new("seg-resume-same-url");
    let destination = temp.0.join("resumable.bin");
    let payload = Arc::new(deterministic_payload(512 * 1024));
    let expected_sha = sha256_hex(&payload);
    let server = RangeHttpServer::start(
        Arc::clone(&payload),
        "etag-resume-1",
        true,
        Some(Duration::from_millis(140)),
    );

    let pause_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let pause_trigger = Arc::clone(&pause_flag);

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 8 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(64 * 1024)
    .with_ssd_staging(false, false, None);

    let req = TransferRequest::new(
        "task-seg-res",
        "item-seg-res",
        &server.url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();

    let paused_observed = Arc::new(AtomicBool::new(false));
    let paused_observed_cb = Arc::clone(&paused_observed);
    let pause_watcher_flag = Arc::clone(&pause_flag);
    let cancel_watcher_flag = Arc::clone(&cancel_flag);
    let watcher = thread::spawn(move || {
        while !paused_observed_cb.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(80));
        cancel_watcher_flag.store(true, Ordering::Release);
        pause_watcher_flag.store(false, Ordering::Release);
    });

    let res1 = transfer.download(
        &req,
        &|| cancel_flag.load(Ordering::Acquire),
        &|| pause_flag.load(Ordering::Acquire),
        &mut |update| {
            if let insar_download::TransferUpdate::Progress(p) = update {
                if p.bytes_completed > 0 && !paused_observed.swap(true, Ordering::AcqRel) {
                    pause_trigger.store(true, Ordering::Release);
                }
            }
        },
    );
    watcher.join().unwrap();
    assert_eq!(res1.outcome, DownloadOutcome::Interrupted);
    let part = part_path(&destination);
    assert!(part.is_file(), ".part must stay on disk across pause");
    let meta_path = PathBuf::from(format!("{}.meta.json", part.to_string_lossy()));
    assert!(
        meta_path.is_file(),
        ".part.meta.json must persist segment progress"
    );

    let res2 = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    let _requests = server.finish();

    assert_eq!(res2.outcome, DownloadOutcome::Completed);
    let disk_bytes = fs::read(&destination).unwrap();
    assert_eq!(sha256_hex(&disk_bytes), expected_sha);
    assert!(!part.exists());
    assert!(!meta_path.exists());
}

#[test]
fn non_range_server_200_ok_falls_back_cleanly_to_single_stream() {
    let temp = TempDir::new("non-range");
    let destination = temp.0.join("fallback.bin");
    let payload = Arc::new(deterministic_payload(128 * 1024));
    let server = RangeHttpServer::start(Arc::clone(&payload), "etag-norange", false, None);

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 16 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(32 * 1024)
    .with_ssd_staging(false, false, None);

    let req = TransferRequest::new(
        "task-fallback",
        "item-fallback",
        &server.url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();

    let result = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    let requests = server.finish();

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(transfer.segmented_transfers_count(), 0);
    assert_eq!(
        requests.len(),
        1,
        "200 OK fallback must reuse probe stream without issuing extra requests"
    );
    assert_eq!(fs::read(&destination).unwrap(), *payload);
}

#[test]
fn single_stream_resume_skips_prefix_sha256_rehash_when_part_meta_matches() {
    let temp = TempDir::new("single-fast-resume");
    let destination = temp.0.join("single.bin");
    let payload = deterministic_payload(64 * 1024);
    let split = 24 * 1024;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/single.bin");

    let payload_clone = payload.clone();
    let server = thread::spawn(move || {
        // Request 1: sends partial body then drops connection (simulating interrupted stream).
        {
            let (mut stream, _) = listener.accept().unwrap();
            let _req1 = read_http_request(&mut stream);
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\nETag: \"single-v1\"\r\nContent-Length: {}\r\n\r\n",
                payload_clone.len()
            );
            stream.write_all(hdr.as_bytes()).unwrap();
            stream.write_all(&payload_clone[..split]).unwrap();
            stream.flush().unwrap();
        }
        // Request 2: serves remaining range from exact split offset.
        {
            let (mut stream, _) = listener.accept().unwrap();
            let req2 = read_http_request(&mut stream);
            assert!(
                req2.to_ascii_lowercase()
                    .contains(&format!("range: bytes={split}-")),
                "expected Range resume from {split}, got: {req2}"
            );
            let tail = &payload_clone[split..];
            let hdr = format!(
                "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nETag: \"single-v1\"\r\nContent-Range: bytes {split}-{}/{}\r\nContent-Length: {}\r\n\r\n",
                payload_clone.len() - 1,
                payload_clone.len(),
                tail.len()
            );
            stream.write_all(hdr.as_bytes()).unwrap();
            stream.write_all(tail).unwrap();
            stream.flush().unwrap();
        }
    });

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 2,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 8 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(1)
    .with_ssd_staging(false, false, None);

    let req = TransferRequest::new(
        "task-fast-res",
        "item-fast-res",
        &url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();

    let res = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    server.join().unwrap();

    assert_eq!(res.outcome, DownloadOutcome::Completed);
    assert_eq!(transfer.resume_prefix_hash_skipped_count(), 1);
    assert_eq!(fs::read(&destination).unwrap(), payload);
}

#[test]
fn low_speed_stall_watchdog_reconnects_from_exact_offset_and_completes() {
    let temp = TempDir::new("stall-reconnect");
    let destination = temp.0.join("stalled.bin");
    let payload = deterministic_payload(32 * 1024);
    let prefix = 8 * 1024;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/stalled.bin");
    let payload_clone = payload.clone();

    let server = thread::spawn(move || {
        // Attempt 1: send 8 KiB then go silent so the stall watchdog aborts & reconnects.
        let (mut stream1, _) = listener.accept().unwrap();
        let _req1 = read_http_request(&mut stream1);
        let hdr1 = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nETag: \"stall-v1\"\r\nContent-Length: {}\r\n\r\n",
            payload_clone.len()
        );
        stream1.write_all(hdr1.as_bytes()).unwrap();
        stream1.write_all(&payload_clone[..prefix]).unwrap();
        stream1.flush().unwrap();

        let silencer = thread::spawn(move || {
            let _ = stream1.set_read_timeout(Some(Duration::from_millis(500)));
            let mut buf = [0u8; 64];
            let _ = stream1.read(&mut buf);
        });

        // Attempt 2: accept reconnect with Range: bytes=8192- and deliver remaining payload.
        let (mut stream2, _) = listener.accept().unwrap();
        let req2 = read_http_request(&mut stream2);
        assert!(
            req2.to_ascii_lowercase()
                .contains(&format!("range: bytes={prefix}-")),
            "expected stall reconnect Range from {prefix}, got: {req2}"
        );
        let tail = &payload_clone[prefix..];
        let hdr2 = format!(
            "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nETag: \"stall-v1\"\r\nContent-Range: bytes {prefix}-{}/{}\r\nContent-Length: {}\r\n\r\n",
            payload_clone.len() - 1,
            payload_clone.len(),
            tail.len()
        );
        stream2.write_all(hdr2.as_bytes()).unwrap();
        stream2.write_all(tail).unwrap();
        stream2.flush().unwrap();
        let _ = silencer.join();
    });

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(1)
    .with_stall_watchdog(true, 1024 * 1024, Duration::from_millis(80), 12)
    .with_ssd_staging(false, false, None);

    let req = TransferRequest::new(
        "task-stall",
        "item-stall",
        &url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();

    let res = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    server.join().unwrap();

    assert_eq!(res.outcome, DownloadOutcome::Completed);
    assert!(
        transfer.stall_reconnect_count() >= 1,
        "expected >= 1 stall reconnect, got {}",
        transfer.stall_reconnect_count()
    );
    assert_eq!(fs::read(&destination).unwrap(), payload);
}

#[test]
fn ssd_staging_checks_free_space_stages_part_and_promotes_sequentially() {
    let target_dir = TempDir::new("ssd-target");
    let staging_dir = TempDir::new("ssd-stage");
    let destination = target_dir.0.join("staged_product.bin");
    let payload = deterministic_payload(96 * 1024);

    // 1) Insufficient free space in ssd_staging_dir fails cleanly before touching network.
    let low_probe = Arc::new(MockFreeSpaceProbe(Some(VolumeSpace {
        free_bytes: 1024,
        total_bytes: 1024 * 1024,
    })));
    let low_transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(2),
        chunk_size: 8 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(1)
    .with_ssd_staging(true, false, Some(staging_dir.0.clone()))
    .with_staging_free_space_probe(low_probe);

    let req_low = TransferRequest::new(
        "task-ssd-low",
        "item-ssd-low",
        "http://127.0.0.1:9/unreachable.bin",
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();

    let res_low = low_transfer.download(&req_low, &|| false, &|| false, &mut |_| {});
    assert_eq!(res_low.outcome, DownloadOutcome::Failed);
    assert!(!destination.exists());

    // 2) Auto-staging via slow target override with sufficient space downloads to staging, verifies,
    //    promotes sequentially to destination, and cleans up staged files.
    let server = RangeHttpServer::start(Arc::new(payload.clone()), "etag-ssd", true, None);
    let plenty_probe = Arc::new(MockFreeSpaceProbe(Some(VolumeSpace {
        free_bytes: 64 * 1024 * 1024 * 1024,
        total_bytes: 128 * 1024 * 1024 * 1024,
    })));

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 8 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(32 * 1024)
    .with_ssd_staging(false, true, Some(staging_dir.0.clone()))
    .with_slow_target_volume_override(Some(true))
    .with_staging_free_space_probe(plenty_probe);

    let req = TransferRequest::new(
        "task-ssd-ok",
        "item-ssd-ok",
        &server.url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();

    let res_ok = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    let _ = server.finish();

    assert_eq!(res_ok.outcome, DownloadOutcome::Completed);
    assert_eq!(transfer.ssd_staging_promotions_count(), 1);
    assert_eq!(transfer.segmented_transfers_count(), 1);
    assert_eq!(fs::read(&destination).unwrap(), payload);

    let staged_root = staging_dir.0.join(".eostage-staging");
    let staging_leftovers: Vec<_> = if staged_root.is_dir() {
        fs::read_dir(&staged_root)
            .unwrap()
            .filter_map(Result::ok)
            .collect()
    } else {
        Vec::new()
    };
    assert!(
        staging_leftovers.is_empty(),
        "staging dir must be clean after promotion, found: {staging_leftovers:?}"
    );
}

#[test]
fn transfer_engine_prefs_persistence_and_smart_concurrency_tuner() {
    let _lock = PREFS_LOCK.lock().unwrap();
    let _reset = ActivePrefsGuard;
    let dir = TempDir::new("prefs");
    let custom = TransferEnginePrefs {
        smart_concurrency: true,
        per_file_segments: 9, // clamps to max 8
        stall_reconnect_enabled: false,
        ssd_staging_enabled: true,
        ssd_staging_auto_slow_disk: false,
        ssd_staging_dir: Some(format!("  {}  ", dir.0.display())),
        ..TransferEnginePrefs::default()
    };
    let saved = save_and_activate_transfer_engine_prefs_in_dir(&dir.0, custom).unwrap();
    assert_eq!(saved.per_file_segments, 8);
    assert_eq!(
        save_and_activate_transfer_engine_prefs_in_dir(
            &dir.0,
            TransferEnginePrefs {
                per_file_segments: 7,
                ..saved.clone()
            }
        )
        .unwrap()
        .per_file_segments,
        4
    );
    let saved = save_and_activate_transfer_engine_prefs_in_dir(&dir.0, saved).unwrap();
    assert_eq!(saved.ssd_staging_dir, Some(dir.0.display().to_string()));
    assert!(!saved.stall_reconnect_enabled);
    assert!(saved.ssd_staging_enabled);

    let loaded = load_and_activate_transfer_engine_prefs_in_dir(&dir.0).unwrap();
    assert_eq!(loaded, saved);

    // Low disk (< 2 GiB) throttles to 1.
    assert_eq!(
        SmartConcurrencyTuner::recommend_concurrency(&SmartConcurrencyInputs {
            configured_concurrency: 4,
            smart_concurrency_enabled: true,
            free_space: Some(VolumeSpace {
                free_bytes: 512 * 1024 * 1024,
                total_bytes: 100 * 1024 * 1024 * 1024,
            }),
            observed_throughput_bps: 32 * 1024 * 1024,
            recent_stall_count: 0,
        }),
        1
    );

    // Moderate disk (< 10 GiB) caps at 2.
    assert_eq!(
        SmartConcurrencyTuner::recommend_concurrency(&SmartConcurrencyInputs {
            configured_concurrency: 4,
            smart_concurrency_enabled: true,
            free_space: Some(VolumeSpace {
                free_bytes: 5 * 1024 * 1024 * 1024,
                total_bytes: 100 * 1024 * 1024 * 1024,
            }),
            observed_throughput_bps: 32 * 1024 * 1024,
            recent_stall_count: 0,
        }),
        2
    );

    // Plenty of disk + 0 stalls preserves configured Tier-1 ceiling (including 8, never caps down to 4..6).
    let healthy_eight = SmartConcurrencyTuner::recommend_concurrency(&SmartConcurrencyInputs {
        configured_concurrency: 8,
        smart_concurrency_enabled: true,
        free_space: Some(VolumeSpace {
            free_bytes: 100 * 1024 * 1024 * 1024,
            total_bytes: 500 * 1024 * 1024 * 1024,
        }),
        observed_throughput_bps: 24 * 1024 * 1024,
        recent_stall_count: 0,
    });
    assert_eq!(healthy_eight, 8);

    // Healthy small configured limit stays within user's ceiling (2 -> 2).
    assert_eq!(
        SmartConcurrencyTuner::recommend_concurrency(&SmartConcurrencyInputs {
            configured_concurrency: 2,
            smart_concurrency_enabled: true,
            free_space: Some(VolumeSpace {
                free_bytes: 100 * 1024 * 1024 * 1024,
                total_bytes: 500 * 1024 * 1024 * 1024,
            }),
            observed_throughput_bps: 24 * 1024 * 1024,
            recent_stall_count: 0,
        }),
        2
    );

    // Single recent stall steps down by 1; multiple stalls halve concurrency cleanly.
    assert_eq!(
        SmartConcurrencyTuner::recommend_concurrency(&SmartConcurrencyInputs {
            configured_concurrency: 8,
            smart_concurrency_enabled: true,
            free_space: Some(VolumeSpace {
                free_bytes: 100 * 1024 * 1024 * 1024,
                total_bytes: 500 * 1024 * 1024 * 1024,
            }),
            observed_throughput_bps: 24 * 1024 * 1024,
            recent_stall_count: 1,
        }),
        7
    );
    let throttled = SmartConcurrencyTuner::recommend_concurrency(&SmartConcurrencyInputs {
        configured_concurrency: 4,
        smart_concurrency_enabled: true,
        free_space: Some(VolumeSpace {
            free_bytes: 100 * 1024 * 1024 * 1024,
            total_bytes: 500 * 1024 * 1024 * 1024,
        }),
        observed_throughput_bps: 24 * 1024 * 1024,
        recent_stall_count: 2,
    });
    assert!(throttled <= 2, "throttled={throttled}");

    // Disabled smart concurrency honors configured concurrency.
    assert_eq!(
        SmartConcurrencyTuner::recommend_concurrency(&SmartConcurrencyInputs {
            configured_concurrency: 3,
            smart_concurrency_enabled: false,
            free_space: Some(VolumeSpace {
                free_bytes: 512 * 1024 * 1024,
                total_bytes: 1024 * 1024 * 1024,
            }),
            observed_throughput_bps: 0,
            recent_stall_count: 4,
        }),
        3
    );
}

#[test]
fn stall_reconnect_disabled_returns_stream_read_error_on_io_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = read_http_request(&mut stream);
            let hdr = "HTTP/1.1 200 OK\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(hdr.as_bytes());
            let _ = stream.write_all(&[0x55_u8; 16]);
            let _ = stream.flush();
            thread::sleep(Duration::from_millis(750));
        }
    });

    let dir = TempDir::new("stall-disabled-timeout");
    let dest = dir.0.join("timeout.bin");
    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_millis(120),
        chunk_size: 4096,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(1)
    .with_stall_watchdog(false, 1024, Duration::from_millis(50), 4)
    .with_ssd_staging(false, false, None);

    let req = TransferRequest::new(
        "task-stall-off",
        "item-stall-off",
        &format!("http://{addr}/timeout.bin"),
        dest.clone(),
        Some(4096),
    )
    .unwrap();

    let started = std::time::Instant::now();
    let result = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    let elapsed = started.elapsed();
    let _ = server.join();

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.attempt_count, 1);
    assert_eq!(transfer.stall_reconnect_count(), 0);
    assert!(
        elapsed < Duration::from_millis(650),
        "disabled stall reconnect should fail on IO timeout without spinning: elapsed={elapsed:?}"
    );
    assert!(!dest.exists());
}

#[test]
fn bhoonidhi_forces_single_stream_even_when_eight_segments_requested() {
    let dir = TempDir::new("bhoonidhi-cap1");
    let dest = dir.0.join("bhoonidhi_scene.bin");
    let payload = deterministic_payload(128 * 1024);
    let server = RangeHttpServer::start(Arc::new(payload.clone()), "etag-bh1", true, None);

    let transfer = HttpTransfer::new_with_proxy_policy(
        TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(5),
            chunk_size: 8 * 1024,
            ..TransferOptions::default()
        },
        insar_core::ProxyPolicy::Direct,
    )
    .unwrap()
    .with_per_file_segments(8)
    .with_segment_threshold_bytes(32 * 1024)
    .with_ssd_staging(false, false, None);

    let asset = ResolvedAsset::new(&server.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(payload.len() as u64));
    let req = TransferRequest::new(
        "task-bh1",
        "item-bh1",
        &server.url,
        dest.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();
    let mut session = transfer.resolved_session();
    let result = transfer.download_resolved(
        &req,
        &asset,
        AuthRefreshPolicyId::BhoonidhiOAuthBearerV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    let requests = server.finish();

    let ResolvedTransferResult::Finished(done) = result else {
        panic!("expected finished result");
    };
    assert_eq!(
        done.outcome,
        DownloadOutcome::Completed,
        "failure={:?}",
        done.failure
    );
    assert_eq!(transfer.segmented_transfers_count(), 0);
    assert_eq!(fs::read(&dest).unwrap(), payload);
    assert_eq!(
        requests.len(),
        1,
        "Bhoonidhi must force single stream even when 8 requested; saw {requests:?}"
    );
}

#[test]
fn blank_ssd_staging_dir_falls_back_to_app_data_ssd_staging() {
    let _lock = PREFS_LOCK.lock().unwrap();
    let _reset = ActivePrefsGuard;

    let app_dir = TempDir::new("app-data-ssd");
    let target_dir = TempDir::new("target-ssd");
    let destination = target_dir.0.join("promoted.bin");
    let payload = deterministic_payload(64 * 1024);
    let server = RangeHttpServer::start(Arc::new(payload.clone()), "etag-default-ssd", true, None);

    let saved = save_and_activate_transfer_engine_prefs_in_dir(
        &app_dir.0,
        TransferEnginePrefs {
            ssd_staging_enabled: true,
            ssd_staging_auto_slow_disk: false,
            ssd_staging_dir: None,
            per_file_segments: 1,
            ..TransferEnginePrefs::default()
        },
    )
    .unwrap();
    assert_eq!(saved.ssd_staging_dir, None);
    let expected_default = app_dir.0.join("ssd_staging");
    assert_eq!(
        active_default_ssd_staging_dir(),
        Some(expected_default.clone())
    );

    let plenty_probe = Arc::new(MockFreeSpaceProbe(Some(VolumeSpace {
        free_bytes: 64 * 1024 * 1024 * 1024,
        total_bytes: 128 * 1024 * 1024 * 1024,
    })));
    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 8 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_staging_free_space_probe(plenty_probe);

    let req = TransferRequest::new(
        "task-default-ssd",
        "item-default-ssd",
        &server.url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();
    let res = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    let _ = server.finish();

    assert_eq!(res.outcome, DownloadOutcome::Completed);
    assert_eq!(transfer.ssd_staging_promotions_count(), 1);
    assert!(
        expected_default.join(".eostage-staging").exists(),
        "staging root under <app_data_dir>/ssd_staging must be initialized"
    );
    assert!(
        !app_dir.0.join("cache").exists(),
        "default SSD staging must NOT live under disposable cache/"
    );
    assert_eq!(fs::read(&destination).unwrap(), payload);
}

#[test]
fn dynamic_work_stealing_bisects_slow_tail_segment_and_matches_sha256() {
    let temp = TempDir::new("work-steal");
    let destination = temp.0.join("steal.bin");
    let payload = Arc::new(deterministic_payload(512 * 1024));
    let expected_sha = sha256_hex(&payload);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/steal.bin");
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = Arc::clone(&requests);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let body = Arc::clone(&payload);

    let server = thread::spawn(move || {
        let mut workers = Vec::new();
        while !stop_flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let req = read_http_request(&mut stream);
                    if req.is_empty() {
                        continue;
                    }
                    captured.lock().unwrap().push(req.clone());
                    let data = Arc::clone(&body);
                    let stop_conn = Arc::clone(&stop_flag);
                    workers.push(thread::spawn(move || {
                        let total = data.len();
                        let Some((start, end_inclusive)) = parse_range_header(&req, total) else {
                            return;
                        };
                        let slice = &data[start..=end_inclusive];
                        let hdr = format!(
                            "HTTP/1.1 206 Partial Content\r\n\
                             Connection: close\r\n\
                             Accept-Ranges: bytes\r\n\
                             ETag: \"ws-v1\"\r\n\
                             Content-Range: bytes {start}-{end_inclusive}/{total}\r\n\
                             Content-Length: {}\r\n\r\n",
                            slice.len()
                        );
                        if stream.write_all(hdr.as_bytes()).is_err() {
                            return;
                        }
                        if start == 0 {
                            // Drip segment 0 slowly after the first 16 KiB so fast peers finish
                            // their segments quickly and bisect segment 0's remaining tail.
                            let head = slice.len().min(16 * 1024);
                            if stream.write_all(&slice[..head]).is_err() {
                                return;
                            }
                            let _ = stream.flush();
                            let mut pos = head;
                            while pos < slice.len() && !stop_conn.load(Ordering::Relaxed) {
                                thread::sleep(Duration::from_millis(12));
                                let step = (slice.len() - pos).min(2048);
                                if stream.write_all(&slice[pos..pos + step]).is_err() {
                                    break;
                                }
                                let _ = stream.flush();
                                pos += step;
                            }
                        } else {
                            let _ = stream.write_all(slice);
                            let _ = stream.flush();
                        }
                    }));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => break,
            }
        }
        for w in workers {
            let _ = w.join();
        }
    });

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(10),
        chunk_size: 16 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(64 * 1024)
    .with_min_split_chunk_bytes(16 * 1024)
    .with_slow_connection_shedding(false, RelativeSpeedConfig::default())
    .with_ssd_staging(false, false, None);

    let req = TransferRequest::new(
        "task-ws",
        "item-ws",
        &url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();
    let res = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();

    assert_eq!(
        res.outcome,
        DownloadOutcome::Completed,
        "failure={:?}",
        res.failure
    );
    assert!(
        transfer.dynamic_segment_splits_count() >= 1,
        "expected >= 1 dynamic work-stealing bisection, got 0"
    );
    let captured_reqs = requests.lock().unwrap().clone();
    assert!(
        captured_reqs.iter().any(|r| {
            parse_range_header(r, payload.len())
                .is_some_and(|(start, _)| start > 0 && start < 128 * 1024)
        }),
        "expected stolen Range start strictly inside slow segment [1..128K), got: {captured_reqs:?}"
    );
    assert_eq!(sha256_hex(&fs::read(&destination).unwrap()), expected_sha);
}

#[test]
fn relative_slow_connection_shedding_drops_slow_socket_without_burning_stall_budget() {
    let temp = TempDir::new("slow-shed");
    let destination = temp.0.join("shed.bin");
    let payload = Arc::new(deterministic_payload(256 * 1024));
    let expected_sha = sha256_hex(&payload);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/shed.bin");
    let upper_hits = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let body = Arc::clone(&payload);

    let server = thread::spawn(move || {
        let mut workers = Vec::new();
        while !stop_flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let req = read_http_request(&mut stream);
                    if req.is_empty() {
                        continue;
                    }
                    let data = Arc::clone(&body);
                    let upper = Arc::clone(&upper_hits);
                    let stop_conn = Arc::clone(&stop_flag);
                    workers.push(thread::spawn(move || {
                        let total = data.len();
                        let Some((start, end_inclusive)) = parse_range_header(&req, total) else {
                            return;
                        };
                        let slice = &data[start..=end_inclusive];
                        let hdr = format!(
                            "HTTP/1.1 206 Partial Content\r\n\
                             Connection: close\r\n\
                             Accept-Ranges: bytes\r\n\
                             ETag: \"shed-v1\"\r\n\
                             Content-Range: bytes {start}-{end_inclusive}/{total}\r\n\
                             Content-Length: {}\r\n\r\n",
                            slice.len()
                        );
                        if stream.write_all(hdr.as_bytes()).is_err() {
                            return;
                        }
                        if start < 128 * 1024 {
                            // Segment 0 delivers body over >= 35ms (~3.5 MiB/s) after headers so
                            // probe pre-connect latency doesn't mask fast-peer body measurement.
                            let split = slice.len() / 2;
                            let _ = stream.write_all(&slice[..split]);
                            let _ = stream.flush();
                            thread::sleep(Duration::from_millis(35));
                            let _ = stream.write_all(&slice[split..]);
                            let _ = stream.flush();
                        } else {
                            let attempt_num = upper.fetch_add(1, Ordering::AcqRel);
                            if attempt_num == 0 {
                                // First connection on Segment 1 drips ~20 KiB/s (>4x slower).
                                let head = slice.len().min(2048);
                                if stream.write_all(&slice[..head]).is_err() {
                                    return;
                                }
                                let _ = stream.flush();
                                let mut pos = head;
                                while pos < slice.len() && !stop_conn.load(Ordering::Relaxed) {
                                    thread::sleep(Duration::from_millis(25));
                                    let step = (slice.len() - pos).min(512);
                                    if stream.write_all(&slice[pos..pos + step]).is_err() {
                                        break;
                                    }
                                    let _ = stream.flush();
                                    pos += step;
                                }
                            } else {
                                // Shed reconnect runs at full speed.
                                let _ = stream.write_all(slice);
                                let _ = stream.flush();
                            }
                        }
                    }));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => break,
            }
        }
        for w in workers {
            let _ = w.join();
        }
    });

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(10),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(2)
    .with_segment_threshold_bytes(64 * 1024)
    .with_min_split_chunk_bytes(256 * 1024)
    .with_stall_watchdog(true, 1024, Duration::from_secs(5), 2)
    .with_slow_connection_shedding(
        true,
        RelativeSpeedConfig {
            min_sample_bytes: 32 * 1024,
            grace_duration: Duration::from_millis(60),
            ratio_threshold: 4.0,
            min_remaining_secs: 0.5,
            max_sheds_per_transfer: 6,
            max_sheds_per_segment: 2,
        },
    )
    .with_ssd_staging(false, false, None);

    let req = TransferRequest::new(
        "task-shed",
        "item-shed",
        &url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();
    let res = transfer.download(&req, &|| false, &|| false, &mut |_| {});
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();

    assert_eq!(
        res.outcome,
        DownloadOutcome::Completed,
        "failure={:?}",
        res.failure
    );
    assert!(
        transfer.slow_connection_shed_count() >= 1,
        "expected >= 1 slow-connection shed"
    );
    assert_eq!(
        transfer.stall_reconnect_count(),
        0,
        "relative shedding must NOT burn fixed stall reconnect budget"
    );
    assert_eq!(sha256_hex(&fs::read(&destination).unwrap()), expected_sha);
}

#[test]
fn segmented_resume_overlap_64b_verifies_matching_bytes_and_rejects_tampered_remote() {
    let temp = TempDir::new("seg-overlap-64");
    let dest_pos = temp.0.join("pos.bin");
    let payload_orig = deterministic_payload(256 * 1024);
    let expected_sha = sha256_hex(&payload_orig);
    let shared_payload = Arc::new(Mutex::new(payload_orig.clone()));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/overlap.bin");
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = Arc::clone(&requests);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let delay_second_half = Arc::new(AtomicBool::new(true));
    let delay_flag = Arc::clone(&delay_second_half);
    let payload_srv = Arc::clone(&shared_payload);

    let server = thread::spawn(move || {
        let mut workers = Vec::new();
        while !stop_flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let req = read_http_request(&mut stream);
                    if req.is_empty() {
                        continue;
                    }
                    captured.lock().unwrap().push(req.clone());
                    let body_lock = Arc::clone(&payload_srv);
                    let delay_upper = delay_flag.load(Ordering::Acquire);
                    let delay_flag_worker = Arc::clone(&delay_flag);
                    let stop_worker = Arc::clone(&stop_flag);
                    workers.push(thread::spawn(move || {
                        let data = body_lock.lock().unwrap().clone();
                        let total = data.len();
                        let Some((start, end_inclusive)) = parse_range_header(&req, total) else {
                            return;
                        };
                        let slice = &data[start..=end_inclusive];
                        let hdr = format!(
                            "HTTP/1.1 206 Partial Content\r\n\
                             Connection: close\r\n\
                             Accept-Ranges: bytes\r\n\
                             ETag: \"ov-64\"\r\n\
                             Content-Range: bytes {start}-{end_inclusive}/{total}\r\n\
                             Content-Length: {}\r\n\r\n",
                            slice.len()
                        );
                        if delay_upper {
                            let _ = stream.write_all(hdr.as_bytes());
                            if start == 0 {
                                let head = slice.len().min(8 * 1024);
                                let _ = stream.write_all(&slice[..head]);
                            }
                            let _ = stream.flush();
                            let wait_start = std::time::Instant::now();
                            while delay_flag_worker.load(Ordering::Acquire)
                                && !stop_worker.load(Ordering::Relaxed)
                                && wait_start.elapsed() < Duration::from_secs(10)
                            {
                                thread::sleep(Duration::from_millis(5));
                            }
                            return;
                        }
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(slice);
                        let _ = stream.flush();
                    }));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => break,
            }
        }
        for w in workers {
            let _ = w.join();
        }
    });

    let pause_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let paused_observed = Arc::new(AtomicBool::new(false));

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(2)
    .with_segment_threshold_bytes(64 * 1024)
    .with_stall_watchdog(true, 1024, Duration::from_secs(5), 2)
    .with_ssd_staging(false, false, None);

    let req_pos = TransferRequest::new(
        "task-ov-pos",
        "item-ov-pos",
        &url,
        dest_pos.clone(),
        Some(payload_orig.len() as u64),
    )
    .unwrap();

    let paused_cb = Arc::clone(&paused_observed);
    let pause_trigger = Arc::clone(&pause_flag);
    let cancel_trigger = Arc::clone(&cancel_flag);
    let watcher = thread::spawn(move || {
        while !paused_cb.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(40));
        cancel_trigger.store(true, Ordering::Release);
        pause_trigger.store(false, Ordering::Release);
    });

    let res_pause = transfer.download(
        &req_pos,
        &|| cancel_flag.load(Ordering::Acquire),
        &|| pause_flag.load(Ordering::Acquire),
        &mut |upd| {
            if let insar_download::TransferUpdate::Progress(p) = upd {
                if p.bytes_completed >= 4 * 1024 && !paused_observed.swap(true, Ordering::AcqRel) {
                    pause_flag.store(true, Ordering::Release);
                }
            }
        },
    );
    watcher.join().unwrap();
    assert_eq!(res_pause.outcome, DownloadOutcome::Interrupted);

    // Save a snapshot of the paused .part + .part.meta.json for negative remote-tamper test!
    let dest_neg = temp.0.join("neg.bin");
    fs::copy(part_path(&dest_pos), part_path(&dest_neg)).unwrap();
    let pos_meta_path = PathBuf::from(format!("{}.meta.json", part_path(&dest_pos).display()));
    let neg_meta_path = PathBuf::from(format!("{}.meta.json", part_path(&dest_neg).display()));
    fs::copy(&pos_meta_path, &neg_meta_path).unwrap();
    let saved_meta_json = fs::read_to_string(&pos_meta_path).unwrap();
    let meta_val: serde_json::Value = serde_json::from_str(&saved_meta_json).unwrap();
    let first_written = meta_val["segmented"]["segments"][0]["written"]
        .as_u64()
        .unwrap();
    assert!(
        first_written >= 64,
        "expected >= 64 bytes written before pause, got {first_written}"
    );
    assert!(
        first_written < 128 * 1024,
        "expected segment 0 to pause before completion, got {first_written}"
    );
    let saved_local_neg_bytes = fs::read(part_path(&dest_neg)).unwrap();

    // Positive resume: server returns identical overlap bytes -> completes with 0 duplication.
    delay_second_half.store(false, Ordering::Release);
    requests.lock().unwrap().clear();
    let res_ok = transfer.download(&req_pos, &|| false, &|| false, &mut |_| {});
    assert_eq!(
        res_ok.outcome,
        DownloadOutcome::Completed,
        "failure={:?}",
        res_ok.failure
    );
    let resume_reqs = requests.lock().unwrap().clone();
    let expected_overlap_start = (first_written - 64) as usize;
    assert!(
        resume_reqs.iter().any(|r| {
            parse_range_header(r, payload_orig.len())
                .is_some_and(|(start, _)| start == expected_overlap_start)
        }),
        "expected resumed Range starting at exact 64B overlap offset {expected_overlap_start}, got: {resume_reqs:?}"
    );
    assert!(transfer.resume_overlap_verified_count() >= 1);
    assert_eq!(sha256_hex(&fs::read(&dest_pos).unwrap()), expected_sha);

    // Negative resume: flip 1 byte inside [first_written - 64, first_written) while preserving size & ETag.
    {
        let mut p = shared_payload.lock().unwrap();
        p[first_written as usize - 16] ^= 0xff;
    }
    let req_neg = TransferRequest::new(
        "task-ov-neg",
        "item-ov-neg",
        &url,
        dest_neg.clone(),
        Some(payload_orig.len() as u64),
    )
    .unwrap();
    let res_neg = transfer.download(&req_neg, &|| false, &|| false, &mut |_| {});
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();

    assert_eq!(res_neg.outcome, DownloadOutcome::Failed);
    assert!(!dest_neg.exists());
    assert!(
        part_path(&dest_neg).exists(),
        "local .part must be preserved on overlap mismatch"
    );
    let preserved_part = fs::read(part_path(&dest_neg)).unwrap();
    assert_eq!(
        preserved_part.len(),
        saved_local_neg_bytes.len(),
        "overlap mismatch must NEVER truncate local .part file"
    );
    assert_eq!(
        &preserved_part[..first_written as usize],
        &saved_local_neg_bytes[..first_written as usize],
        "overlap mismatch must NEVER mutate verified local segment prefix"
    );
}

#[test]
fn single_stream_opt_in_overlap_64b_verifies_and_fails_closed_on_mismatch() {
    let temp = TempDir::new("single-overlap-64");
    let payload = deterministic_payload(32 * 1024);
    let resume_at = 16 * 1024_usize;
    let overlap_start = resume_at - 64;

    // 1) Positive opt-in overlap verification
    let dest_ok = temp.0.join("single_ok.bin");
    fs::write(part_path(&dest_ok), &payload[..resume_at]).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/single_ov.bin");
    let p_clone = payload.clone();
    let srv_ok = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let req = read_http_request(&mut stream);
        assert!(
            req.to_ascii_lowercase()
                .contains(&format!("range: bytes={overlap_start}-")),
            "expected Range from {overlap_start}, got: {req}"
        );
        let body = &p_clone[overlap_start..];
        let hdr = format!(
            "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nETag: \"single-ov\"\r\nContent-Range: bytes {overlap_start}-{}/{}\r\nContent-Length: {}\r\n\r\n",
            p_clone.len() - 1,
            p_clone.len(),
            body.len()
        );
        stream.write_all(hdr.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
    });

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(1)
    .with_single_stream_resume_overlap_bytes(64)
    .with_ssd_staging(false, false, None);

    let req_ok = TransferRequest::new(
        "task-s-ov-ok",
        "item-s-ov-ok",
        &url,
        dest_ok.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();
    let res_ok = transfer.download(&req_ok, &|| false, &|| false, &mut |_| {});
    srv_ok.join().unwrap();
    assert_eq!(
        res_ok.outcome,
        DownloadOutcome::Completed,
        "failure={:?}",
        res_ok.failure
    );
    assert_eq!(transfer.resume_overlap_verified_count(), 1);
    assert_eq!(fs::read(&dest_ok).unwrap(), payload);

    // 2) Negative opt-in overlap mismatch preserves .part prefix
    let dest_bad = temp.0.join("single_bad.bin");
    fs::write(part_path(&dest_bad), &payload[..resume_at]).unwrap();
    let listener_bad = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr_bad = listener_bad.local_addr().unwrap();
    let url_bad = format!("http://{addr_bad}/single_ov_bad.bin");
    let mut bad_payload = payload.clone();
    bad_payload[overlap_start + 8] ^= 0xff;
    let srv_bad = thread::spawn(move || {
        let (mut stream, _) = listener_bad.accept().unwrap();
        let _req = read_http_request(&mut stream);
        let body = &bad_payload[overlap_start..];
        let hdr = format!(
            "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nETag: \"single-ov\"\r\nContent-Range: bytes {overlap_start}-{}/{}\r\nContent-Length: {}\r\n\r\n",
            bad_payload.len() - 1,
            bad_payload.len(),
            body.len()
        );
        stream.write_all(hdr.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
    });
    let req_bad = TransferRequest::new(
        "task-s-ov-bad",
        "item-s-ov-bad",
        &url_bad,
        dest_bad.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();
    let res_bad = transfer.download(&req_bad, &|| false, &|| false, &mut |_| {});
    srv_bad.join().unwrap();
    assert_eq!(res_bad.outcome, DownloadOutcome::Failed);
    assert!(!dest_bad.exists());
    assert_eq!(
        fs::read(part_path(&dest_bad)).unwrap(),
        payload[..resume_at],
        "mismatch must preserve local prefix untouched"
    );
}

#[test]
fn global_speed_limiter_paces_transfer_live_adjusts_and_avoids_false_stall() {
    let temp = TempDir::new("speed-limit");
    let destination = temp.0.join("paced.bin");
    let payload = Arc::new(deterministic_payload(64 * 1024));
    let expected_sha = sha256_hex(&payload);
    let server = RangeHttpServer::start(Arc::clone(&payload), "etag-paced", true, None);

    // Deliberately set speed limit (16 KiB/s) BELOW stall_min_bps (128 KiB/s)!
    let limiter = Arc::new(BandwidthLimiter::new(16 * 1024));
    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(10),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(2)
    .with_segment_threshold_bytes(16 * 1024)
    .with_bandwidth_limiter(Arc::clone(&limiter))
    .with_stall_watchdog(true, 128 * 1024, Duration::from_millis(80), 4)
    .with_ssd_staging(false, false, None);

    let req = TransferRequest::new(
        "task-limit",
        "item-limit",
        &server.url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();

    let started = std::time::Instant::now();
    let unlocked = Arc::new(AtomicBool::new(false));
    let unlocked_cb = Arc::clone(&unlocked);
    let limiter_cb = Arc::clone(&limiter);
    let first_16k_elapsed = Arc::new(Mutex::new(None::<Duration>));
    let elapsed_cb = Arc::clone(&first_16k_elapsed);

    let res = transfer.download(&req, &|| false, &|| false, &mut |upd| {
        if let insar_download::TransferUpdate::Progress(p) = upd {
            if p.bytes_completed >= 16 * 1024 && !unlocked_cb.swap(true, Ordering::AcqRel) {
                *elapsed_cb.lock().unwrap() = Some(started.elapsed());
                // Live remove speed limit mid-transfer -> remaining 48 KiB finishes immediately!
                limiter_cb.set_limit_bps(0);
            }
        }
    });
    let _ = server.finish();

    assert_eq!(
        res.outcome,
        DownloadOutcome::Completed,
        "failure={:?}",
        res.failure
    );
    assert!(unlocked.load(Ordering::Acquire));
    let gated_elapsed = first_16k_elapsed.lock().unwrap().unwrap();
    assert!(
        gated_elapsed >= Duration::from_millis(400),
        "16 KiB/s limit should pace first 16 KiB for >= 400ms, elapsed={gated_elapsed:?}"
    );
    assert_eq!(
        transfer.stall_reconnect_count(),
        0,
        "BandwidthLimiter must scale StallWatchdog down so user throttling never triggers false stall"
    );
    assert_eq!(sha256_hex(&fs::read(&destination).unwrap()), expected_sha);
}

#[test]
fn host_concurrency_tracker_steps_down_on_429_and_resumes_preallocated_part_at_cap_one() {
    let temp = TempDir::new("host-429");
    let tracker = Arc::new(HostConcurrencyTracker::with_timings(
        Duration::from_secs(5),
        Duration::from_secs(60),
        Duration::from_secs(300),
    ));

    // Part 1: server rejects with 429 when concurrent active requests > 2, succeeds at <= 2.
    let dest1 = temp.0.join("stepdown.bin");
    let payload = Arc::new(deterministic_payload(256 * 1024));
    let expected_sha = sha256_hex(&payload);
    let listener1 = TcpListener::bind("127.0.0.1:0").unwrap();
    listener1.set_nonblocking(true).unwrap();
    let addr1 = listener1.local_addr().unwrap();
    let url1_str = format!("http://{addr1}/rate_limited.bin");
    let url1 = Url::parse(&url1_str).unwrap();

    let stop1 = Arc::new(AtomicBool::new(false));
    let stop1_flag = Arc::clone(&stop1);
    let body1 = Arc::clone(&payload);
    let reqs1 = Arc::new(Mutex::new(Vec::<String>::new()));
    let reqs1_cap = Arc::clone(&reqs1);
    let srv1 = thread::spawn(move || {
        let mut workers = Vec::new();
        while !stop1_flag.load(Ordering::Relaxed) {
            match listener1.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let req = read_http_request(&mut stream);
                    if req.is_empty() {
                        continue;
                    }
                    let idx = {
                        let mut g = reqs1_cap.lock().unwrap();
                        let i = g.len();
                        g.push(req.clone());
                        i
                    };
                    let data = Arc::clone(&body1);
                    workers.push(thread::spawn(move || {
                        let total = data.len();
                        let Some((start, end_inclusive)) = parse_range_header(&req, total) else {
                            return;
                        };
                        // Reject 3rd & 4th initial parallel segment requests with HTTP 429;
                        // step-down to cap=2 lets all retry requests pass with 206.
                        let slice_len = end_inclusive - start + 1;
                        if idx < 4 && slice_len == 64 * 1024 && start >= 128 * 1024 {
                            let hdr = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                            let _ = stream.write_all(hdr.as_bytes());
                            let _ = stream.flush();
                            return;
                        }
                        let slice = &data[start..=end_inclusive];
                        let hdr = format!(
                            "HTTP/1.1 206 Partial Content\r\n\
                             Connection: close\r\n\
                             Accept-Ranges: bytes\r\n\
                             ETag: \"rl-v1\"\r\n\
                             Content-Range: bytes {start}-{end_inclusive}/{total}\r\n\
                             Content-Length: {}\r\n\r\n",
                            slice.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(slice);
                        let _ = stream.flush();
                    }));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => break,
            }
        }
        for w in workers {
            let _ = w.join();
        }
    });

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 3,
        backoff: Duration::from_millis(20),
        timeout: Duration::from_secs(5),
        chunk_size: 8 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(64 * 1024)
    .with_host_concurrency_tracker(Arc::clone(&tracker))
    .with_ssd_staging(false, false, None);

    let req1 = TransferRequest::new(
        "task-429-1",
        "item-429-1",
        &url1_str,
        dest1.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();
    let res1 = transfer.download(&req1, &|| false, &|| false, &mut |_| {});
    stop1.store(true, Ordering::Relaxed);
    srv1.join().unwrap();

    assert_eq!(
        res1.outcome,
        DownloadOutcome::Completed,
        "failure={:?}",
        res1.failure
    );
    assert_eq!(res1.attempt_count, 2);
    assert_eq!(tracker.effective_cap(&url1), Some(2));
    assert_eq!(sha256_hex(&fs::read(&dest1).unwrap()), expected_sha);

    // Part 2: resuming a pre-allocated segmented .part after learned host cap drops to 1
    // MUST still resume through the segmented worker (worker_count = 1), NOT fail with DL005 InvalidPreparation!
    let dest2 = temp.0.join("cap1_resume.bin");
    let srv2 = RangeHttpServer::start(
        Arc::clone(&payload),
        "etag-cap1",
        true,
        Some(Duration::from_millis(120)),
    );
    let url2 = Url::parse(&srv2.url).unwrap();
    let tracker2 = Arc::new(HostConcurrencyTracker::with_timings(
        Duration::from_millis(1),
        Duration::from_secs(60),
        Duration::from_secs(300),
    ));
    let transfer2 = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(64 * 1024)
    .with_host_concurrency_tracker(Arc::clone(&tracker2))
    .with_ssd_staging(false, false, None);

    let req2 = TransferRequest::new(
        "task-cap1",
        "item-cap1",
        &srv2.url,
        dest2.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();
    let pause2 = Arc::new(AtomicBool::new(false));
    let cancel2 = Arc::new(AtomicBool::new(false));
    let observed2 = Arc::new(AtomicBool::new(false));
    let obs_cb = Arc::clone(&observed2);
    let p_trig = Arc::clone(&pause2);
    let c_trig = Arc::clone(&cancel2);
    let watcher2 = thread::spawn(move || {
        while !obs_cb.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(30));
        c_trig.store(true, Ordering::Release);
        p_trig.store(false, Ordering::Release);
    });
    let r_pause = transfer2.download(
        &req2,
        &|| cancel2.load(Ordering::Acquire),
        &|| pause2.load(Ordering::Acquire),
        &mut |upd| {
            if let insar_download::TransferUpdate::Progress(p) = upd {
                if p.bytes_completed > 0 && !observed2.swap(true, Ordering::AcqRel) {
                    pause2.store(true, Ordering::Release);
                }
            }
        },
    );
    watcher2.join().unwrap();
    assert_eq!(r_pause.outcome, DownloadOutcome::Interrupted);
    assert_eq!(
        fs::metadata(part_path(&dest2)).unwrap().len(),
        payload.len() as u64
    );

    // Step host cap down to 1 before resuming pre-allocated .part
    assert_eq!(tracker2.record_rate_limit(&url2, 4), Some(2));
    thread::sleep(Duration::from_millis(5));
    assert_eq!(tracker2.record_rate_limit(&url2, 2), Some(1));
    assert_eq!(tracker2.effective_cap(&url2), Some(1));

    let r_resumed = transfer2.download(&req2, &|| false, &|| false, &mut |_| {});
    let _ = srv2.finish();
    assert_eq!(
        r_resumed.outcome,
        DownloadOutcome::Completed,
        "resuming pre-allocated segmented .part at host cap 1 must succeed: {:?}",
        r_resumed.failure
    );
    assert_eq!(sha256_hex(&fs::read(&dest2).unwrap()), expected_sha);
}

#[test]
fn throttled_pause_longer_than_stall_window_resumes_without_false_stall() {
    let temp = TempDir::new("throttled-long-pause");
    let destination = temp.0.join("paused_throttled.bin");
    let payload = Arc::new(deterministic_payload(64 * 1024));
    let expected_sha = sha256_hex(&payload);
    let server = RangeHttpServer::start(Arc::clone(&payload), "etag-tpause", true, None);

    let limiter = Arc::new(BandwidthLimiter::new(16 * 1024));
    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(2)
    .with_segment_threshold_bytes(16 * 1024)
    .with_bandwidth_limiter(Arc::clone(&limiter))
    .with_stall_watchdog(true, 128 * 1024, Duration::from_millis(80), 2)
    .with_ssd_staging(false, false, None);

    let req = TransferRequest::new(
        "task-tpause",
        "item-tpause",
        &server.url,
        destination.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();

    let paused_flag = Arc::new(AtomicBool::new(false));
    let paused_cb = Arc::clone(&paused_flag);
    let triggered = Arc::new(AtomicBool::new(false));
    let triggered_cb = Arc::clone(&triggered);
    let limiter_resume = Arc::clone(&limiter);
    let unpauser = thread::spawn(move || {
        while !triggered_cb.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
        // Hold paused for > 2.5x stall_window while workers wait inside rate limiter.
        thread::sleep(Duration::from_millis(220));
        limiter_resume.set_limit_bps(0);
        paused_cb.store(false, Ordering::Release);
    });

    let res = transfer.download(
        &req,
        &|| false,
        &|| paused_flag.load(Ordering::Acquire),
        &mut |upd| {
            if let insar_download::TransferUpdate::Progress(p) = upd {
                if p.bytes_completed >= 8 * 1024 && !triggered.swap(true, Ordering::AcqRel) {
                    paused_flag.store(true, Ordering::Release);
                }
            }
        },
    );
    unpauser.join().unwrap();
    let _ = server.finish();

    assert_eq!(
        res.outcome,
        DownloadOutcome::Completed,
        "failure={:?}",
        res.failure
    );
    assert_eq!(
        transfer.stall_reconnect_count(),
        0,
        "pause longer than stall_window must reset watchdog cleanly instead of triggering false stall"
    );
    assert_eq!(sha256_hex(&fs::read(&destination).unwrap()), expected_sha);
}

#[test]
fn preallocated_segmented_part_url_change_and_restart_without_checksum_recover_cleanly() {
    let temp = TempDir::new("prealloc-fallback");
    let dest = temp.0.join("switched_url.bin");
    let payload_a = Arc::new(deterministic_payload(128 * 1024));
    let payload_b: Arc<Vec<u8>> = Arc::new(payload_a.iter().map(|b| b ^ 0x5a).collect());
    let expected_sha_b = sha256_hex(&payload_b);

    let srv_a = RangeHttpServer::start(
        Arc::clone(&payload_a),
        "etag-url-a",
        true,
        Some(Duration::from_millis(120)),
    );
    let transfer_seg = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(2)
    .with_segment_threshold_bytes(32 * 1024)
    .with_ssd_staging(false, false, None);

    let req_a = TransferRequest::new(
        "task-sw-a",
        "item-sw-a",
        &srv_a.url,
        dest.clone(),
        Some(payload_a.len() as u64),
    )
    .unwrap();
    let pause_a = Arc::new(AtomicBool::new(false));
    let cancel_a = Arc::new(AtomicBool::new(false));
    let observed_a = Arc::new(AtomicBool::new(false));
    let obs_cb = Arc::clone(&observed_a);
    let p_trig = Arc::clone(&pause_a);
    let c_trig = Arc::clone(&cancel_a);
    let watcher_a = thread::spawn(move || {
        while !obs_cb.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(30));
        c_trig.store(true, Ordering::Release);
        p_trig.store(false, Ordering::Release);
    });
    let r_a = transfer_seg.download(
        &req_a,
        &|| cancel_a.load(Ordering::Acquire),
        &|| pause_a.load(Ordering::Acquire),
        &mut |upd| {
            if let insar_download::TransferUpdate::Progress(p) = upd {
                if p.bytes_completed > 0 && !observed_a.swap(true, Ordering::AcqRel) {
                    pause_a.store(true, Ordering::Release);
                }
            }
        },
    );
    watcher_a.join().unwrap();
    let _ = srv_a.finish();
    assert_eq!(r_a.outcome, DownloadOutcome::Interrupted);

    // Pre-allocated .part has full 128 KiB size, while completed bytes must reflect partial progress (< 128 KiB).
    assert_eq!(fs::metadata(part_path(&dest)).unwrap().len(), 128 * 1024);
    let completed = part_completed_bytes(&dest);
    assert!(
        completed > 0 && completed < 128 * 1024,
        "part_completed_bytes must return actual segmented progress, got {completed}"
    );

    // Part 1: Resuming with same URL on a RecoveredAfterRestart session WITHOUT checksum
    // MUST restart from zero instead of reusing unverified segmented metadata.
    let srv_b = RangeHttpServer::start(Arc::clone(&payload_b), "etag-url-b", true, None);
    let asset_b = ResolvedAsset::new(&srv_b.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(payload_b.len() as u64));
    let req_b = TransferRequest::new(
        "task-sw-b",
        "item-sw-b",
        &srv_b.url,
        dest.clone(),
        Some(payload_b.len() as u64),
    )
    .unwrap();
    // Single-stream transfer with new URL against leftover pre-allocated segmented .part
    // MUST recover cleanly instead of failing with DL005 InvalidPreparation.
    let transfer_single = HttpTransfer::new(TransferOptions {
        max_attempts: 2,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(1)
    .with_ssd_staging(false, false, None);
    let mut session_b = transfer_single.resolved_session();
    let ResolvedTransferResult::Finished(done_b) = transfer_single.download_resolved(
        &req_b,
        &asset_b,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_b,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected finished transfer after URL switch from pre-allocated segmented .part");
    };
    let _ = srv_b.finish();
    assert_eq!(
        done_b.outcome,
        DownloadOutcome::Completed,
        "single-stream fallback from pre-allocated .part must not hit DL005: {:?}",
        done_b.failure
    );
    assert_eq!(sha256_hex(&fs::read(&dest).unwrap()), expected_sha_b);
    // Part 2: RecoveredAfterRestart session WITHOUT checksum against a pre-allocated segmented
    // .part whose prefix was modified must ignore stale segmented meta and restart cleanly from 0.
    let dest_restart = temp.0.join("restart_nochecksum.bin");
    let srv_clean = RangeHttpServer::start(Arc::clone(&payload_a), "etag-restart", true, None);
    let corrupted_len = payload_a.len();
    let corrupted_prefix = vec![0xee_u8; corrupted_len];
    fs::write(part_path(&dest_restart), &corrupted_prefix).unwrap();
    let meta_json = format!(
        r#"{{"schema_version":1,"file_len":{corrupted_len},"mtime_secs":0,"mtime_nanos":0,"url_sha256":"{}","etag":"\"etag-restart\"","last_modified":null,"segmented":{{"total_length":{corrupted_len},"segments":[{{"start":0,"end":{half},"written":{quarter}}},{{"start":{half},"end":{corrupted_len},"written":0}}]}}}}"#,
        sha256_hex(srv_clean.url.as_bytes()),
        half = corrupted_len / 2,
        quarter = corrupted_len / 4,
    );
    let meta_path = PathBuf::from(format!("{}.meta.json", part_path(&dest_restart).display()));
    fs::write(&meta_path, meta_json).unwrap();
    let asset_restart = ResolvedAsset::new(&srv_clean.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(corrupted_len as u64));
    let req_restart = TransferRequest::new(
        "task-restart-nocs",
        "item-restart-nocs",
        &srv_clean.url,
        dest_restart.clone(),
        Some(corrupted_len as u64),
    )
    .unwrap();
    let mut session_restart = transfer_seg.resolved_session();
    let ResolvedTransferResult::Finished(done_restart) = transfer_seg.download_resolved(
        &req_restart,
        &asset_restart,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_restart,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected finished transfer on RecoveredAfterRestart without checksum");
    };
    let _ = srv_clean.finish();
    assert_eq!(done_restart.outcome, DownloadOutcome::Completed);
    assert_eq!(
        sha256_hex(&fs::read(&dest_restart).unwrap()),
        sha256_hex(&payload_a),
        "RecoveredAfterRestart without checksum must reset unverified leftover segmented .part"
    );
}

#[test]
fn segmented_resume_guard_preserves_url_switch_and_refresh_contracts_at_default_concurrency() {
    let temp = TempDir::new("seg-guard-default-conc");
    let payload_a = Arc::new(deterministic_payload(128 * 1024));
    let expected_sha = sha256_hex(&payload_a);
    let len = payload_a.len();
    let half = len / 2;
    let quarter = len / 4;

    // Default-style segmented engine (`per_file_segments = 4`, `segment_threshold_bytes = 32 KiB`).
    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(32 * 1024)
    .with_ssd_staging(false, false, None);

    // Part A: Checksum-backed pre-allocated segmented `.part` with old URL `url_a` resumed against
    // `url_b` MUST quarantine `.part` via SetAside (`.eostage-conflict-`) instead of truncating it
    // through `try_segmented_download`.
    let dest_cs = temp.0.join("url_switch_checksum.bin");
    let part_cs = part_path(&dest_cs);
    let mut prealloc_bytes = vec![0_u8; len];
    prealloc_bytes[..quarter].copy_from_slice(&payload_a[..quarter]);
    fs::write(&part_cs, &prealloc_bytes).unwrap();
    let old_url = "http://127.0.0.1:9/old-url-a";
    let meta_cs = format!(
        r#"{{"schema_version":1,"file_len":{len},"mtime_secs":0,"mtime_nanos":0,"url_sha256":"{}","etag":"\"etag-a\"","last_modified":null,"segmented":{{"total_length":{len},"segments":[{{"start":0,"end":{half},"written":{quarter}}},{{"start":{half},"end":{len},"written":0}}]}}}}"#,
        sha256_hex(old_url.as_bytes()),
    );
    fs::write(format!("{}.meta.json", part_cs.display()), meta_cs).unwrap();

    let srv_b = RangeHttpServer::start(Arc::clone(&payload_a), "etag-b", true, None);
    let asset_b = ResolvedAsset::new(&srv_b.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(len as u64))
        .with_checksum(Some(
            ResolvedChecksum::new("sha256", &expected_sha).unwrap(),
        ));
    let req_b = TransferRequest::new(
        "task-cs-url-switch",
        "item-cs-url-switch",
        &srv_b.url,
        dest_cs.clone(),
        Some(len as u64),
    )
    .unwrap();
    let mut session_cs = transfer.resolved_session();
    let ResolvedTransferResult::Finished(done_cs) = transfer.download_resolved(
        &req_b,
        &asset_b,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_cs,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected finished failure with SetAside quarantine on checksummed URL switch");
    };
    let srv_b_reqs = srv_b.finish();
    assert_eq!(done_cs.outcome, DownloadOutcome::Completed);
    assert_eq!(done_cs.attempt_count, 2);
    assert_eq!(sha256_hex(&fs::read(&dest_cs).unwrap()), expected_sha);
    assert!(
        !srv_b_reqs.is_empty(),
        "Attempt 2 cleanly downloads from byte 0 after Attempt 1 sets aside the conflict .part"
    );
    assert!(
        !part_cs.exists(),
        ".part must be finalized after Attempt 2 and old prefix moved aside on Attempt 1"
    );
    let quarantine: Vec<PathBuf> = fs::read_dir(&temp.0)
        .unwrap()
        .filter_map(|e| e.ok().map(|entry| entry.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|s| s.starts_with("url_switch_checksum.bin.part.eostage-conflict-"))
        })
        .collect();
    assert_eq!(
        quarantine.len(),
        1,
        "expected SetAside quarantine file for checksummed URL-switched segmented .part"
    );
    assert_eq!(fs::read(&quarantine[0]).unwrap(), prealloc_bytes);

    // Part B: Non-checksummed pre-allocated segmented `.part` during the same execution after a
    // signed-URL refresh returning `RefreshDecision::ResumeFrom { offset }` against a new URL
    // MUST hit `IntegrityMismatch` (`DL002`) and leave `.part` untouched (not reset via segmented).
    let transfer_ref = HttpTransfer::new(TransferOptions {
        max_attempts: 2,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(32 * 1024)
    .with_ssd_staging(false, false, None);
    let dest_ref = temp.0.join("url_switch_refresh.bin");
    let srv_ref_a = RangeHttpServer::start(
        Arc::clone(&payload_a),
        "etag-ref",
        true,
        Some(Duration::from_millis(120)),
    );
    let req_ref_a = TransferRequest::new(
        "task-ref-a",
        "item-ref-a",
        &srv_ref_a.url,
        dest_ref.clone(),
        Some(len as u64),
    )
    .unwrap();
    let asset_ref_a = ResolvedAsset::new(&srv_ref_a.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(len as u64));
    let mut session_ref = transfer_ref.resolved_session();
    let cancel_ref = Arc::new(AtomicBool::new(false));
    let obs_ref = Arc::new(AtomicBool::new(false));
    let cancel_trig = Arc::clone(&cancel_ref);
    let obs_trig = Arc::clone(&obs_ref);
    let ResolvedTransferResult::Finished(interrupted_ref) = transfer_ref.download_resolved(
        &req_ref_a,
        &asset_ref_a,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_ref,
        &|| cancel_trig.load(Ordering::Acquire),
        &|| false,
        &mut |upd| {
            if let insar_download::TransferUpdate::Progress(p) = upd {
                if p.bytes_completed > 0 && !obs_trig.swap(true, Ordering::AcqRel) {
                    cancel_trig.store(true, Ordering::Release);
                }
            }
        },
    ) else {
        panic!("expected interrupted resolved transfer");
    };
    let _ = srv_ref_a.finish();
    assert_eq!(interrupted_ref.outcome, DownloadOutcome::Interrupted);
    let completed_ref = part_completed_bytes(&dest_ref);
    assert!(completed_ref > 0 && completed_ref < len as u64);
    let part_before_refresh = fs::read(part_path(&dest_ref)).unwrap();

    let srv_ref_b = RangeHttpServer::start(Arc::clone(&payload_a), "etag-ref", true, None);
    let asset_ref_b = ResolvedAsset::new(&srv_ref_b.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(len as u64));
    let req_ref_b = TransferRequest::new(
        "task-ref-a",
        "item-ref-a",
        &srv_ref_b.url,
        dest_ref.clone(),
        Some(len as u64),
    )
    .unwrap();
    session_ref.apply_refresh_decision(RefreshDecision::ResumeFrom {
        offset: completed_ref,
    });
    let ResolvedTransferResult::Finished(done_ref) = transfer_ref.download_resolved(
        &req_ref_b,
        &asset_ref_b,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_ref,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected IntegrityMismatch failure on URL change with required_resume_offset");
    };
    let srv_ref_b_reqs = srv_ref_b.finish();
    assert_eq!(done_ref.outcome, DownloadOutcome::Failed);
    assert_eq!(done_ref.attempt_count, 2);
    assert_eq!(done_ref.failure.unwrap().code, "DL002");
    assert!(
        srv_ref_b_reqs.is_empty(),
        "must not probe fresh segmented ranges when required_resume_offset conflicts: {srv_ref_b_reqs:?}"
    );
    assert_eq!(
        fs::read(part_path(&dest_ref)).unwrap(),
        part_before_refresh,
        ".part bytes must stay untouched rather than truncated to 0"
    );
}

#[test]
fn single_stream_part_above_threshold_with_default_concurrency_resumes_and_quarantines_200() {
    let temp = TempDir::new("single-above-thresh");
    let payload = Arc::new(deterministic_payload(128 * 1024));
    let expected_sha = sha256_hex(&payload);
    let len = payload.len();
    let half = len / 2;

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(32 * 1024)
    .with_ssd_staging(false, false, None);

    // Case 1 (`206`): Single-stream checksum-backed `.part` (64 KiB of 128 KiB, >= 32 KiB threshold)
    // under default `per_file_segments = 4` MUST resume from 65536 without resetting to 0.
    let dest_resume = temp.0.join("single_resume_206.bin");
    fs::write(part_path(&dest_resume), &payload[..half]).unwrap();
    let srv_range = RangeHttpServer::start(Arc::clone(&payload), "etag-ss-206", true, None);
    let asset_range = ResolvedAsset::new(&srv_range.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(len as u64))
        .with_checksum(Some(
            ResolvedChecksum::new("sha256", &expected_sha).unwrap(),
        ));
    let req_range = TransferRequest::new(
        "task-ss-206",
        "item-ss-206",
        &srv_range.url,
        dest_resume.clone(),
        Some(len as u64),
    )
    .unwrap();
    let mut session_range = transfer.resolved_session();
    let ResolvedTransferResult::Finished(done_range) = transfer.download_resolved(
        &req_range,
        &asset_range,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_range,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected completed resume of single-stream .part");
    };
    let reqs_range = srv_range.finish();
    assert_eq!(done_range.outcome, DownloadOutcome::Completed);
    assert_eq!(sha256_hex(&fs::read(&dest_resume).unwrap()), expected_sha);
    assert_eq!(reqs_range.len(), 1);
    assert!(
        reqs_range[0]
            .to_ascii_lowercase()
            .contains(&format!("range: bytes={half}-")),
        "must resume cleanly from single-stream offset {half}, got: {}",
        reqs_range[0]
    );

    // Case 2 (`200 OK`, TASK-REG-007): When server ignores Range for a checksum-backed single-stream
    // `.part` >= threshold under `per_file_segments = 4`, MUST quarantine `.part` via SetAside
    // (`.eostage-conflict-`) instead of truncating it in `try_segmented_download`.
    let dest_200 = temp.0.join("single_quarantine_200.bin");
    fs::write(part_path(&dest_200), &payload[..half]).unwrap();
    let srv_200 = RangeHttpServer::start(Arc::clone(&payload), "etag-ss-200", false, None);
    let asset_200 = ResolvedAsset::new(&srv_200.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(len as u64))
        .with_checksum(Some(
            ResolvedChecksum::new("sha256", &expected_sha).unwrap(),
        ));
    let req_200 = TransferRequest::new(
        "task-ss-200",
        "item-ss-200",
        &srv_200.url,
        dest_200.clone(),
        Some(len as u64),
    )
    .unwrap();
    let mut session_200 = transfer.resolved_session();
    let ResolvedTransferResult::Finished(done_200) = transfer.download_resolved(
        &req_200,
        &asset_200,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_200,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected SetAside failure on 200 OK for checksummed single-stream .part");
    };
    let reqs_200 = srv_200.finish();
    assert_eq!(done_200.outcome, DownloadOutcome::Completed);
    assert_eq!(done_200.attempt_count, 2);
    assert_eq!(sha256_hex(&fs::read(&dest_200).unwrap()), expected_sha);
    assert!(reqs_200.len() >= 2);
    assert!(
        reqs_200[0]
            .to_ascii_lowercase()
            .contains(&format!("range: bytes={half}-")),
        "Attempt 1 must resume from single-stream offset {half}, got: {}",
        reqs_200[0]
    );
    assert!(!part_path(&dest_200).exists());
    let quarantined: Vec<PathBuf> = fs::read_dir(&temp.0)
        .unwrap()
        .filter_map(|e| e.ok().map(|entry| entry.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|s| s.starts_with("single_quarantine_200.bin.part.eostage-conflict-"))
        })
        .collect();
    assert_eq!(
        quarantined.len(),
        1,
        "TASK-REG-007: existing checksummed single-stream .part must be quarantined on 200 OK"
    );
    assert_eq!(fs::read(&quarantined[0]).unwrap(), &payload[..half]);
}

#[test]
fn recovered_after_restart_without_checksum_falls_back_on_416_probe() {
    let temp = TempDir::new("restart-416-fallback");
    let dest = temp.0.join("archive_restart_416.zip");
    let payload = Arc::new(deterministic_payload(128 * 1024));
    let expected_sha = sha256_hex(&payload);

    // Pre-populate an unverified `.part` without checksum -> `RecoveredAfterRestart` returns `Restart`.
    fs::write(part_path(&dest), vec![0xab_u8; 32 * 1024]).unwrap();

    // Server returns 416 for any `Range:` request (like dynamic archive generators rejecting speculative
    // segmented `Range: bytes=0-...`), and 200 OK with full payload for un-ranged GET.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let body = Arc::clone(&payload);
    let handle = thread::spawn(move || {
        while !stop_flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let req = read_http_request(&mut stream);
                    if req.is_empty() {
                        continue;
                    }
                    let has_range = req
                        .lines()
                        .any(|line| line.to_ascii_lowercase().starts_with("range:"));
                    captured.lock().unwrap().push(req);
                    if has_range {
                        let hdr = format!(
                            "HTTP/1.1 416 Range Not Satisfiable\r\n\
                             Connection: close\r\n\
                             Content-Range: bytes */{}\r\n\
                             Content-Length: 0\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                    } else {
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\n\
                             Connection: close\r\n\
                             ETag: \"archive-ok\"\r\n\
                             Content-Length: {}\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&body);
                    }
                    let _ = stream.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    let url = format!("http://{addr}/archive.zip");
    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 2,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(32 * 1024)
    .with_ssd_staging(false, false, None);
    let asset = ResolvedAsset::new(&url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(payload.len() as u64));
    let req = TransferRequest::new(
        "task-416-restart",
        "item-416-restart",
        &url,
        dest.clone(),
        Some(payload.len() as u64),
    )
    .unwrap();
    let mut session = transfer.resolved_session();
    let ResolvedTransferResult::Finished(done) = transfer.download_resolved(
        &req,
        &asset,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected clean fallback completion after 416 on Restart");
    };
    stop.store(true, Ordering::Release);
    handle.join().unwrap();

    assert_eq!(
        done.outcome,
        DownloadOutcome::Completed,
        "416 on Restart with leftover unverified .part must fall back to clean GET: {:?}",
        done.failure
    );
    assert_eq!(sha256_hex(&fs::read(&dest).unwrap()), expected_sha);
    let recorded = requests.lock().unwrap();
    assert_eq!(
        recorded.len(),
        2,
        "expected 1 ranged probe (416) + 1 clean GET (200)"
    );
}

#[test]
fn segmented_download_validates_pinned_remote_validator_and_records_session_validator() {
    let temp = TempDir::new("seg-validator-pin");
    let payload = Arc::new(deterministic_payload(128 * 1024));
    let expected_sha = sha256_hex(&payload);
    let len = payload.len();

    let transfer = HttpTransfer::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 4 * 1024,
        ..TransferOptions::default()
    })
    .unwrap()
    .with_per_file_segments(4)
    .with_segment_threshold_bytes(32 * 1024)
    .with_ssd_staging(false, false, None);

    // 1. Pinned RemoteValidator happy path (GACOS-REG-001 / TASK-REG-007): matches server ETag & size
    //    and records observed RemoteValidator onto ResolvedTransferSession.
    let dest_ok = temp.0.join("pinned_ok.bin");
    let srv_ok = RangeHttpServer::start(Arc::clone(&payload), "etag-pinned", true, None);
    let expected_validator = RemoteValidator {
        etag: Some("\"etag-pinned\"".to_owned()),
        last_modified: None,
        content_length: Some(len as u64),
    };
    let asset_ok = ResolvedAsset::new(&srv_ok.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(len as u64))
        .with_remote_validator(expected_validator.clone());
    let req_ok = TransferRequest::new(
        "task-pin-ok",
        "item-pin-ok",
        &srv_ok.url,
        dest_ok.clone(),
        Some(len as u64),
    )
    .unwrap();
    let mut session_ok = transfer.resolved_session();
    let ResolvedTransferResult::Finished(done_ok) = transfer.download_resolved(
        &req_ok,
        &asset_ok,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_ok,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected completed pinned-validator segmented transfer");
    };
    let _ = srv_ok.finish();
    assert_eq!(done_ok.outcome, DownloadOutcome::Completed);
    assert_eq!(transfer.segmented_transfers_count(), 1);
    assert_eq!(sha256_hex(&fs::read(&dest_ok).unwrap()), expected_sha);
    assert_eq!(
        session_ok.prior_response_validator(),
        Some(&expected_validator),
        "segmented download must record RemoteValidator onto session for GACOS/downstream callers"
    );

    // 2. Pinned RemoteValidator mismatch fail-closed (GACOS-REG-001 / TASK-REG-007): server returns
    //    drifted ETag on probe -> fails with DL002 before creating `.part` or final file.
    let dest_mismatch = temp.0.join("pinned_mismatch.bin");
    let srv_mismatch = RangeHttpServer::start(Arc::clone(&payload), "etag-drifted", true, None);
    let pinned_expected = RemoteValidator {
        etag: Some("\"etag-expected\"".to_owned()),
        last_modified: None,
        content_length: Some(len as u64),
    };
    let asset_mismatch = ResolvedAsset::new(&srv_mismatch.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(len as u64))
        .with_remote_validator(pinned_expected);
    let req_mismatch = TransferRequest::new(
        "task-pin-bad",
        "item-pin-bad",
        &srv_mismatch.url,
        dest_mismatch.clone(),
        Some(len as u64),
    )
    .unwrap();
    let mut session_mismatch = transfer.resolved_session();
    let ResolvedTransferResult::Finished(done_mismatch) = transfer.download_resolved(
        &req_mismatch,
        &asset_mismatch,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_mismatch,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected DL002 failure on pinned RemoteValidator mismatch");
    };
    let _ = srv_mismatch.finish();
    assert_eq!(done_mismatch.outcome, DownloadOutcome::Failed);
    assert_eq!(done_mismatch.failure.unwrap().code, "DL002");
    assert!(!dest_mismatch.exists());
    assert!(!part_path(&dest_mismatch).exists());

    // 3. AuthRejected after initial 206 probe (TASK-REG-010): request #0 succeeds with 206 + ETag,
    //    subsequent worker request receives 401 -> AuthRejected still preserves recorded validator.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let auth_addr = listener.local_addr().unwrap();
    let stop_auth = Arc::new(AtomicBool::new(false));
    let stop_auth_flag = Arc::clone(&stop_auth);
    let req_counter = Arc::new(AtomicUsize::new(0));
    let body_auth = Arc::clone(&payload);
    let auth_handle = thread::spawn(move || {
        while !stop_auth_flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let req = read_http_request(&mut stream);
                    if req.is_empty() {
                        continue;
                    }
                    let idx = req_counter.fetch_add(1, Ordering::AcqRel);
                    if idx == 0 {
                        if let Some((start, end_inc)) = parse_range_header(&req, body_auth.len()) {
                            let slice = &body_auth[start..=end_inc];
                            let hdr = format!(
                                "HTTP/1.1 206 Partial Content\r\n\
                                 Connection: close\r\n\
                                 Accept-Ranges: bytes\r\n\
                                 ETag: \"etag-auth\"\r\n\
                                 Content-Range: bytes {start}-{end_inc}/{}\r\n\
                                 Content-Length: {}\r\n\r\n",
                                body_auth.len(),
                                slice.len()
                            );
                            let _ = stream.write_all(hdr.as_bytes());
                            let _ = stream.write_all(slice);
                            let _ = stream.flush();
                            continue;
                        }
                    }
                    let hdr = "HTTP/1.1 401 Unauthorized\r\n\
                               Connection: close\r\n\
                               Content-Length: 0\r\n\r\n";
                    let _ = stream.write_all(hdr.as_bytes());
                    let _ = stream.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    let auth_url = format!("http://{auth_addr}/auth_after_probe.bin");
    let dest_auth = temp.0.join("auth_after_probe.bin");
    let asset_auth = ResolvedAsset::new(&auth_url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(len as u64));
    let req_auth = TransferRequest::new(
        "task-auth-seg",
        "item-auth-seg",
        &auth_url,
        dest_auth,
        Some(len as u64),
    )
    .unwrap();
    let mut session_auth = transfer.resolved_session();
    let auth_res = transfer.download_resolved(
        &req_auth,
        &asset_auth,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_auth,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    stop_auth.store(true, Ordering::Release);
    auth_handle.join().unwrap();
    let ResolvedTransferResult::AuthRejected(rej) = auth_res else {
        panic!("expected AuthRejected when subsequent segment returns 401");
    };
    assert_eq!(rej.status_code, 401);
    let recorded_auth_val = session_auth.prior_response_validator().expect(
        "TASK-REG-010: prior_response_validator must be recorded after probe 206 before worker 401",
    );
    assert_eq!(recorded_auth_val.etag.as_deref(), Some("\"etag-auth\""));
    assert_eq!(recorded_auth_val.content_length, Some(len as u64));

    // 4. Interrupted segmented download + resumed `existing_meta` on a fresh session both record
    //    live RemoteValidator (GACOS-REG-001 partial checkpoint + resume evidence).
    let dest_resume = temp.0.join("pinned_resume.bin");
    let srv_res = RangeHttpServer::start(
        Arc::clone(&payload),
        "etag-resume-pin",
        true,
        Some(Duration::from_millis(120)),
    );
    let resume_validator = RemoteValidator {
        etag: Some("\"etag-resume-pin\"".to_owned()),
        last_modified: None,
        content_length: Some(len as u64),
    };
    let asset_res = ResolvedAsset::new(&srv_res.url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(len as u64))
        .with_checksum(Some(
            ResolvedChecksum::new("sha256", &expected_sha).unwrap(),
        ))
        .with_remote_validator(resume_validator.clone());
    let req_res = TransferRequest::new(
        "task-pin-res",
        "item-pin-res",
        &srv_res.url,
        dest_resume.clone(),
        Some(len as u64),
    )
    .unwrap();
    let mut session_part1 = transfer.resolved_session();
    let cancel_res = Arc::new(AtomicBool::new(false));
    let seen_prog = Arc::new(AtomicBool::new(false));
    let cancel_res_cb = Arc::clone(&cancel_res);
    let seen_prog_cb = Arc::clone(&seen_prog);
    let ResolvedTransferResult::Finished(done_int) = transfer.download_resolved(
        &req_res,
        &asset_res,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_part1,
        &|| cancel_res_cb.load(Ordering::Acquire),
        &|| false,
        &mut |upd| {
            if let insar_download::TransferUpdate::Progress(p) = upd {
                if p.bytes_completed > 0 && !seen_prog_cb.swap(true, Ordering::AcqRel) {
                    cancel_res_cb.store(true, Ordering::Release);
                }
            }
        },
    ) else {
        panic!("expected Interrupted result after cancel");
    };
    assert_eq!(done_int.outcome, DownloadOutcome::Interrupted);
    assert_eq!(
        session_part1.prior_response_validator(),
        Some(&resume_validator),
        "interrupted segmented transfer must retain probe RemoteValidator on session"
    );

    let mut session_part2 = transfer.resolved_session();
    let ResolvedTransferResult::Finished(done_part2) = transfer.download_resolved(
        &req_res,
        &asset_res,
        AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        &mut session_part2,
        &|| false,
        &|| false,
        &mut |_| {},
    ) else {
        panic!("expected completed resume on fresh session");
    };
    let _ = srv_res.finish();
    assert_eq!(done_part2.outcome, DownloadOutcome::Completed);
    assert_eq!(sha256_hex(&fs::read(&dest_resume).unwrap()), expected_sha);
    assert_eq!(
        session_part2.prior_response_validator(),
        Some(&resume_validator),
        "resumed existing_meta segmented transfer on fresh session must record live RemoteValidator"
    );
}
