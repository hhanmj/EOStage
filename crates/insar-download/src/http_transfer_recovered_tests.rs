use super::*;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "insar-verified-prefix-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn validator(etag: &str) -> RemoteValidator {
    RemoteValidator {
        etag: Some(etag.to_owned()),
        last_modified: None,
        content_length: Some(6),
    }
}
fn prefix_sha() -> String {
    let mut sha = Sha256Accumulator::default();
    sha.update(b"abc");
    sha.finalize_hex()
}
fn inputs(fixture: &Fixture, url: &str) -> (TransferRequest, ResolvedAsset) {
    let destination = fixture.0.join("result.tar");
    fs::write(part_path(&destination), b"abc").unwrap();
    (
        TransferRequest::new("result-task", "result-item", url, destination, Some(6)).unwrap(),
        ResolvedAsset::new(url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(6))
            .with_remote_validator(validator("\"version-one\"")),
    )
}
#[test]
fn verified_recovered_part_rejects_changed_evidence_without_modifying_bytes_or_session() {
    for case in 0..6 {
        let fixture = Fixture::new();
        let (request, mut asset) = inputs(&fixture, "https://example.invalid/result.tar");
        let mut prior = validator("\"version-one\"");
        let mut hash = prefix_sha();
        let mut length = 3;
        match case {
            0 => hash = "0".repeat(64),
            1 => length = 2,
            2 => prior.etag = Some("W/\"version-one\"".to_owned()),
            3 => asset = asset.with_remote_validator(validator("\"version-two\"")),
            4 => prior.content_length = Some(7),
            _ => {}
        }
        let mut session = ResolvedTransferSession::new(1).unwrap();
        assert!(session
            .resume_verified_part(&request, &asset, &prior, length, &hash, &|| case == 5)
            .is_err());
        assert_eq!(fs::read(part_path(&request.destination)).unwrap(), b"abc");
        assert!(session.identity.is_none());
        assert!(session.prior_response_validator().is_none());
        session.observe_initial_part(&request.destination);
        assert_eq!(
            inspect_resolved_for_session(&session, &request.destination, Some(6), None).unwrap(),
            FilePreparation::Restart
        );
    }
}
#[test]
fn verified_recovered_part_sends_if_range_and_rejects_changed_response_before_append() {
    for changed in [false, true] {
        let fixture = Fixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/result.tar", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "no transfer request arrived");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            // Accepted Windows sockets inherit nonblocking mode from the listener.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(8)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                headers.push_str(&line.to_ascii_lowercase());
            }
            assert!(headers.contains("range: bytes=3-\r\n"));
            assert!(headers.contains("if-range: \"version-one\"\r\n"));
            let etag = if changed {
                "version-two"
            } else {
                "version-one"
            };
            write!(reader.into_inner(), "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nETag: \"{etag}\"\r\nConnection: close\r\n\r\ndef").unwrap();
        });
        let (request, asset) = inputs(&fixture, &url);
        let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            timeout: Duration::from_secs(4),
            backoff: Duration::ZERO,
            ..TransferOptions::default()
        })
        .unwrap();
        let mut session = transfer.resolved_session();
        session
            .resume_verified_part(
                &request,
                &asset,
                &validator("\"version-one\""),
                3,
                &prefix_sha(),
                &|| false,
            )
            .unwrap();
        let result = transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );
        let ResolvedTransferResult::Finished(result) = result else {
            panic!("unexpected auth rejection")
        };
        if changed {
            assert_eq!(result.outcome, DownloadOutcome::Failed);
            assert!(!request.destination.exists());
            assert_eq!(fs::read(part_path(&request.destination)).unwrap(), b"abc");
        } else {
            assert_eq!(result.outcome, DownloadOutcome::Completed);
            assert_eq!(fs::read(&request.destination).unwrap(), b"abcdef");
        }
        server.join().unwrap();
    }
}

// Owner 2026-09-30: an abnormal remote response is not proof that local bytes
// are invalid. Exercise every shared HTTP authentication policy, not only MPC.
#[test]
fn all_resume_routes_preserve_prefix_when_range_or_access_is_rejected() {
    let policies = [
        None,
        Some(AuthRefreshPolicyId::EarthdataStandardV1),
        Some(AuthRefreshPolicyId::EarthdataBurstSessionV1),
        Some(AuthRefreshPolicyId::CdseOAuthBearerV1),
        Some(AuthRefreshPolicyId::BhoonidhiOAuthBearerV1),
        Some(AuthRefreshPolicyId::UsgsM2mSignedUrlV1),
        Some(AuthRefreshPolicyId::PlanetaryComputerSasV1),
    ];
    let responses = [
        ("416 Range Not Satisfiable", "bytes */6", ""),
        ("200 OK", "", "abcdef"),
        ("206 Partial Content", "bytes 4-5/6", "ef"),
        ("206 Partial Content", "bytes 3-2/6", "def"),
        ("206 Partial Content", "bytes 3-8/6", "def"),
        ("401 Unauthorized", "", ""),
        ("403 Forbidden", "", ""),
        ("404 Not Found", "", ""),
    ];
    let mut failures = Vec::new();
    for policy in policies {
        for (status, range, body) in responses {
            let fixture = Fixture::new();
            let response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nETag: \"version-one\"\r\n{}Connection: close\r\n\r\n{body}",
                body.len(), if range.is_empty() {String::new()} else {format!("Content-Range: {range}\r\n")});
            let (url, server) = one_response(response);
            let (request, asset) = inputs(&fixture, &url);
            let transfer = checkpoint_transfer();
            let failed = if let Some(policy) = policy {
                let mut session = transfer.resolved_session();
                session
                    .resume_verified_part(
                        &request,
                        &asset,
                        &validator("\"version-one\""),
                        3,
                        &prefix_sha(),
                        &|| false,
                    )
                    .unwrap();
                match transfer.download_resolved(
                    &request,
                    &asset,
                    policy,
                    &mut session,
                    &|| false,
                    &|| false,
                    &mut |_| {},
                ) {
                    ResolvedTransferResult::Finished(result) => {
                        result.outcome == DownloadOutcome::Failed
                    }
                    ResolvedTransferResult::AuthRejected(_) => true,
                }
            } else {
                transfer
                    .download(&request, &|| false, &|| false, &mut |_| {})
                    .outcome
                    == DownloadOutcome::Failed
            };
            let headers = server.join().unwrap();
            if !failed
                || request.destination.exists()
                || fs::read(part_path(&request.destination)).ok().as_deref() != Some(b"abc")
                || !headers.contains("range: bytes=3-")
            {
                failures.push(format!("{policy:?}: {status} {range}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "remote rejection destroyed/replaced accepted prefixes: {failures:?}"
    );
}

fn one_response(response: String) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/result.tar", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(8);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "expected local request missing");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        };
        // Accepted Windows sockets inherit nonblocking mode from the listener.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            request.push_str(&line);
        }
        reader.into_inner().write_all(response.as_bytes()).unwrap();
        request.to_ascii_lowercase()
    });
    (url, worker)
}
fn checkpoint_transfer() -> HttpTransfer {
    HttpTransfer::new_without_system_proxy(TransferOptions {
        chunk_size: 3,
        max_attempts: 1,
        timeout: Duration::from_secs(4),
        backoff: Duration::ZERO,
        ..TransferOptions::default()
    })
    .unwrap()
}
fn checkpoint_request(fixture: &Fixture, url: &str) -> (TransferRequest, ResolvedAsset) {
    (
        TransferRequest::new(
            "saved-task",
            "saved-item",
            url,
            fixture.0.join("result.tar"),
            Some(6),
        )
        .unwrap(),
        ResolvedAsset::new(url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(6)),
    )
}
fn stored_path(fixture: &Fixture) -> PathBuf {
    fixture.0.join("result.tar.part.resume.json")
}

#[test]
fn periodic_checkpoint_survives_crash_and_refetches_unverified_tail() {
    let fixture = Fixture::new();
    let mut transfer = checkpoint_transfer();
    transfer.checkpoint_interval_bytes = Some(3);
    let (url, first) = one_response("HTTP/1.1 200 OK\r\nContent-Length: 6\r\nETag: \"version-one\"\r\nConnection: close\r\n\r\nabcdef".into());
    let (request, asset) = checkpoint_request(&fixture, &url);
    let mut session = transfer.resolved_session();
    session.set_recovery_identity("a".repeat(64));
    let crash = std::panic::catch_unwind(AssertUnwindSafe(|| {
        transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::PlanetaryComputerSasV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |update| {
                if let TransferUpdate::Progress(p) = update {
                    if p.bytes_completed == 3 {
                        let saved: serde_json::Value =
                            serde_json::from_slice(&fs::read(stored_path(&fixture)).unwrap())
                                .unwrap();
                        assert_eq!(saved["bytes"], 3);
                        assert_eq!(saved["prefix_sha256"], prefix_sha());
                        assert_eq!(fs::read(part_path(&request.destination)).unwrap(), b"abc");
                        panic!("simulated crash");
                    }
                }
            },
        );
    }));
    assert!(crash.is_err());
    first.join().unwrap();
    fs::write(part_path(&request.destination), b"abcXY").unwrap();
    let (url, second) = one_response("HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nETag: \"version-one\"\r\nConnection: close\r\n\r\ndef".into());
    let (request, asset) = checkpoint_request(&fixture, &url);
    let mut reopened = transfer.resolved_session();
    reopened.set_recovery_identity("a".repeat(64));
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::PlanetaryComputerSasV1,
        &mut reopened,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        matches!(
            result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                ..
            })
        ),
        "{result:?}"
    );
    let headers = second.join().unwrap();
    assert!(headers.contains("range: bytes=3-\r\n"));
    assert!(headers.contains("if-range: \"version-one\"\r\n"));
    let data = fs::read(&request.destination).unwrap();
    assert_eq!(data, b"abcdef");
    let mut digest = Sha256Accumulator::default();
    digest.update(&data);
    assert_eq!(
        digest.finalize_hex(),
        "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721"
    );
    assert!(!stored_path(&fixture).exists());
}

#[test]
fn periodic_checkpoint_failure_does_not_interrupt_body() {
    let fixture = Fixture::new();
    let mut transfer = checkpoint_transfer();
    transfer.checkpoint_interval_bytes = Some(3);
    let (url, server) = one_response("HTTP/1.1 200 OK\r\nContent-Length: 6\r\nETag: \"version-one\"\r\nConnection: close\r\n\r\nabcdef".into());
    let (request, asset) = checkpoint_request(&fixture, &url);
    let mut session = transfer.resolved_session();
    session.set_recovery_identity("a".repeat(64));
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::PlanetaryComputerSasV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |update| {
            if matches!(update, TransferUpdate::BodyStarted { .. }) {
                fs::write(stored_path(&fixture), b"bad checkpoint").unwrap();
            }
            if matches!(
                update,
                TransferUpdate::Progress(TransferProgress {
                    bytes_completed: 3,
                    ..
                })
            ) {
                assert_eq!(fs::read(stored_path(&fixture)).unwrap(), b"bad checkpoint");
            }
        },
    );
    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(TransferResult {
            outcome: DownloadOutcome::Completed,
            ..
        })
    ));
    assert_eq!(fs::read(&request.destination).unwrap(), b"abcdef");
    assert_eq!(fs::read(stored_path(&fixture)).unwrap(), b"bad checkpoint");
    server.join().unwrap();
}

#[test]
fn checkpoint_left_after_promotion_is_cleared_on_restart() {
    for policy in [
        AuthRefreshPolicyId::PlanetaryComputerSasV1,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
    ] {
        let fixture = Fixture::new();
        let destination = fixture.0.join("result.tar");
        fs::write(part_path(&destination), b"abc").unwrap();
        PartCheckpoint::save(
            &destination,
            &"a".repeat(64),
            &validator("\"version-one\""),
            3,
            prefix_sha(),
        )
        .unwrap();
        fs::write(&destination, b"abcdef").unwrap();
        fs::remove_file(part_path(&destination)).unwrap();
        let (request, asset) = checkpoint_request(&fixture, "http://127.0.0.1:1/result.tar");
        let transfer = checkpoint_transfer();
        let mut session = transfer.resolved_session();
        session.set_recovery_identity("a".repeat(64));
        let result = transfer.download_resolved(
            &request,
            &asset,
            policy,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );
        assert!(
            matches!(
                result,
                ResolvedTransferResult::Finished(TransferResult {
                    outcome: DownloadOutcome::Skipped,
                    ..
                })
            ),
            "{result:?}"
        );
        assert!(!stored_path(&fixture).exists());
        assert_eq!(fs::read(&destination).unwrap(), b"abcdef");
        assert_eq!(session.attempts_used(), 0);
    }
}

#[test]
fn resolved_part_checkpoint_rejects_short_or_corrupt_prefix_without_truncation() {
    for bytes in [b"ab".as_slice(), b"abXtail".as_slice()] {
        let fixture = Fixture::new();
        let destination = fixture.0.join("result.tar");
        let binding = "a".repeat(64);
        fs::write(part_path(&destination), b"abc").unwrap();
        PartCheckpoint::save(
            &destination,
            &binding,
            &validator("\"version-one\""),
            3,
            prefix_sha(),
        )
        .unwrap();
        fs::write(part_path(&destination), bytes).unwrap();
        assert!(PartCheckpoint::load(&destination, &binding, Some(6), &|| false).is_err());
        assert_eq!(fs::read(part_path(&destination)).unwrap(), bytes);
        assert!(stored_path(&fixture).exists());
    }
}

#[test]
fn persisted_mpc_prefix_resumes_after_new_session_and_changed_access_url() {
    let fixture = Fixture::new();
    let transfer = checkpoint_transfer();
    let (url, first) = one_response("HTTP/1.1 200 OK\r\nContent-Length: 6\r\nETag: \"version-one\"\r\nConnection: close\r\n\r\nabcdef".into());
    let (request, asset) = checkpoint_request(&fixture, &format!("{url}?sig=TEST_FIRST_ACCESS"));
    let mut session = transfer.resolved_session();
    session.set_recovery_identity("a".repeat(64));
    let cancel = AtomicBool::new(false);
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::PlanetaryComputerSasV1,
        &mut session,
        &|| cancel.load(Ordering::SeqCst),
        &|| false,
        &mut |update| {
            if let TransferUpdate::Progress(p) = update {
                if p.bytes_completed == 3 {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        },
    );
    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(TransferResult {
            outcome: DownloadOutcome::Interrupted,
            ..
        })
    ));
    assert_eq!(fs::read(part_path(&request.destination)).unwrap(), b"abc");
    assert!(!first.join().unwrap().contains("range:"));
    let stored = fs::read_to_string(stored_path(&fixture)).unwrap();
    assert!(!stored.contains("TEST_FIRST_ACCESS"));
    assert!(!stored.contains("http"));
    drop(session);
    let (url, second) = one_response("HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nETag: \"version-one\"\r\nConnection: close\r\n\r\ndef".into());
    let (request, asset) = checkpoint_request(&fixture, &format!("{url}?sig=TEST_SECOND_ACCESS"));
    let mut reopened = transfer.resolved_session();
    reopened.set_recovery_identity("a".repeat(64));
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::PlanetaryComputerSasV1,
        &mut reopened,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        matches!(
            result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                ..
            })
        ),
        "{result:?}"
    );
    let headers = second.join().unwrap();
    assert!(headers.contains("range: bytes=3-\r\n"));
    assert!(headers.contains("if-range: \"version-one\"\r\n"));
    assert_eq!(fs::read(&request.destination).unwrap(), b"abcdef");
    assert!(!stored_path(&fixture).exists());
    assert!(!part_path(&request.destination).exists());
}

#[test]
fn persisted_mpc_prefix_rejects_changed_weak_missing_validator_and_ignored_range() {
    for (status, etag, range, body) in [
        ("206 Partial Content", "\"changed\"", "bytes 3-5/6", "def"),
        (
            "206 Partial Content",
            "W/\"version-one\"",
            "bytes 3-5/6",
            "def",
        ),
        ("206 Partial Content", "", "bytes 3-5/6", "def"),
        (
            "206 Partial Content",
            "\"version-one\"",
            "bytes 2-4/6",
            "def",
        ),
        (
            "206 Partial Content",
            "\"version-one\"",
            "bytes 3-4/6",
            "def",
        ),
        (
            "206 Partial Content",
            "\"version-one\"",
            "bytes 3-5/*",
            "def",
        ),
        ("206 Partial Content", "\"version-one\"", "invalid", "def"),
        ("200 OK", "\"version-one\"", "", "abcdef"),
        (
            "416 Range Not Satisfiable",
            "\"version-one\"",
            "bytes */6",
            "",
        ),
    ] {
        let fixture = Fixture::new();
        let destination = fixture.0.join("result.tar");
        fs::write(part_path(&destination), b"abc").unwrap();
        PartCheckpoint::save(
            &destination,
            &"a".repeat(64),
            &validator("\"version-one\""),
            3,
            prefix_sha(),
        )
        .unwrap();
        let before = fs::read(stored_path(&fixture)).unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{}{}Connection: close\r\n\r\n{body}",
            body.len(),
            if etag.is_empty() {
                String::new()
            } else {
                format!("ETag: {etag}\r\n")
            },
            if range.is_empty() {
                String::new()
            } else {
                format!("Content-Range: {range}\r\n")
            }
        );
        let (url, server) = one_response(response);
        let (request, asset) = checkpoint_request(&fixture, &url);
        let transfer = checkpoint_transfer();
        let mut session = transfer.resolved_session();
        session.set_recovery_identity("a".repeat(64));
        let result = transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::PlanetaryComputerSasV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );
        assert!(
            matches!(
                result,
                ResolvedTransferResult::Finished(TransferResult {
                    outcome: DownloadOutcome::Failed,
                    ..
                })
            ),
            "{result:?}"
        );
        assert!(server.join().unwrap().contains("range: bytes=3-"));
        assert_eq!(fs::read(part_path(&destination)).unwrap(), b"abc");
        assert_eq!(fs::read(stored_path(&fixture)).unwrap(), before);
        assert!(!destination.exists());
    }
}

#[test]
fn persisted_mpc_prefix_rejects_corrupt_cross_task_and_hardlinked_local_evidence_before_io() {
    for mutation in 0..5 {
        let fixture = Fixture::new();
        let destination = fixture.0.join("result.tar");
        fs::write(part_path(&destination), b"abc").unwrap();
        PartCheckpoint::save(
            &destination,
            &"a".repeat(64),
            &validator("\"version-one\""),
            3,
            prefix_sha(),
        )
        .unwrap();
        match mutation {
            0 => fs::write(part_path(&destination), b"abx").unwrap(),
            1 => fs::write(stored_path(&fixture), b"corrupt").unwrap(),
            2 => {}
            3 => fs::hard_link(part_path(&destination), fixture.0.join("other-part")).unwrap(),
            _ => fs::hard_link(stored_path(&fixture), fixture.0.join("other-record")).unwrap(),
        }
        let before = fs::read(part_path(&destination)).unwrap();
        let record = fs::read(stored_path(&fixture)).unwrap();
        let (request, asset) = checkpoint_request(&fixture, "http://127.0.0.1:1/result.tar");
        let transfer = checkpoint_transfer();
        let mut session = transfer.resolved_session();
        session.set_recovery_identity(if mutation == 2 { "b" } else { "a" }.repeat(64));
        let result = transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::PlanetaryComputerSasV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );
        assert_eq!(session.attempts_used(), 0);
        assert!(matches!(
            result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Failed,
                ..
            })
        ));
        assert_eq!(fs::read(part_path(&destination)).unwrap(), before);
        assert_eq!(fs::read(stored_path(&fixture)).unwrap(), record);
    }
}

#[test]
fn persisted_prefix_survives_auth_rejection_and_the_same_session_refresh() {
    let fixture = Fixture::new();
    let destination = fixture.0.join("result.tar");
    fs::write(part_path(&destination), b"abc").unwrap();
    PartCheckpoint::save(
        &destination,
        &"a".repeat(64),
        &validator("\"version-one\""),
        3,
        prefix_sha(),
    )
    .unwrap();
    let before = fs::read(stored_path(&fixture)).unwrap();
    let (url, denied) = one_response(
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
    );
    let (request, asset) = checkpoint_request(&fixture, &url);
    let transfer = checkpoint_transfer();
    let mut session = ResolvedTransferSession::new(2).unwrap();
    session.set_recovery_identity("a".repeat(64));
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::PlanetaryComputerSasV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(result, ResolvedTransferResult::AuthRejected(_)));
    assert!(denied.join().unwrap().contains("range: bytes=3-"));
    assert_eq!(fs::read(stored_path(&fixture)).unwrap(), before);
    session.apply_refresh_decision(RefreshDecision::RestartFromBeginning);
    let (url, accepted) = one_response("HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nETag: \"version-one\"\r\nConnection: close\r\n\r\ndef".into());
    let (request, asset) = checkpoint_request(&fixture, &url);
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::PlanetaryComputerSasV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        matches!(
            result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                ..
            })
        ),
        "{result:?}"
    );
    let headers = accepted.join().unwrap();
    assert!(headers.contains("range: bytes=3-"));
    assert!(headers.contains("if-range: \"version-one\""));
    assert_eq!(session.attempts_used(), 2);
    assert_eq!(fs::read(destination).unwrap(), b"abcdef");
}

#[test]
fn checkpoint_updates_only_its_own_interrupted_prefix_and_preserves_conflicts() {
    let fixture = Fixture::new();
    let destination = fixture.0.join("result.tar");
    let binding = "a".repeat(64);
    fs::write(part_path(&destination), b"abc").unwrap();
    PartCheckpoint::save(
        &destination,
        &binding,
        &validator("\"version-one\""),
        3,
        prefix_sha(),
    )
    .unwrap();
    fs::write(part_path(&destination), b"abcd").unwrap();
    let mut hash = Sha256Accumulator::default();
    hash.update(b"abcd");
    let hash = hash.finalize_hex();
    PartCheckpoint::save(
        &destination,
        &binding,
        &validator("\"version-one\""),
        4,
        hash.clone(),
    )
    .unwrap();
    let saved = PartCheckpoint::load(&destination, &binding, Some(6), &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(saved.bytes, 4);
    assert_eq!(saved.prefix_sha256, hash);
    let before = fs::read(stored_path(&fixture)).unwrap();
    assert!(PartCheckpoint::save(
        &destination,
        &"b".repeat(64),
        &validator("\"version-one\""),
        4,
        hash.clone()
    )
    .is_err());
    assert!(
        PartCheckpoint::save(&destination, &binding, &validator("\"changed\""), 4, hash).is_err()
    );
    assert_eq!(fs::read(stored_path(&fixture)).unwrap(), before);
    assert_eq!(fs::read(part_path(&destination)).unwrap(), b"abcd");
}

// B13: a USGS official-archive transfer restarts the app mid-download. The
// M2M resolve then returns a newly signed dds.cr.usgs.gov object URL, so the
// durable prefix evidence must bind to the frozen locator, never the URL, and
// the re-resolved object must still prove itself with a matching strong ETag
// and an exact 206 Content-Range before any byte is appended.
#[test]
fn persisted_usgs_prefix_resumes_across_restart_with_a_new_signed_url() {
    let fixture = Fixture::new();
    let transfer = checkpoint_transfer();
    let binding = "a".repeat(64);

    // First execution: 3 of 6 bytes land, then the app is "restarted" (the
    // transfer is cancelled; the prefix and its checkpoint record remain).
    let (first_url, first) = one_response(
        "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nETag: \"version-one\"\r\nConnection: close\r\n\r\nabcdef"
            .into(),
    );
    let (request, asset) = checkpoint_request(&fixture, &first_url);
    let mut session = transfer.resolved_session();
    session.set_recovery_identity(binding.clone());
    let cancel = AtomicBool::new(false);
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        &mut session,
        &|| cancel.load(Ordering::SeqCst),
        &|| false,
        &mut |update| {
            if let TransferUpdate::Progress(progress) = update {
                if progress.bytes_completed == 3 {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        },
    );
    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(TransferResult {
            outcome: DownloadOutcome::Interrupted,
            ..
        })
    ));
    assert_eq!(fs::read(part_path(&request.destination)).unwrap(), b"abc");
    assert!(!first.join().unwrap().contains("range:"));
    assert!(
        stored_path(&fixture).exists(),
        "the interrupted prefix must leave durable evidence"
    );
    drop(session);

    // After the restart the item is re-resolved: same object, new signed URL
    // (different host spelling, fresh signature). The server honours Range
    // with a matching strong ETag, exactly as recorded for dds.cr.usgs.gov
    // object downloads.
    let (second_url, second) = one_response(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nETag: \"version-one\"\r\nConnection: close\r\n\r\ndef"
            .into(),
    );
    let (request, asset) = checkpoint_request(&fixture, &second_url);
    let mut reopened = transfer.resolved_session();
    reopened.set_recovery_identity(binding.clone());
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        &mut reopened,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        matches!(
            result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                ..
            })
        ),
        "{result:?}"
    );
    let headers = second.join().unwrap();
    assert!(
        headers.contains("range: bytes=3-\r\n"),
        "the restart must resume, not restart from zero: {headers}"
    );
    assert!(headers.contains("if-range: \"version-one\"\r\n"));
    assert_eq!(fs::read(&request.destination).unwrap(), b"abcdef");
    assert!(!stored_path(&fixture).exists());
    assert!(!part_path(&request.destination).exists());
}

#[test]
fn persisted_usgs_prefix_refuses_a_re_resolved_object_that_changed() {
    for (status, etag, range, body) in [
        // A different object generation behind the same locator: the strong
        // ETag no longer matches the checkpoint's prefix evidence.
        ("206 Partial Content", "\"changed\"", "bytes 3-5/6", "def"),
        // The re-resolved object is served fresh (200 ignoring Range): not
        // authority to splice onto the old prefix.
        ("200 OK", "\"version-one\"", "", "abcdef"),
        // The remote lost the object: no bytes may be trusted or written.
        ("404 Not Found", "\"version-one\"", "", ""),
    ] {
        let fixture = Fixture::new();
        let destination = fixture.0.join("result.tar");
        fs::write(part_path(&destination), b"abc").unwrap();
        PartCheckpoint::save(
            &destination,
            &"a".repeat(64),
            &validator("\"version-one\""),
            3,
            prefix_sha(),
        )
        .unwrap();
        let before = fs::read(part_path(&destination)).unwrap();
        let record = fs::read(stored_path(&fixture)).unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{}{}Connection: close\r\n\r\n{body}",
            body.len(),
            if etag.is_empty() {
                String::new()
            } else {
                format!("ETag: {etag}\r\n")
            },
            if range.is_empty() {
                String::new()
            } else {
                format!("Content-Range: {range}\r\n")
            }
        );
        let (url, server) = one_response(response);
        let (request, asset) = checkpoint_request(&fixture, &url);
        let transfer = checkpoint_transfer();
        let mut session = transfer.resolved_session();
        session.set_recovery_identity("a".repeat(64));
        let result = transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );
        assert!(
            !matches!(
                &result,
                ResolvedTransferResult::Finished(TransferResult {
                    outcome: DownloadOutcome::Completed,
                    ..
                })
            ),
            "a changed re-resolved object must never complete from the old prefix: {status}"
        );
        let headers = server.join().unwrap();
        if status != "404 Not Found" {
            assert!(
                headers.contains("range: bytes=3-"),
                "the prefix is still probed, never silently discarded"
            );
        }
        assert_eq!(
            fs::read(part_path(&destination)).unwrap(),
            before,
            "no byte of the old prefix is modified"
        );
        assert_eq!(
            fs::read(stored_path(&fixture)).unwrap(),
            record,
            "the checkpoint record is preserved for the next attempt"
        );
        assert!(
            !destination.exists(),
            "no finished file is fabricated from mismatched generations"
        );
    }
}

// B13 live evidence: dds.cr.usgs.gov object responses may carry no strong
// ETag at all. A checkpoint bound only to a strong ETag can then never be
// saved, and the restart discards the prefix. The schema-2 checkpoint also
// accepts the validator pair `If-Range` itself accepts: Last-Modified plus
// the exact total.
#[test]
fn persisted_usgs_prefix_resumes_across_restart_with_last_modified_only() {
    let fixture = Fixture::new();
    let transfer = checkpoint_transfer();
    let binding = "a".repeat(64);
    let last_modified = "Wed, 21 Oct 2026 07:28:00 GMT";

    // First execution: 3 of 6 bytes land, then the app is "restarted". The
    // server response carries Last-Modified and no ETag.
    let (first_url, first) = one_response(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nLast-Modified: {last_modified}\r\nConnection: close\r\n\r\nabcdef"
        ),
    );
    let (request, asset) = checkpoint_request(&fixture, &first_url);
    let mut session = transfer.resolved_session();
    session.set_recovery_identity(binding.clone());
    let cancel = AtomicBool::new(false);
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        &mut session,
        &|| cancel.load(Ordering::SeqCst),
        &|| false,
        &mut |update| {
            if let TransferUpdate::Progress(progress) = update {
                if progress.bytes_completed == 3 {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        },
    );
    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(TransferResult {
            outcome: DownloadOutcome::Interrupted,
            ..
        })
    ));
    assert_eq!(fs::read(part_path(&request.destination)).unwrap(), b"abc");
    assert!(
        stored_path(&fixture).exists(),
        "a Last-Modified response must still leave durable evidence"
    );
    drop(session);

    // After the restart the item re-resolves: same object, new signed URL,
    // same Last-Modified, still no ETag. The server honours Range.
    let (second_url, second) = one_response(
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nLast-Modified: {last_modified}\r\nConnection: close\r\n\r\ndef"
        ),
    );
    let (request, asset) = checkpoint_request(&fixture, &second_url);
    let mut reopened = transfer.resolved_session();
    reopened.set_recovery_identity(binding.clone());
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        &mut reopened,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        matches!(
            result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                ..
            })
        ),
        "{result:?}"
    );
    let headers = second.join().unwrap();
    assert!(
        headers.contains("range: bytes=3-\r\n"),
        "the restart must resume, not restart from zero: {headers}"
    );
    // one_response lowercases the captured request; the probe carries the
    // Last-Modified validator.
    assert!(
        headers.contains("if-range: wed, 21 oct 2026 07:28:00 gmt\r\n"),
        "the probe must carry the Last-Modified validator: {headers}"
    );
    assert_eq!(fs::read(&request.destination).unwrap(), b"abcdef");
    assert!(!stored_path(&fixture).exists());
    assert!(!part_path(&request.destination).exists());
    let _ = first.join();
}

#[test]
fn persisted_usgs_prefix_refuses_a_changed_last_modified() {
    // A re-resolved object whose Last-Modified no longer matches the
    // checkpoint's evidence is a different generation: never spliced.
    let fixture = Fixture::new();
    let destination = fixture.0.join("result.tar");
    fs::write(part_path(&destination), b"abc").unwrap();
    let mut validator = validator("\"version-one\"");
    validator.etag = None;
    validator.last_modified = Some("Wed, 21 Oct 2026 07:28:00 GMT".to_owned());
    PartCheckpoint::save(&destination, &"a".repeat(64), &validator, 3, prefix_sha()).unwrap();
    let before = fs::read(part_path(&destination)).unwrap();
    let (url, server) = one_response(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nLast-Modified: Thu, 22 Oct 2026 07:28:00 GMT\r\nConnection: close\r\n\r\ndef".into(),
    );
    let (request, asset) = checkpoint_request(&fixture, &url);
    let transfer = checkpoint_transfer();
    let mut session = transfer.resolved_session();
    session.set_recovery_identity("a".repeat(64));
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        !matches!(
            &result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                ..
            })
        ),
        "a changed Last-Modified must never complete from the old prefix"
    );
    assert!(server.join().unwrap().contains("range: bytes=3-"));
    assert_eq!(fs::read(part_path(&destination)).unwrap(), before);
    assert!(!destination.exists());
}

// B13 follow-up (live 2026-10-31): a checkpoint whose binding does not match
// the current task's frozen identity is not authority to fail the USGS
// archive task. The evidence (both `.part` and record) is quarantined under a
// conflict name and the transfer restarts cleanly from zero, mirroring the
// CDSE "resume refused" set-aside policy. The MPC route keeps failing closed.
#[test]
fn usgs_unprovable_checkpoint_quarantines_and_restarts_from_zero() {
    let fixture = Fixture::new();
    let destination = fixture.0.join("result.tar");
    fs::write(part_path(&destination), b"abc").unwrap();
    PartCheckpoint::save(
        &destination,
        &"a".repeat(64),
        &validator("\"version-one\""),
        3,
        prefix_sha(),
    )
    .unwrap();
    let before = fs::read(part_path(&destination)).unwrap();
    let record = fs::read(stored_path(&fixture)).unwrap();
    // The fixture server answers a plain 200 with the full object: the
    // restarted transfer re-downloads all 6 bytes from zero.
    let (url, server) = one_response(
        "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nETag: \"version-two\"\r\nConnection: close\r\n\r\nabcdef".into(),
    );
    let (request, asset) = checkpoint_request(&fixture, &url);
    let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
        chunk_size: 3,
        max_attempts: 2,
        timeout: Duration::from_secs(8),
        backoff: Duration::ZERO,
        ..TransferOptions::default()
    })
    .unwrap();
    let mut session = transfer.resolved_session();
    // A different binding: the saved checkpoint belongs to another identity.
    session.set_recovery_identity("b".repeat(64));
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        matches!(
            result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                ..
            })
        ),
        "an unprovable checkpoint must restart from zero and complete: {result:?}"
    );
    let headers = server.join().unwrap();
    assert!(
        !headers.contains("range:"),
        "the restarted transfer is fresh, not a resume"
    );
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"abcdef",
        "the fresh download completes the object"
    );
    // The old evidence survives under a quarantined (conflict-stamped) name;
    // nothing was deleted or truncated.
    let parent = destination.parent().unwrap();
    let quarantined_parts: Vec<_> = std::fs::read_dir(parent)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.contains("eostage-conflict-"))
        })
        .collect();
    assert!(
        !quarantined_parts.is_empty(),
        "the unprovable evidence is preserved under a conflict name"
    );
    let mut preserved_prefix = false;
    let mut preserved_record = false;
    for entry in &quarantined_parts {
        let name = entry.file_name();
        let name = name.to_str().unwrap();
        let content = fs::read(entry.path()).unwrap();
        // Conflict names are "<name>.eostage-conflict-<stamp>".
        if name.contains("result.tar.part.eostage-conflict-") {
            preserved_prefix = content == before;
        }
        if name.contains("result.tar.part.resume.json.eostage-conflict-") {
            preserved_record = content == record;
        }
    }
    assert!(preserved_prefix, "the prefix bytes are preserved verbatim");
    assert!(
        preserved_record || quarantined_parts.len() >= 2,
        "the checkpoint record is preserved"
    );
    assert!(!part_path(&destination).exists());
    assert!(!stored_path(&fixture).exists());
}

#[test]
fn checkpoint_schema2_roundtrip_binds_last_modified_and_total() {
    let fixture = Fixture::new();
    let destination = fixture.0.join("result.tar");
    fs::write(part_path(&destination), b"abc").unwrap();
    let last_modified_validator = RemoteValidator {
        etag: None,
        last_modified: Some("Wed, 21 Oct 2026 07:28:00 GMT".to_owned()),
        content_length: Some(6),
    };
    PartCheckpoint::save(
        &destination,
        &"a".repeat(64),
        &last_modified_validator,
        3,
        prefix_sha(),
    )
    .unwrap();
    let stored = fs::read_to_string(stored_path(&fixture)).unwrap();
    assert!(stored.contains("\"schema\":2"), "schema 2 record: {stored}");
    assert!(!stored.contains("version-one"), "no fabricated etag value");
    let loaded = PartCheckpoint::load(&destination, &"a".repeat(64), Some(6), &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.bytes, 3);
    let restored = loaded.validator();
    assert_eq!(restored.etag, None);
    assert_eq!(
        restored.last_modified.as_deref(),
        Some("Wed, 21 Oct 2026 07:28:00 GMT")
    );
    assert_eq!(restored.content_length, Some(6));

    // A genuine schema-1 record as the historical writer serialized it: no
    // last_modified field at all, digest over the original field set. It must
    // still load, digest-validate, resume, and migrate only on the next save.
    let fixture_two = Fixture::new();
    let destination_two = fixture_two.0.join("result.tar");
    fs::write(part_path(&destination_two), b"abc").unwrap();
    let legacy_record = legacy_schema1_record(
        &destination_two,
        &"a".repeat(64),
        "\"version-one\"",
        3,
        &prefix_sha(),
    );
    fs::write(stored_path(&fixture_two), legacy_record).unwrap();
    let legacy = PartCheckpoint::load(&destination_two, &"a".repeat(64), Some(6), &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(legacy.validator().etag.as_deref(), Some("\"version-one\""));
    assert_eq!(legacy.validator().last_modified, None);
    assert_eq!(legacy.bytes, 3);

    // The legacy prefix resumes through a serving response carrying the same
    // strong ETag plus an unrelated Last-Modified — coexistence must not
    // turn an unchanged object into a rejection.
    let (url, server) = one_response(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nETag: \"version-one\"\r\nLast-Modified: Wed, 21 Oct 2026 07:28:00 GMT\r\nConnection: close\r\n\r\ndef".into(),
    );
    let (request, asset) = inputs(&fixture_two, &url);
    let transfer = checkpoint_transfer();
    let mut session = transfer.resolved_session();
    session.set_recovery_identity("a".repeat(64));
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        matches!(
            result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                ..
            })
        ),
        "a legacy ETag checkpoint must resume even when the response also carries Last-Modified: {result:?}"
    );
    let headers = server.join().unwrap();
    assert!(
        headers.contains("range: bytes=3-\r\n"),
        "the legacy checkpoint must resume, not restart"
    );
    assert!(
        headers.contains("if-range: \"version-one\"\r\n"),
        "the probe must carry the legacy ETag validator"
    );
    assert_eq!(fs::read(&destination_two).unwrap(), b"abcdef");
}

/// Serialize a schema-1 checkpoint record exactly as the historical writer
/// did: struct serialization in declared field order, with the original
/// digest over the original field set. The directory marker comes from the
/// same pinned directory the loader will verify against.
fn legacy_schema1_record(
    destination: &std::path::Path,
    binding: &str,
    etag: &str,
    bytes: u64,
    prefix_sha256: &str,
) -> String {
    use crate::resolved_part_checkpoint::PartCheckpointV1;
    let directory =
        crate::pinned_directory::PinnedDirectory::create(destination.parent().unwrap()).unwrap();
    let record = PartCheckpointV1 {
        schema: 1,
        binding: binding.to_owned(),
        directory: directory.identity_marker(),
        file_name: destination
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
        total: 6,
        bytes,
        etag: etag.to_owned(),
        prefix_sha256: prefix_sha256.to_owned(),
        digest: String::new(),
    };
    let unsigned = serde_json::to_vec(&record).unwrap();
    let digest = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(unsigned))
    };
    let signed = PartCheckpointV1 { digest, ..record };
    serde_json::to_string(&signed).unwrap()
}

// B13 live evidence 2026-10-31: the frozen D806 advisory filesize
// (957,283,399) disagreed with the real dds object size (957,306,880). For
// the remote-authoritative USGS route the checkpoint total is the real
// object size, so the restart must resume — validated against the response's
// exact 206 Content-Range, never against the advisory value.
#[test]
fn usgs_restart_resumes_when_advisory_size_disagrees_with_the_real_object() {
    let fixture = Fixture::new();
    let transfer = checkpoint_transfer();
    let binding = "a".repeat(64);

    // Execution 1: the frozen advisory size (6) disagrees with the real
    // object (8 bytes), exactly like the live task; the server serves the
    // real 8-byte object and the transfer is interrupted after 3 bytes, so
    // the checkpoint stores total=8 (the real size).
    let (first_url, first) = one_response(
        "HTTP/1.1 200 OK\r\nContent-Length: 8\r\nLast-Modified: Fri, 09 Oct 2026 19:34:42 GMT\r\nConnection: close\r\n\r\nabcdefgh".into(),
    );
    let (request, asset) = (
        TransferRequest::new(
            "saved-task",
            "saved-item",
            &first_url,
            fixture.0.join("result.tar"),
            Some(6),
        )
        .unwrap(),
        ResolvedAsset::new(&first_url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(6))
            .with_remote_response_size_authority(),
    );
    let mut session = transfer.resolved_session();
    session.set_recovery_identity(binding.clone());
    let cancel = AtomicBool::new(false);
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        &mut session,
        &|| cancel.load(Ordering::SeqCst),
        &|| false,
        &mut |update| {
            if let TransferUpdate::Progress(progress) = update {
                if progress.bytes_completed == 3 {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        },
    );
    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(TransferResult {
            outcome: DownloadOutcome::Interrupted,
            ..
        })
    ));
    assert_eq!(fs::read(part_path(&request.destination)).unwrap(), b"abc");
    let stored = fs::read_to_string(stored_path(&fixture)).unwrap();
    assert!(
        stored.contains("\"total\":8"),
        "the checkpoint total is the real object size: {stored}"
    );
    drop(session);

    // Restart: fresh session, same frozen advisory size. The re-resolved URL
    // is new; the server honours Range with the real total in Content-Range.
    let (second_url, second) = one_response(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: 5\r\nContent-Range: bytes 3-7/8\r\nLast-Modified: Fri, 09 Oct 2026 19:34:42 GMT\r\nConnection: close\r\n\r\ndefgh".into(),
    );
    let (request, asset) = (
        TransferRequest::new(
            "saved-task",
            "saved-item",
            &second_url,
            fixture.0.join("result.tar"),
            Some(6),
        )
        .unwrap(),
        ResolvedAsset::new(&second_url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(6))
            .with_remote_response_size_authority(),
    );
    let mut reopened = transfer.resolved_session();
    reopened.set_recovery_identity(binding.clone());
    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        &mut reopened,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        matches!(
            result,
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                ..
            })
        ),
        "an advisory/frozen size disagreement must not break the restart resume: {result:?}"
    );
    let headers = second.join().unwrap();
    assert!(
        headers.contains("range: bytes=3-\r\n"),
        "the restart must resume from the checkpoint"
    );
    assert!(
        headers.contains("if-range: fri, 09 oct 2026 19:34:42 gmt\r\n"),
        "the probe must carry the Last-Modified validator"
    );
    assert_eq!(
        fs::read(&request.destination).unwrap(),
        b"abcdefgh",
        "the completed file is the real object"
    );
    let _ = first.join();
}
