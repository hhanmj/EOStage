//! The process-wide Earthdata session: established once, shared by later
//! transfers, renewed once after a rejected bearer. Loopback fixtures only.

use super::*;
use crate::{
    AdmittedTransferBackend, CredentialMaterial, TransferAdmissionController, TransferBackend,
};
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Serves by path until idle: `/token` issues a token, `/authorize` sets the
/// ASF cookie, `/reject*` answers 401 once per path, anything else is `ok`.
fn earthdata_fixture() -> (String, thread::JoinHandle<Vec<String>>) {
    earthdata_fixture_with_token_delay(Duration::ZERO)
}

/// As [`earthdata_fixture`], answering `/token` only after `token_delay`.
fn earthdata_fixture_with_token_delay(
    token_delay: Duration,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let mut seen = Vec::new();
        let mut rejected = std::collections::HashSet::new();
        let mut idle_since = std::time::Instant::now();
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut request = String::new();
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        request.push_str(&line.to_ascii_lowercase());
                    }
                    let first = request.lines().next().unwrap_or_default().to_owned();
                    let path = first
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_owned();
                    seen.push(request);
                    let mut stream = reader.into_inner();
                    if path == "/token" {
                        thread::sleep(token_delay);
                        let body = br#"{"access_token":"fixture-token"}"#;
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(body);
                    } else if path == "/authorize" {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    } else if path.starts_with("/reject") && rejected.insert(path) {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    } else {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                        );
                    }
                    let _ = stream.flush();
                    idle_since = std::time::Instant::now();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if idle_since.elapsed() > Duration::from_millis(1500) {
                        return seen;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return seen,
            }
        }
    });
    (base, server)
}

fn fixture_transfer(base: &str) -> HttpTransfer {
    let mut transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(2),
        ..TransferOptions::default()
    })
    .unwrap();
    transfer.resolved_earthdata_session_endpoints = ResolvedEarthdataSessionEndpoints::new(
        &format!("{base}/token"),
        &format!("{base}/authorize"),
        &format!("{base}/"),
    )
    .unwrap();
    transfer
}

fn download(transfer: &HttpTransfer, base: &str, path: &str) -> ResolvedTransferResult {
    let destination = std::env::temp_dir().join(format!(
        "insar-earthdata-shared-{}-{}.bin",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_file(&destination);
    let _ = fs::remove_file(part_path(&destination));
    let url = format!("{base}{path}");
    let mut asset = ResolvedAsset::new(&url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(2))
        .with_host_policy(crate::asset_resolver::ResolvedHostPolicy::EarthdataAsfV1);
    CredentialMaterial::basic("fixture-user", "fixture-password")
        .unwrap()
        .apply_to(&mut asset)
        .unwrap();
    let request = TransferRequest::new(
        "earthdata-task",
        "earthdata-item",
        &url,
        destination.clone(),
        Some(2),
    )
    .unwrap();
    let admitted =
        AdmittedTransferBackend::new(transfer, TransferAdmissionController::compatibility());
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
    let _ = fs::remove_file(&destination);
    let _ = fs::remove_file(part_path(&destination));
    result
}

fn completed(result: &ResolvedTransferResult) -> bool {
    matches!(result, ResolvedTransferResult::Finished(result) if result.outcome == DownloadOutcome::Completed)
}

fn token_requests(seen: &[String]) -> usize {
    seen.iter()
        .filter(|request| request.starts_with("post /token "))
        .count()
}

#[test]
fn later_transfers_reuse_the_established_earthdata_session() {
    let (base, server) = earthdata_fixture();
    let transfer = fixture_transfer(&base);
    assert!(completed(&download(&transfer, &base, "/first")));
    assert!(completed(&download(&transfer, &base, "/second")));
    let seen = server.join().unwrap();
    assert_eq!(token_requests(&seen), 1, "{seen:#?}");
    let second = seen
        .iter()
        .find(|request| request.starts_with("get /second "))
        .expect("second transfer reached the archive");
    assert!(second.contains("authorization: bearer fixture-token"));
    assert!(second.contains("cookie: asf-urs=fixture-session"));
}

#[test]
fn burst_extractor_rejection_of_the_session_bearer_is_refreshed_once() {
    let rejection = classify_auth_rejection(
        AuthRefreshPolicyId::EarthdataBurstSessionV1,
        401,
        ResponseHostClass::Approved,
        crate::asset_resolver::ResolvedAuthKind::Bearer,
        None,
    );
    assert!(rejection.refresh_candidate);
    for (status, host, auth) in [
        (
            401,
            ResponseHostClass::Unapproved,
            crate::asset_resolver::ResolvedAuthKind::Bearer,
        ),
        (
            403,
            ResponseHostClass::Approved,
            crate::asset_resolver::ResolvedAuthKind::Bearer,
        ),
        (
            401,
            ResponseHostClass::Approved,
            crate::asset_resolver::ResolvedAuthKind::Basic,
        ),
    ] {
        assert!(
            !classify_auth_rejection(
                AuthRefreshPolicyId::EarthdataBurstSessionV1,
                status,
                host,
                auth,
                None
            )
            .refresh_candidate,
            "{status} {host:?} {auth:?}"
        );
    }
}

#[test]
fn a_session_established_before_admission_serves_the_first_transfer() {
    let (base, server) = earthdata_fixture();
    let transfer = fixture_transfer(&base);
    transfer
        .ensure_earthdata_session_with_credential(
            &TransferAuth::basic("fixture-user", "fixture-password").unwrap(),
        )
        .expect("pre-admission session");
    assert!(completed(&download(&transfer, &base, "/first")));
    let seen = server.join().unwrap();
    assert_eq!(token_requests(&seen), 1, "{seen:#?}");
    assert!(seen[0].starts_with("post /token "));
    // Requests are recorded lowercased; the Basic value is base64 of
    // "fixture-user:fixture-password".
    let basic = "authorization: basic zml4dhvyzs11c2vyomzpehr1cmutcgfzc3dvcmq=";
    for request in &seen {
        assert!(!request.contains("fixture-password"), "{request}");
        let login = request.starts_with("post /token ") || request.starts_with("get /authorize ");
        assert_eq!(
            request.contains(basic),
            login,
            "the saved credential may reach only the Earthdata login endpoints: {request}"
        );
        assert_eq!(request.matches("authorization:").count(), 1, "{request}");
    }
    let archive = seen
        .iter()
        .find(|request| request.starts_with("get /first "))
        .expect("first transfer reached the archive");
    assert!(archive.contains("authorization: bearer fixture-token"));
}

#[test]
fn a_rejected_session_is_replaced_once_and_concurrent_transfers_share_it() {
    let (base, server) = earthdata_fixture();
    let transfer = fixture_transfer(&base);
    assert!(matches!(
        download(&transfer, &base, "/reject-once"),
        ResolvedTransferResult::AuthRejected(_)
    ));
    // The resolver's single refresh retries; both concurrent transfers must
    // wait for one new session instead of each requesting a token.
    thread::scope(|scope| {
        let retry = scope.spawn(|| download(&transfer, &base, "/reject-once"));
        let other = scope.spawn(|| download(&transfer, &base, "/other"));
        assert!(completed(&retry.join().unwrap()));
        assert!(completed(&other.join().unwrap()));
    });
    let seen = server.join().unwrap();
    assert_eq!(token_requests(&seen), 2, "{seen:#?}");
}

#[test]
fn changing_the_saved_credential_retires_the_shared_session() {
    let (base, server) = earthdata_fixture();
    let transfer = fixture_transfer(&base);
    assert!(completed(&download(&transfer, &base, "/first")));
    transfer.invalidate_all_earthdata_sessions();
    assert!(completed(&download(&transfer, &base, "/second")));
    let seen = server.join().unwrap();
    assert_eq!(token_requests(&seen), 2, "{seen:#?}");
}

#[test]
fn a_cancelled_waiter_leaves_while_another_transfer_establishes_the_session() {
    let (base, server) = earthdata_fixture_with_token_delay(Duration::from_millis(1500));
    let transfer = fixture_transfer(&base);
    let credential = TransferAuth::basic("fixture-user", "fixture-password").unwrap();
    thread::scope(|scope| {
        let establishing =
            scope.spawn(|| transfer.ensure_earthdata_session_with_credential(&credential));
        thread::sleep(Duration::from_millis(300));
        let started = std::time::Instant::now();
        let waiter = transfer.ensure_earthdata_session(&|builder| builder, &|| true);
        let waited = started.elapsed();
        assert!(
            matches!(&waiter, Err(failure) if failure.code == "DL001"),
            "a cancelled waiter must not take the session"
        );
        assert!(
            waited < Duration::from_millis(700),
            "a cancelled waiter blocked {waited:?} behind the network request"
        );
        establishing.join().unwrap().expect("first establishment");
    });
    let seen = server.join().unwrap();
    assert_eq!(token_requests(&seen), 1, "{seen:#?}");
}

#[test]
fn the_established_session_is_visible_without_a_token_request() {
    let (base, server) = earthdata_fixture();
    let transfer = fixture_transfer(&base);
    assert!(!transfer.has_earthdata_session());
    let credential = TransferAuth::basic("fixture-user", "fixture-password").unwrap();
    transfer
        .ensure_earthdata_session_with_credential(&credential)
        .expect("first admission establishes the session");
    assert!(transfer.has_earthdata_session());
    // A later admission in the same session reuses it.
    transfer
        .ensure_earthdata_session_with_credential(&credential)
        .expect("second admission reuses the session");
    transfer.invalidate_all_earthdata_sessions();
    assert!(!transfer.has_earthdata_session());
    let seen = server.join().unwrap();
    assert_eq!(token_requests(&seen), 1, "{seen:#?}");
}
