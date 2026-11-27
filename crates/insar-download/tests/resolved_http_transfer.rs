use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::DownloadOutcome;
use insar_download::{
    part_path, AcceptedPollBudget, AcceptedPollBudgetError, AdmittedTransferBackend,
    AuthRefreshPolicyId, CredentialMaterial, HttpTransfer, RedirectBudget, RefreshDecision,
    ResolvedAsset, ResolvedChecksum, ResolvedMethod, ResolvedTransferResult, ResponseHostClass,
    TransferAdmissionController, TransferAttemptLedger, TransferBackend, TransferOptions,
    TransferRequest,
};

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-resolved-http-{}-{nonce}-{seq}",
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
                captured.lock().unwrap().push(read_request(&mut stream));
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

fn transfer_request(url: &str, destination: &Path, expected_size: u64) -> TransferRequest {
    TransferRequest::new(
        "resolved-task",
        "resolved-item",
        url,
        destination.to_path_buf(),
        Some(expected_size),
    )
    .unwrap()
}

fn bearer_asset(url: &str, expected_size: u64) -> ResolvedAsset {
    let mut asset = ResolvedAsset::new(url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(expected_size))
        .with_checksum(Some(
            ResolvedChecksum::new(
                "sha256",
                "36bbe50ed96841d10443bcb670d6554f0a34b761be67ec9c4a8ad2c0c44ca42c",
            )
            .unwrap(),
        ));
    CredentialMaterial::bearer("RESOLVED_TEST_TOKEN")
        .unwrap()
        .apply_to(&mut asset)
        .unwrap();
    asset
}

fn bearer_asset_without_checksum(url: &str, expected_size: u64) -> ResolvedAsset {
    let mut asset = ResolvedAsset::new(url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(expected_size));
    CredentialMaterial::bearer("RESOLVED_TEST_TOKEN")
        .unwrap()
        .apply_to(&mut asset)
        .unwrap();
    asset
}

#[test]
fn production_options_bound_accepted_polling() {
    let options = TransferOptions::default();
    assert_eq!(options.max_accepted_polls, 60);
    assert_eq!(options.accepted_poll_timeout, Duration::from_secs(30 * 60));

    for invalid in [
        TransferOptions {
            max_accepted_polls: 0,
            ..options
        },
        TransferOptions {
            accepted_poll_timeout: Duration::ZERO,
            ..options
        },
        TransferOptions {
            accepted_poll_timeout: Duration::MAX,
            ..options
        },
    ] {
        assert!(HttpTransfer::new(invalid).is_err());
    }
    assert!(AcceptedPollBudget::new(1, Duration::MAX, Instant::now()).is_err());
}

#[test]
fn top_level_attempt_ledger_is_shared_and_never_resets() {
    let mut ledger = TransferAttemptLedger::new(2).unwrap();
    assert_eq!(ledger.try_claim(), Some(1));
    assert_eq!(ledger.try_claim(), Some(2));
    assert_eq!(ledger.try_claim(), None);
    assert_eq!(ledger.attempts_used(), 2);
    assert_eq!(ledger.remaining(), 0);
    assert!(TransferAttemptLedger::new(0).is_err());
}

#[test]
fn redirect_budget_allows_exactly_ten_hops() {
    let mut budget = RedirectBudget::default();
    for _ in 0..10 {
        assert!(budget.try_claim());
    }
    assert!(!budget.try_claim());
}

#[test]
fn accepted_poll_budget_has_independent_count_and_deadline() {
    let started = Instant::now();
    let mut count_limited =
        AcceptedPollBudget::new(60, Duration::from_secs(30 * 60), started).unwrap();
    for index in 0..60 {
        count_limited
            .try_claim(started + Duration::from_secs(index))
            .unwrap();
    }
    assert_eq!(
        count_limited
            .try_claim(started + Duration::from_secs(60))
            .unwrap_err(),
        AcceptedPollBudgetError::PollLimit
    );

    let mut deadline_limited =
        AcceptedPollBudget::new(60, Duration::from_secs(30 * 60), started).unwrap();
    assert_eq!(
        deadline_limited
            .try_claim(started + Duration::from_secs(30 * 60))
            .unwrap_err(),
        AcceptedPollBudgetError::Deadline
    );
}

#[test]
fn admitted_resolved_transfer_preserves_one_native_request_and_releases_its_permit() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("admitted.bin");
    let server = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"hello",
    )]);
    let transfer = transfer(1);
    let request = transfer_request(&server.url, &destination, 5);
    let asset = bearer_asset_without_checksum(&server.url, 5);
    let controller = TransferAdmissionController::compatibility();
    let admitted = AdmittedTransferBackend::new(&transfer, controller.clone());
    let mut session = admitted.resolved_session();

    let result = admitted.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result) if result.outcome == DownloadOutcome::Completed
    ));
    assert_eq!(server.finish().len(), 1);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.grants, 1);
    assert_eq!(snapshot.releases, 1);
    assert_eq!(snapshot.active_global, 0);
    assert_eq!(snapshot.queued_global, 0);
}

#[test]
fn auth_rejection_is_typed_and_preserves_the_partial_file() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("auth.bin");
    fs::write(part_path(&destination), b"ab").unwrap();
    let server = LocalServer::new(vec![response(
        "401 Unauthorized",
        &[("Content-Length", "0")],
        b"",
    )]);
    let transfer = transfer(2);
    let request = transfer_request(&server.url, &destination, 5);
    let asset = bearer_asset(&server.url, 5);
    let backend: &dyn TransferBackend = &transfer;
    let mut session = backend.resolved_session();

    let result = backend.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    let ResolvedTransferResult::AuthRejected(rejection) = result else {
        panic!("expected typed authentication rejection");
    };
    assert_eq!(rejection.status_code, 401);
    assert_eq!(rejection.response_host_class, ResponseHostClass::Unapproved);
    assert!(!rejection.refresh_candidate);
    assert_eq!(session.attempts_used(), 1);
    assert_eq!(fs::read(part_path(&destination)).unwrap(), b"ab");
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn ordinary_403_is_nonrefreshable_and_preserves_the_partial_file() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("forbidden.bin");
    fs::write(part_path(&destination), b"ab").unwrap();
    let server = LocalServer::new(vec![response(
        "403 Forbidden",
        &[("Content-Length", "0")],
        b"",
    )]);
    let transfer = transfer(1);
    let asset = bearer_asset(&server.url, 5);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &transfer_request(&server.url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    let ResolvedTransferResult::AuthRejected(rejection) = result else {
        panic!("expected typed permission rejection");
    };
    assert_eq!(rejection.status_code, 403);
    assert!(!rejection.refresh_candidate);
    assert_eq!(fs::read(part_path(&destination)).unwrap(), b"ab");
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn a_second_resolved_call_uses_only_the_shared_ledger_remainder() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("shared-ledger.bin");
    let first = LocalServer::new(vec![
        response("503 Service Unavailable", &[("Content-Length", "0")], b""),
        response("401 Unauthorized", &[("Content-Length", "0")], b""),
    ]);
    let transfer = transfer(3);
    let mut session = transfer.resolved_session();
    let first_asset = bearer_asset(&first.url, 5);
    let first_result = transfer.download_resolved(
        &transfer_request(&first.url, &destination, 5),
        &first_asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        first_result,
        ResolvedTransferResult::AuthRejected(_)
    ));
    assert_eq!(session.attempts_used(), 2);
    assert_eq!(first.finish().len(), 2);

    let second = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"abcde",
    )]);
    let second_asset = bearer_asset(&second.url, 5);
    let second_result = transfer.download_resolved(
        &transfer_request(&second.url, &destination, 5),
        &second_asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    let ResolvedTransferResult::Finished(result) = second_result else {
        panic!("remaining attempt should complete");
    };
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(result.attempt_count, 3);
    assert_eq!(session.attempts_used(), 3);
    assert_eq!(fs::read(&destination).unwrap(), b"abcde");
    assert_eq!(second.finish().len(), 1);
}

#[test]
fn resolved_runtime_headers_and_cookies_are_applied_only_at_request_time() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("runtime-access.bin");
    let server = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"abcde",
    )]);
    let transfer = transfer(1);
    let mut asset = bearer_asset(&server.url, 5);
    asset
        .insert_header("X-Resolved-Fixture", "HEADER_SECRET_VALUE")
        .unwrap();
    asset
        .insert_cookie("resolved_fixture", "COOKIE_SECRET_VALUE")
        .unwrap();
    let debug = format!("{asset:?}");
    assert!(!debug.contains("HEADER_SECRET_VALUE"));
    assert!(!debug.contains("COOKIE_SECRET_VALUE"));
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &transfer_request(&server.url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    let requests = server.finish();
    let request = requests[0].to_ascii_lowercase();
    assert!(request.contains("x-resolved-fixture: header_secret_value"));
    assert!(request.contains("cookie: resolved_fixture=cookie_secret_value"));
}

#[test]
fn resolved_runtime_material_cannot_override_transfer_control_headers() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("controlled-headers.bin");
    fs::write(part_path(&destination), b"ab").unwrap();
    let server = LocalServer::new(vec![response(
        "206 Partial Content",
        &[("Content-Length", "3"), ("Content-Range", "bytes 2-4/5")],
        b"cde",
    )]);
    let transfer = transfer(1);
    let mut asset = bearer_asset(&server.url, 5);
    for (name, value) in [
        ("Authorization", "Bearer WRONG"),
        ("Range", "bytes=0-"),
        ("If-Range", "\"wrong\""),
        ("Host", "wrong.example"),
        ("Content-Length", "999"),
    ] {
        asset.insert_header(name, value).unwrap();
    }
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &transfer_request(&server.url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    let request = server.finish()[0].to_ascii_lowercase();
    assert_eq!(request.matches("range:").count(), 1);
    assert!(request.contains("range: bytes=2-"));
    assert!(!request.contains("if-range:"));
    assert!(!request.contains("wrong.example"));
    assert!(!request.contains("content-length: 999"));
    assert!(!request.contains("bearer wrong"));
    assert!(request.contains("authorization: bearer resolved_test_token"));
}

#[test]
fn same_execution_strong_etag_sends_range_and_if_range() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("etag-resume.bin");
    let server = LocalServer::new(vec![
        response(
            "200 OK",
            &[("Content-Length", "5"), ("ETag", "\"stable-etag\"")],
            b"abc",
        ),
        response(
            "206 Partial Content",
            &[
                ("Content-Length", "2"),
                ("Content-Range", "bytes 3-4/5"),
                ("ETag", "\"stable-etag\""),
            ],
            b"de",
        ),
    ]);
    let transfer = transfer(2);
    let asset = bearer_asset(&server.url, 5);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &transfer_request(&server.url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed && result.attempt_count == 2
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"abcde");
    let requests = server.finish();
    let resumed = requests[1].to_ascii_lowercase();
    assert!(resumed.contains("range: bytes=3-"));
    assert!(resumed.contains("if-range: \"stable-etag\""));
}

#[test]
fn last_modified_with_total_length_is_an_if_range_validator() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("last-modified-resume.bin");
    let date = "Wed, 01 Jan 2025 00:00:00 GMT";
    let server = LocalServer::new(vec![
        response(
            "200 OK",
            &[("Content-Length", "5"), ("Last-Modified", date)],
            b"abc",
        ),
        response(
            "206 Partial Content",
            &[
                ("Content-Length", "2"),
                ("Content-Range", "bytes 3-4/5"),
                ("Last-Modified", date),
            ],
            b"de",
        ),
    ]);
    let transfer = transfer(2);
    let asset = bearer_asset(&server.url, 5);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &transfer_request(&server.url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    let requests = server.finish();
    assert!(requests[1]
        .to_ascii_lowercase()
        .contains("if-range: wed, 01 jan 2025 00:00:00 gmt"));
}

#[test]
fn weak_etag_alone_never_claims_if_range_identity() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("weak-etag.bin");
    let server = LocalServer::new(vec![
        response(
            "200 OK",
            &[("Content-Length", "5"), ("ETag", "W/\"weak\"")],
            b"abc",
        ),
        response(
            "206 Partial Content",
            &[
                ("Content-Length", "2"),
                ("Content-Range", "bytes 3-4/5"),
                ("ETag", "W/\"weak\""),
            ],
            b"de",
        ),
    ]);
    let transfer = transfer(2);
    let asset = bearer_asset(&server.url, 5);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &transfer_request(&server.url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    let requests = server.finish();
    let resumed = requests[1].to_ascii_lowercase();
    assert!(resumed.contains("range: bytes=3-"));
    assert!(!resumed.contains("if-range:"));
}

#[test]
fn restart_with_checksum_can_range_and_verifies_the_whole_part() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("restart-checksum.bin");
    fs::write(part_path(&destination), b"ab").unwrap();
    let server = LocalServer::new(vec![response(
        "206 Partial Content",
        &[("Content-Length", "3"), ("Content-Range", "bytes 2-4/5")],
        b"cde",
    )]);
    let transfer = transfer(1);
    let asset = bearer_asset(&server.url, 5);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &transfer_request(&server.url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"abcde");
    let requests = server.finish();
    assert!(requests[0].to_ascii_lowercase().contains("range: bytes=2-"));
    assert!(!requests[0].to_ascii_lowercase().contains("if-range:"));
}

#[test]
fn restart_without_checksum_preserves_part_until_replacement_response_begins() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("controlled-restart.bin");
    fs::write(part_path(&destination), b"ab").unwrap();
    let server = LocalServer::new(vec![response(
        "503 Service Unavailable",
        &[("Content-Length", "0")],
        b"",
    )]);
    let transfer = transfer(1);
    let asset = bearer_asset_without_checksum(&server.url, 5);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &transfer_request(&server.url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Failed
    ));
    assert_eq!(fs::read(part_path(&destination)).unwrap(), b"ab");
    let requests = server.finish();
    assert!(!requests[0].to_ascii_lowercase().contains("range:"));
}

#[test]
fn resumed_checksum_mismatch_never_promotes() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("ignored-range.bin");
    fs::write(part_path(&destination), b"ab").unwrap();
    let server = LocalServer::new(vec![response(
        // Keep checksum validation independently covered after ignored Range
        // becomes a pre-write rejection under the Owner's preservation rule.
        "206 Partial Content",
        &[("Content-Length", "3"), ("Content-Range", "bytes 2-4/5")],
        b"cdf",
    )]);
    let transfer = transfer(1);
    let asset = bearer_asset(&server.url, 5);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &transfer_request(&server.url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    let ResolvedTransferResult::Finished(result) = result else {
        panic!("checksum mismatch must be a transfer failure");
    };
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.failure.unwrap().code, "DL002");
    assert!(!destination.exists());
    assert_eq!(fs::read(part_path(&destination)).unwrap(), b"abcdf");
    let requests = server.finish();
    assert!(requests[0].to_ascii_lowercase().contains("range: bytes=2-"));
}

#[test]
fn redirects_and_accepted_polls_do_not_consume_top_level_attempts() {
    let temp = TempDirectory::new();
    let redirect_destination = temp.0.join("redirects.bin");
    let final_server = LocalServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "5")],
        b"abcde",
    )]);
    let middle_server = LocalServer::new(vec![response(
        "307 Temporary Redirect",
        &[("Location", &final_server.url), ("Content-Length", "0")],
        b"",
    )]);
    let source_server = LocalServer::new(vec![response(
        "302 Found",
        &[("Location", &middle_server.url), ("Content-Length", "0")],
        b"",
    )]);
    let redirect_transfer = transfer(1);
    let asset = bearer_asset(&source_server.url, 5);
    let mut session = redirect_transfer.resolved_session();

    let redirected = redirect_transfer.download_resolved(
        &transfer_request(&source_server.url, &redirect_destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        redirected,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed && result.attempt_count == 1
    ));
    assert_eq!(session.attempts_used(), 1);
    assert_eq!(source_server.finish().len(), 1);
    assert_eq!(middle_server.finish().len(), 1);
    assert_eq!(final_server.finish().len(), 1);

    let accepted_destination = temp.0.join("accepted.bin");
    let accepted_server = LocalServer::new(vec![
        response(
            "202 Accepted",
            &[("Retry-After", "0"), ("Content-Length", "0")],
            b"",
        ),
        response(
            "202 Accepted",
            &[("Retry-After", "0"), ("Content-Length", "0")],
            b"",
        ),
        response("200 OK", &[("Content-Length", "5")], b"abcde"),
    ]);
    let accepted_transfer = transfer(1);
    let asset = bearer_asset(&accepted_server.url, 5);
    let mut session = accepted_transfer.resolved_session();
    let accepted = accepted_transfer.download_resolved(
        &transfer_request(&accepted_server.url, &accepted_destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        accepted,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed && result.attempt_count == 1
    ));
    assert_eq!(session.attempts_used(), 1);
    assert_eq!(accepted_server.finish().len(), 3);
}

#[test]
fn resolved_transfer_source_has_no_credential_or_resolver_dependency() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source =
        fs::read_to_string(root.join("crates/insar-download/src/http_transfer.rs")).unwrap();
    for forbidden in [
        "CredentialProvider",
        "AssetResolverRegistry",
        ".resolve_frozen(",
    ] {
        assert!(
            !source.contains(forbidden),
            "transfer crossed the resolver boundary: {forbidden}"
        );
    }
}

#[test]
fn refresh_restart_decision_suppresses_range_until_replacement_stream_begins() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("refresh-restart.bin");
    fs::write(part_path(&destination), b"ab").unwrap();
    let first = LocalServer::new(vec![response(
        "401 Unauthorized",
        &[("Content-Length", "0")],
        b"",
    )]);
    let transfer = transfer(3);
    let first_asset = bearer_asset(&first.url, 5);
    let mut session = transfer.resolved_session();
    assert!(matches!(
        transfer.download_resolved(
            &transfer_request(&first.url, &destination, 5),
            &first_asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        ),
        ResolvedTransferResult::AuthRejected(_)
    ));
    assert_eq!(first.finish().len(), 1);

    session.apply_refresh_decision(RefreshDecision::RestartFromBeginning);
    let replacement = LocalServer::new(vec![
        response("503 Service Unavailable", &[("Content-Length", "0")], b""),
        response("200 OK", &[("Content-Length", "5")], b"abcde"),
    ]);
    let replacement_asset = bearer_asset(&replacement.url, 5);
    let result = transfer.download_resolved(
        &transfer_request(&replacement.url, &destination, 5),
        &replacement_asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed && result.attempt_count == 3
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"abcde");
    let requests = replacement.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| !request.to_ascii_lowercase().contains("range:")));
}

#[test]
fn refresh_resume_offset_must_match_the_current_partial_file() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("refresh-offset.bin");
    fs::write(part_path(&destination), b"ab").unwrap();
    let transfer = transfer(1);
    let url = "http://127.0.0.1:9/unreachable";
    let asset = bearer_asset(url, 5);
    let mut session = transfer.resolved_session();
    session.apply_refresh_decision(RefreshDecision::ResumeFrom { offset: 3 });

    let result = transfer.download_resolved(
        &transfer_request(url, &destination, 5),
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    let ResolvedTransferResult::Finished(result) = result else {
        panic!("offset mismatch must fail before transfer");
    };
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.attempt_count, 0);
    assert_eq!(result.failure.unwrap().code, "DL002");
    assert_eq!(fs::read(part_path(&destination)).unwrap(), b"ab");
}

#[test]
fn resolved_session_cannot_cross_item_or_destination_identity() {
    let temp = TempDirectory::new();
    let first_destination = temp.0.join("first.bin");
    let first = LocalServer::new(vec![response(
        "401 Unauthorized",
        &[("Content-Length", "0")],
        b"",
    )]);
    let transfer = transfer(2);
    let first_asset = bearer_asset(&first.url, 5);
    let mut session = transfer.resolved_session();
    assert!(matches!(
        transfer.download_resolved(
            &transfer_request(&first.url, &first_destination, 5),
            &first_asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        ),
        ResolvedTransferResult::AuthRejected(_)
    ));
    assert_eq!(first.finish().len(), 1);

    let second_destination = temp.0.join("second.bin");
    let url = "http://127.0.0.1:9/unreachable";
    let second_asset = bearer_asset(url, 5);
    let second_request =
        TransferRequest::new("other-task", "other-item", url, second_destination, Some(5)).unwrap();
    let result = transfer.download_resolved(
        &second_request,
        &second_asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    let ResolvedTransferResult::Finished(result) = result else {
        panic!("cross-item session use must fail locally");
    };
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.attempt_count, 1);
    assert_eq!(session.attempts_used(), 1);
}

#[test]
fn resolved_session_rejects_refreshed_size_or_checksum_drift_before_network() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("integrity-drift.bin");
    let first = LocalServer::new(vec![response(
        "401 Unauthorized",
        &[("Content-Length", "0")],
        b"",
    )]);
    let transfer = transfer(2);
    let first_asset = bearer_asset(&first.url, 5);
    let mut session = transfer.resolved_session();
    assert!(matches!(
        transfer.download_resolved(
            &transfer_request(&first.url, &destination, 5),
            &first_asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        ),
        ResolvedTransferResult::AuthRejected(_)
    ));
    assert_eq!(first.finish().len(), 1);

    let url = "http://127.0.0.1:9/unreachable";
    let mut drifted = bearer_asset(url, 5).with_checksum(Some(
        ResolvedChecksum::new(
            "sha256",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap(),
    ));
    CredentialMaterial::bearer("REFRESHED_TEST_TOKEN")
        .unwrap()
        .apply_to(&mut drifted)
        .unwrap();
    let result = transfer.download_resolved(
        &transfer_request(url, &destination, 5),
        &drifted,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    let ResolvedTransferResult::Finished(result) = result else {
        panic!("integrity drift must fail locally");
    };
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.attempt_count, 1);
    assert_eq!(session.attempts_used(), 1);
}

#[test]
fn transient_success_and_second_auth_rejection_share_exact_attempt_counts() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("transient-success.bin");
    let retry_server = LocalServer::new(vec![
        response("503 Service Unavailable", &[("Content-Length", "0")], b""),
        response("200 OK", &[("Content-Length", "5")], b"abcde"),
    ]);
    let retry_transfer = transfer(2);
    let retry_asset = bearer_asset(&retry_server.url, 5);
    let mut retry_session = retry_transfer.resolved_session();
    let result = retry_transfer.download_resolved(
        &transfer_request(&retry_server.url, &destination, 5),
        &retry_asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut retry_session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed && result.attempt_count == 2
    ));
    assert_eq!(retry_session.attempts_used(), 2);
    assert_eq!(retry_server.finish().len(), 2);

    let auth_destination = temp.0.join("two-auth-rejections.bin");
    let first = LocalServer::new(vec![response(
        "401 Unauthorized",
        &[("Content-Length", "0")],
        b"",
    )]);
    let auth_transfer = transfer(2);
    let mut auth_session = auth_transfer.resolved_session();
    let first_asset = bearer_asset(&first.url, 5);
    assert!(matches!(
        auth_transfer.download_resolved(
            &transfer_request(&first.url, &auth_destination, 5),
            &first_asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut auth_session,
            &|| false,
            &|| false,
            &mut |_| {},
        ),
        ResolvedTransferResult::AuthRejected(_)
    ));
    assert_eq!(first.finish().len(), 1);

    let second = LocalServer::new(vec![response(
        "403 Forbidden",
        &[("Content-Length", "0")],
        b"",
    )]);
    let second_asset = bearer_asset(&second.url, 5);
    assert!(matches!(
        auth_transfer.download_resolved(
            &transfer_request(&second.url, &auth_destination, 5),
            &second_asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut auth_session,
            &|| false,
            &|| false,
            &mut |_| {},
        ),
        ResolvedTransferResult::AuthRejected(_)
    ));
    assert_eq!(auth_session.attempts_used(), 2);
    assert_eq!(second.finish().len(), 1);
}
