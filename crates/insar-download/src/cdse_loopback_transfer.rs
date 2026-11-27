//! CDSE archive transfer integration tests.
//!
//! This module is compiled only for crate tests.  The test-only retarget seam
//! keeps the production resolver URL intact until an individual fixture opts
//! into a literal-loopback HTTP server.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetResolverParameters, AssetRole, DownloadOutcome, FrozenAssetAccess, StableAssetReference,
};

use crate::asset_resolver::ResolvedAuthKind;
use crate::{
    part_path, AssetResolverRegistry, AuthRefreshPolicyId, CdseArchiveAssetResolver, Clock,
    CredentialMaterial, CredentialProvider, CredentialProviderError, HttpTransfer, ResolveContext,
    ResolvedAsset, ResolvedMethod, ResolvedTransferResult, TransferOptions, TransferRequest,
    TransferUpdate,
};

const PRODUCT_ID: &str = "00000000-0000-4000-8000-000000000001";
const SYNTHETIC_BEARER: &str = "SYNTHETIC_CDSE_TRANSFER_BEARER";
const ARCHIVE_BYTES: &[u8] = b"abcdef";
const ARCHIVE_MD5: &str = "e80b5017098950fc58aad83c8c14978e";
const MAX_REQUEST_HEADER_BYTES: usize = 8 * 1024;
const MAX_FIXTURE_BODY_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_SCRIPTS: usize = 8;

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct FixtureCredentials;

impl CredentialProvider for FixtureCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "copernicus-dataspace-default");
        CredentialMaterial::bearer(SYNTHETIC_BEARER)
    }
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-cdse-loopback-transfer-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create isolated fixture directory");
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct CapturedRequest {
    request_line: String,
    headers: Vec<(String, String)>,
}

impl CapturedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

struct ScriptedServer {
    origin: String,
    captures: Arc<Mutex<Vec<CapturedRequest>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ScriptedServer {
    fn new(responses: Vec<Vec<u8>>) -> Self {
        assert!(
            responses.len() <= MAX_RESPONSE_SCRIPTS,
            "fixture response-script count exceeded bound"
        );
        assert!(
            responses.iter().all(|response| {
                response.len() <= MAX_REQUEST_HEADER_BYTES + MAX_FIXTURE_BODY_BYTES
            }),
            "fixture response exceeded bound"
        );
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind literal loopback");
        listener
            .set_nonblocking(true)
            .expect("configure fixture listener");
        let address = listener.local_addr().expect("fixture listener address");
        let captures = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&captures);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut response_index = 0;
            while response_index < responses.len() && !stop_worker.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("configure accepted fixture stream");
                        let request = read_bounded_request(&mut stream);
                        captured.lock().expect("fixture capture lock").push(request);
                        stream
                            .write_all(&responses[response_index])
                            .expect("write fixture response");
                        stream.flush().expect("flush fixture response");
                        response_index += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            }
        });
        Self {
            origin: format!("http://{address}"),
            captures,
            stop,
            worker: Some(worker),
        }
    }

    fn url_for_path(&self, path: &str) -> String {
        assert!(path.starts_with('/'));
        format!("{}{}", self.origin, path)
    }

    fn finish(mut self) -> Vec<CapturedRequest> {
        self.stop.store(true, Ordering::Release);
        self.worker
            .take()
            .expect("fixture worker")
            .join()
            .expect("fixture worker completed");
        self.captures.lock().expect("fixture capture lock").clone()
    }
}

impl Drop for ScriptedServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_bounded_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("fixture request timeout");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 256];
    let header_end = loop {
        let count = stream.read(&mut buffer).expect("read fixture request");
        assert!(count > 0, "fixture request ended before CRLF terminator");
        request.extend_from_slice(&buffer[..count]);
        assert!(
            request.len() <= MAX_REQUEST_HEADER_BYTES,
            "fixture request header exceeded bound"
        );
        if let Some(position) = request.windows(4).position(|value| value == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let request = std::str::from_utf8(&request[..header_end]).expect("fixture request is UTF-8");
    let mut lines = request.split("\r\n");
    let request_line = lines.next().expect("fixture request line").to_owned();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line.split_once(':').expect("fixture header syntax");
            (name.trim().to_owned(), value.trim().to_owned())
        })
        .collect();
    CapturedRequest {
        request_line,
        headers,
    }
}

fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    assert!(
        body.len() <= MAX_FIXTURE_BODY_BYTES,
        "fixture response body exceeded bound"
    );
    let mut value = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for (name, content) in headers {
        value.push_str(name);
        value.push_str(": ");
        value.push_str(content);
        value.push_str("\r\n");
    }
    value.push_str("\r\n");
    let mut value = value.into_bytes();
    value.extend_from_slice(body);
    value
}

fn frozen_archive(expected_size: u64, checksum: &str) -> FrozenAssetAccess {
    let provider_product_key =
        provider_product_key("copernicus.dataspace.odata", "SENTINEL-2", PRODUCT_ID)
            .expect("fixture product key");
    let stable_asset_key =
        stable_asset_key(&provider_product_key, "archive").expect("fixture stable asset key");
    FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: "copernicus.dataspace.odata".to_owned(),
            collection_id: "SENTINEL-2".to_owned(),
            provider_product_id: PRODUCT_ID.to_owned(),
            provider_product_key,
            canonical_product_key: None,
            asset_key: "archive".to_owned(),
            stable_asset_key,
        },
        descriptor: AssetAccessDescriptor {
            resolver_id: "cdse.archive.resolver.v1".to_owned(),
            scheme: AssetAccessScheme::OauthBearerHttps,
            stable_href: Some(format!(
                "https://download.dataspace.copernicus.eu/odata/v1/Products({PRODUCT_ID})/$value"
            )),
            requires_credentials: true,
            credential_profile: Some("copernicus-dataspace-default".to_owned()),
            resolver_parameters: AssetResolverParameters {},
        },
        role: AssetRole::Primary,
        file_name: Some("S2_LOOPBACK_FIXTURE.SAFE.zip".to_owned()),
        expected_size_bytes: Some(expected_size),
        checksum_algorithm: Some("MD5".to_owned()),
        checksum_value: Some(checksum.to_owned()),
    }
}

fn resolve_cdse_archive(expected_size: u64, checksum: &str) -> ResolvedAsset {
    let credentials = FixtureCredentials;
    let cancelled = || false;
    let clock = FixedClock;
    let mut resolvers = AssetResolverRegistry::new();
    resolvers
        .register(Arc::new(CdseArchiveAssetResolver::new()))
        .expect("explicit fixture registration only");
    resolvers
        .resolve_frozen(
            &frozen_archive(expected_size, checksum),
            &ResolveContext::new(&credentials, &cancelled, &clock),
        )
        .expect("valid frozen CDSE archive")
}

fn retarget_to_loopback(asset: &mut ResolvedAsset, server: &ScriptedServer) {
    let production_url = asset.effective_url().to_string();
    let path = asset.effective_url().path().to_owned();
    let loopback_url = server.url_for_path(&path);
    asset
        .retarget_for_loopback_fixture(&loopback_url)
        .expect("test-only loopback retarget");
    assert_eq!(
        production_url,
        format!("https://download.dataspace.copernicus.eu/odata/v1/Products({PRODUCT_ID})/$value")
    );
    assert_eq!(asset.effective_url().as_str(), loopback_url);
}

fn transfer(max_attempts: u32) -> HttpTransfer {
    HttpTransfer::new(TransferOptions {
        max_attempts,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(2),
        chunk_size: 2,
        max_accepted_polls: 1,
        accepted_poll_timeout: Duration::from_secs(2),
    })
    .expect("fixture transfer")
}

fn request_for(asset: &ResolvedAsset, destination: &Path, expected_size: u64) -> TransferRequest {
    TransferRequest::new(
        "cdse-loopback-task",
        "cdse-loopback-item",
        asset.effective_url().as_str(),
        destination.to_path_buf(),
        Some(expected_size),
    )
    .expect("fixture request")
}

#[test]
fn cdse_resolver_transfer_uses_http_transfer_and_keeps_bearer_runtime_only() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("archive.SAFE.zip");
    let server = ScriptedServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "6"), ("ETag", "\"fixture-v1\"")],
        ARCHIVE_BYTES,
    )]);
    assert!(server.origin.starts_with("http://127.0.0.1:"));
    let expected_host = server
        .origin
        .strip_prefix("http://")
        .expect("literal loopback origin")
        .to_owned();
    let frozen = frozen_archive(ARCHIVE_BYTES.len() as u64, ARCHIVE_MD5);
    assert!(!format!("{frozen:?}").contains(SYNTHETIC_BEARER));
    let mut asset = resolve_cdse_archive(ARCHIVE_BYTES.len() as u64, ARCHIVE_MD5);
    assert_eq!(
        asset.effective_url().as_str(),
        format!("https://download.dataspace.copernicus.eu/odata/v1/Products({PRODUCT_ID})/$value")
    );
    assert_eq!(asset.method(), ResolvedMethod::Get);
    assert_eq!(asset.auth_kind(), ResolvedAuthKind::Bearer);
    assert_eq!(asset.expected_size(), Some(ARCHIVE_BYTES.len() as u64));
    assert_eq!(
        asset
            .expected_checksum()
            .expect("frozen checksum")
            .algorithm,
        "MD5"
    );
    assert_eq!(
        asset.suggested_filename(),
        Some("S2_LOOPBACK_FIXTURE.SAFE.zip")
    );
    assert_eq!(asset.asset_role(), Some(&AssetRole::Primary));
    let resolved_debug = format!("{asset:?}");
    assert!(resolved_debug.contains("CdseDownloadV1"));
    assert!(!resolved_debug.contains(SYNTHETIC_BEARER));
    retarget_to_loopback(&mut asset, &server);
    let request = request_for(&asset, &destination, ARCHIVE_BYTES.len() as u64);
    let request_debug = format!("{request:?}");
    assert!(!request_debug.contains(SYNTHETIC_BEARER));
    let transfer = transfer(1);
    let mut session = transfer.resolved_session();
    let mut progress = Vec::new();

    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |value| progress.push(value),
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    assert_eq!(
        fs::read(&destination).expect("final archive"),
        ARCHIVE_BYTES
    );
    assert!(!format!("{result:?}").contains(SYNTHETIC_BEARER));
    assert!(!format!("{progress:?}").contains(SYNTHETIC_BEARER));
    assert!(!fs::read(&destination)
        .expect("final archive bytes")
        .windows(SYNTHETIC_BEARER.len())
        .any(|value| value == SYNTHETIC_BEARER.as_bytes()));
    assert!(!destination.to_string_lossy().contains(SYNTHETIC_BEARER));
    assert!(!part_path(&destination).exists());
    let captures = server.finish();
    assert_eq!(captures.len(), 1);
    assert_eq!(
        captures[0].request_line,
        format!("GET /odata/v1/Products({PRODUCT_ID})/$value HTTP/1.1")
    );
    assert_eq!(
        captures[0].header("Authorization"),
        Some("Bearer SYNTHETIC_CDSE_TRANSFER_BEARER")
    );
    assert!(captures[0].header("Cookie").is_none());
    assert_eq!(captures[0].header("Host"), Some(expected_host.as_str()));
}

#[test]
fn cdse_cancellation_before_request_sends_nothing_and_publishes_nothing() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("cancel-before-request.SAFE.zip");
    let server = ScriptedServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "6")],
        ARCHIVE_BYTES,
    )]);
    let mut asset = resolve_cdse_archive(ARCHIVE_BYTES.len() as u64, ARCHIVE_MD5);
    retarget_to_loopback(&mut asset, &server);
    let request = request_for(&asset, &destination, ARCHIVE_BYTES.len() as u64);
    let transfer = transfer(1);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut session,
        &|| true,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Interrupted && result.attempt_count == 0
    ));
    assert!(!destination.exists());
    assert!(!part_path(&destination).exists());
    assert!(server.finish().is_empty());
}

#[test]
fn cdse_complete_verified_part_finalizes_without_another_request() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("finalize-part.SAFE.zip");
    fs::write(part_path(&destination), ARCHIVE_BYTES).expect("write verified fixture partial");
    let server = ScriptedServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "6")],
        ARCHIVE_BYTES,
    )]);
    let mut asset = resolve_cdse_archive(ARCHIVE_BYTES.len() as u64, ARCHIVE_MD5);
    retarget_to_loopback(&mut asset, &server);
    let request = request_for(&asset, &destination, ARCHIVE_BYTES.len() as u64);
    let transfer = transfer(1);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed && result.attempt_count == 0
    ));
    assert_eq!(
        fs::read(&destination).expect("promoted fixture part"),
        ARCHIVE_BYTES
    );
    assert!(!part_path(&destination).exists());
    assert!(server.finish().is_empty());
}

#[test]
fn cdse_cancelled_partial_resumes_with_exact_range_and_validator() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("resume.SAFE.zip");
    let server = ScriptedServer::new(vec![
        response(
            "200 OK",
            &[("Content-Length", "6"), ("ETag", "\"fixture-v1\"")],
            ARCHIVE_BYTES,
        ),
        response(
            "206 Partial Content",
            &[
                ("Content-Length", "4"),
                ("Content-Range", "bytes 2-5/6"),
                ("ETag", "\"fixture-v1\""),
            ],
            b"cdef",
        ),
    ]);
    let mut asset = resolve_cdse_archive(ARCHIVE_BYTES.len() as u64, ARCHIVE_MD5);
    retarget_to_loopback(&mut asset, &server);
    let request = request_for(&asset, &destination, ARCHIVE_BYTES.len() as u64);
    let transfer = transfer(2);
    let mut session = transfer.resolved_session();
    let cancelled = AtomicBool::new(false);
    let cancelled_probe = || cancelled.load(Ordering::Acquire);
    let mut progress = |update: TransferUpdate| {
        if let TransferUpdate::Progress(value) = update {
            if value.bytes_completed >= 2 {
                cancelled.store(true, Ordering::Release);
            }
        }
    };

    let interrupted = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut session,
        &cancelled_probe,
        &|| false,
        &mut progress,
    );
    assert!(matches!(
        interrupted,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Interrupted
    ));
    assert!(!destination.exists());
    assert_eq!(
        fs::read(part_path(&destination)).expect("preserved partial"),
        b"ab"
    );

    cancelled.store(false, Ordering::Release);
    let completed = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut session,
        &cancelled_probe,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        completed,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    assert_eq!(
        fs::read(&destination).expect("resumed final archive"),
        ARCHIVE_BYTES
    );
    let captures = server.finish();
    assert_eq!(captures.len(), 2);
    assert_eq!(captures[1].header("Range"), Some("bytes=2-"));
    assert_eq!(captures[1].header("If-Range"), Some("\"fixture-v1\""));
}

#[test]
fn cdse_refused_resume_sets_the_partial_aside_and_downloads_again() {
    // CDSE assembles a Sentinel-2 SAFE zip on the fly and stops honouring a
    // Range resume about a minute after the original transfer.
    let refusals = [
        response("416 Range Not Satisfiable", &[("Content-Length", "0")], b""),
        response("200 OK", &[("Content-Length", "6")], ARCHIVE_BYTES),
    ];
    for (index, refusal) in refusals.into_iter().enumerate() {
        let temp = TempDirectory::new();
        let destination = temp.0.join("restart.SAFE.zip");
        fs::write(part_path(&destination), b"abc").expect("create prior partial");
        let server = ScriptedServer::new(vec![
            refusal,
            response("200 OK", &[("Content-Length", "6")], ARCHIVE_BYTES),
        ]);
        let mut asset = resolve_cdse_archive(ARCHIVE_BYTES.len() as u64, ARCHIVE_MD5);
        retarget_to_loopback(&mut asset, &server);
        let request = request_for(&asset, &destination, ARCHIVE_BYTES.len() as u64);
        let transfer = transfer(2);
        let mut session = transfer.resolved_session();

        let result = transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::CdseOAuthBearerV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );
        assert!(
            matches!(
                result,
                ResolvedTransferResult::Finished(ref result)
                    if result.outcome == DownloadOutcome::Completed
            ),
            "refusal {index}: {result:?}"
        );
        assert_eq!(
            fs::read(&destination).expect("verified archive"),
            ARCHIVE_BYTES
        );
        assert!(!part_path(&destination).exists());
        let set_aside = fs::read_dir(&temp.0)
            .expect("list destination folder")
            .map(|entry| entry.expect("folder entry").path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("restart.SAFE.zip.part.eostage-conflict-"))
            })
            .collect::<Vec<_>>();
        assert_eq!(set_aside.len(), 1, "refusal {index}: {set_aside:?}");
        assert_eq!(
            fs::read(&set_aside[0]).expect("kept partial"),
            b"abc",
            "the refused partial is kept unchanged"
        );
        let captures = server.finish();
        assert_eq!(captures.len(), 2);
        assert_eq!(captures[0].header("Range"), Some("bytes=3-"));
        assert_eq!(
            captures[1].header("Range"),
            None,
            "the new transfer starts at zero"
        );
    }
}

#[test]
fn cdse_refused_resume_on_the_last_attempt_still_downloads_from_zero() {
    // Pause and resume share one attempt budget; setting the partial aside
    // makes no progress, so it must not use up the new transfer's attempt.
    let temp = TempDirectory::new();
    let destination = temp.0.join("last-attempt.SAFE.zip");
    fs::write(part_path(&destination), b"ab").expect("create prior partial");
    let server = ScriptedServer::new(vec![
        response("416 Range Not Satisfiable", &[("Content-Length", "0")], b""),
        response("200 OK", &[("Content-Length", "6")], ARCHIVE_BYTES),
    ]);
    let mut asset = resolve_cdse_archive(6, ARCHIVE_MD5);
    retarget_to_loopback(&mut asset, &server);
    let request = request_for(&asset, &destination, 6);
    let transfer = transfer(1);
    let mut session = transfer.resolved_session();

    let result = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(
        matches!(
            result,
            ResolvedTransferResult::Finished(ref result)
                if result.outcome == DownloadOutcome::Completed
        ),
        "{result:?}"
    );
    assert_eq!(
        fs::read(&destination).expect("verified archive"),
        ARCHIVE_BYTES
    );
    assert!(!part_path(&destination).exists());
    let kept = fs::read_dir(&temp.0)
        .expect("list destination folder")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("last-attempt.SAFE.zip.part.eostage-conflict-")
        })
        .map(|entry| fs::read(entry.path()).expect("kept partial"))
        .collect::<Vec<_>>();
    assert_eq!(kept, vec![b"ab".to_vec()]);
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn cdse_invalid_resume_responses_fail_closed_without_a_final_destination() {
    let temp = TempDirectory::new();

    let invalid_range_destination = temp.0.join("invalid-range.SAFE.zip");
    fs::write(part_path(&invalid_range_destination), b"ab").expect("prior partial");
    let invalid_range_server = ScriptedServer::new(vec![response(
        "206 Partial Content",
        &[("Content-Length", "3"), ("Content-Range", "bytes 3-5/6")],
        b"def",
    )]);
    let mut invalid_range_asset = resolve_cdse_archive(6, ARCHIVE_MD5);
    retarget_to_loopback(&mut invalid_range_asset, &invalid_range_server);
    let invalid_range_request = request_for(&invalid_range_asset, &invalid_range_destination, 6);
    let invalid_range_transfer = transfer(1);
    let mut invalid_range_session = invalid_range_transfer.resolved_session();
    let invalid_range_result = invalid_range_transfer.download_resolved(
        &invalid_range_request,
        &invalid_range_asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut invalid_range_session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        invalid_range_result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Failed
    ));
    assert!(!invalid_range_destination.exists());
    assert_eq!(
        fs::read(part_path(&invalid_range_destination)).expect("preserved partial"),
        b"ab",
        "a malformed partial response is no reason to set the prefix aside"
    );
    assert_eq!(invalid_range_server.finish().len(), 1);
}

#[test]
fn cdse_pause_holds_partial_without_publishing_then_completes() {
    let temp = TempDirectory::new();
    let destination = temp.0.join("pause.SAFE.zip");
    let server = ScriptedServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "6")],
        ARCHIVE_BYTES,
    )]);
    let mut asset = resolve_cdse_archive(ARCHIVE_BYTES.len() as u64, ARCHIVE_MD5);
    retarget_to_loopback(&mut asset, &server);
    let request = request_for(&asset, &destination, ARCHIVE_BYTES.len() as u64);
    let paused = Arc::new(AtomicBool::new(false));
    let paused_worker = Arc::clone(&paused);
    let pause_once = Arc::new(AtomicBool::new(true));
    let pause_once_worker = Arc::clone(&pause_once);
    let (paused_sender, paused_receiver) = mpsc::channel();
    let (finished_sender, finished_receiver) = mpsc::channel();
    let transfer = transfer(1);
    let worker = thread::spawn(move || {
        let mut session = transfer.resolved_session();
        let cancelled = || false;
        let paused_check = || paused_worker.load(Ordering::Acquire);
        let mut progress = |update: TransferUpdate| {
            if let TransferUpdate::Progress(value) = update {
                if value.bytes_completed >= 2 && pause_once_worker.swap(false, Ordering::AcqRel) {
                    paused_worker.store(true, Ordering::Release);
                    paused_sender.send(()).expect("notify pause");
                }
            }
        };
        let result = transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::CdseOAuthBearerV1,
            &mut session,
            &cancelled,
            &paused_check,
            &mut progress,
        );
        finished_sender.send(result).expect("notify completion");
    });

    paused_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("transfer reaches pause after first body chunk");
    thread::sleep(Duration::from_millis(120));
    let no_completion = matches!(finished_receiver.try_recv(), Err(mpsc::TryRecvError::Empty));
    let no_final = !destination.exists();
    let partial = fs::read(part_path(&destination)).expect("paused partial");
    paused.store(false, Ordering::Release);
    assert!(no_completion, "paused transfer must not publish completion");
    assert!(no_final, "paused transfer must not publish final file");
    assert_eq!(partial, b"ab");

    let result = finished_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("unpaused transfer completes");
    worker.join().expect("pause fixture worker");
    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    assert_eq!(
        fs::read(&destination).expect("paused final archive"),
        ARCHIVE_BYTES
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn cdse_integrity_conflicts_never_publish_a_final_file() {
    let temp = TempDirectory::new();
    let checksum_destination = temp.0.join("checksum.SAFE.zip");
    let checksum_server = ScriptedServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "6")],
        b"abcdeg",
    )]);
    let mut checksum_asset = resolve_cdse_archive(6, ARCHIVE_MD5);
    retarget_to_loopback(&mut checksum_asset, &checksum_server);
    let checksum_request = request_for(&checksum_asset, &checksum_destination, 6);
    let checksum_transfer = transfer(1);
    let mut checksum_session = checksum_transfer.resolved_session();
    let checksum_result = checksum_transfer.download_resolved(
        &checksum_request,
        &checksum_asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut checksum_session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        checksum_result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Failed
    ));
    assert!(!checksum_destination.exists());
    assert!(part_path(&checksum_destination).exists());
    assert_eq!(checksum_server.finish().len(), 1);

    let size_destination = temp.0.join("size.SAFE.zip");
    let size_server = ScriptedServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "7")],
        b"abcdefg",
    )]);
    let mut size_asset = resolve_cdse_archive(6, ARCHIVE_MD5);
    retarget_to_loopback(&mut size_asset, &size_server);
    let size_request = request_for(&size_asset, &size_destination, 6);
    let size_transfer = transfer(1);
    let mut size_session = size_transfer.resolved_session();
    let size_result = size_transfer.download_resolved(
        &size_request,
        &size_asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut size_session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        size_result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Failed
    ));
    assert!(!size_destination.exists());
    assert_eq!(size_server.finish().len(), 1);
}

#[test]
fn cdse_auth_rejections_and_redirects_do_not_escape_the_fixture_boundary() {
    let temp = TempDirectory::new();
    let auth_destination = temp.0.join("auth.SAFE.zip");
    let auth_server = ScriptedServer::new(vec![response("401 Unauthorized", &[], b"")]);
    let mut auth_asset = resolve_cdse_archive(6, ARCHIVE_MD5);
    retarget_to_loopback(&mut auth_asset, &auth_server);
    let auth_request = request_for(&auth_asset, &auth_destination, 6);
    let auth_transfer = transfer(1);
    let mut auth_session = auth_transfer.resolved_session();
    let auth_result = auth_transfer.download_resolved(
        &auth_request,
        &auth_asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut auth_session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        auth_result,
        ResolvedTransferResult::AuthRejected(rejection)
            if rejection.status_code == 401 && rejection.refresh_candidate
    ));
    assert!(!format!("{auth_result:?}").contains(SYNTHETIC_BEARER));
    let auth_captures = auth_server.finish();
    assert_eq!(auth_captures.len(), 1);
    assert_eq!(
        auth_captures[0].header("Authorization"),
        Some("Bearer SYNTHETIC_CDSE_TRANSFER_BEARER")
    );

    let ordinary_403 = ScriptedServer::new(vec![response("403 Forbidden", &[], b"")]);
    let ordinary_destination = temp.0.join("ordinary-403.SAFE.zip");
    let mut ordinary_asset = resolve_cdse_archive(6, ARCHIVE_MD5);
    retarget_to_loopback(&mut ordinary_asset, &ordinary_403);
    let ordinary_request = request_for(&ordinary_asset, &ordinary_destination, 6);
    let ordinary_transfer = transfer(1);
    let mut ordinary_session = ordinary_transfer.resolved_session();
    let ordinary_result = ordinary_transfer.download_resolved(
        &ordinary_request,
        &ordinary_asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut ordinary_session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        ordinary_result,
        ResolvedTransferResult::AuthRejected(rejection)
            if rejection.status_code == 403 && !rejection.refresh_candidate
    ));
    assert!(!format!("{ordinary_result:?}").contains(SYNTHETIC_BEARER));
    assert_eq!(ordinary_403.finish().len(), 1);

    let invalid_token_403 = ScriptedServer::new(vec![response(
        "403 Forbidden",
        &[("WWW-Authenticate", "Bearer error=\"invalid_token\"")],
        b"",
    )]);
    let invalid_token_destination = temp.0.join("invalid-token-403.SAFE.zip");
    let mut invalid_token_asset = resolve_cdse_archive(6, ARCHIVE_MD5);
    retarget_to_loopback(&mut invalid_token_asset, &invalid_token_403);
    let invalid_token_request = request_for(&invalid_token_asset, &invalid_token_destination, 6);
    let invalid_token_transfer = transfer(1);
    let mut invalid_token_session = invalid_token_transfer.resolved_session();
    let invalid_token_result = invalid_token_transfer.download_resolved(
        &invalid_token_request,
        &invalid_token_asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut invalid_token_session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        invalid_token_result,
        ResolvedTransferResult::AuthRejected(rejection)
            if rejection.status_code == 403 && rejection.refresh_candidate
    ));
    assert!(!format!("{invalid_token_result:?}").contains(SYNTHETIC_BEARER));
    assert_eq!(invalid_token_403.finish().len(), 1);

    let target = ScriptedServer::new(vec![response(
        "200 OK",
        &[("Content-Length", "6")],
        ARCHIVE_BYTES,
    )]);
    let redirect = target.url_for_path("/unrelated");
    let source = ScriptedServer::new(vec![response(
        "302 Found",
        &[("Location", redirect.as_str())],
        b"",
    )]);
    let redirect_destination = temp.0.join("redirect.SAFE.zip");
    let mut redirect_asset = resolve_cdse_archive(6, ARCHIVE_MD5);
    retarget_to_loopback(&mut redirect_asset, &source);
    let redirect_request = request_for(&redirect_asset, &redirect_destination, 6);
    let redirect_transfer = transfer(1);
    let mut redirect_session = redirect_transfer.resolved_session();
    let redirect_result = redirect_transfer.download_resolved(
        &redirect_request,
        &redirect_asset,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut redirect_session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        redirect_result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Failed
    ));
    assert_eq!(source.finish().len(), 1);
    assert!(
        target.finish().is_empty(),
        "redirect target must receive no request"
    );
    assert!(!redirect_destination.exists());
}
