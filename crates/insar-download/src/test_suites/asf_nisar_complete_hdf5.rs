use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::nisar_hdf5_support::{
    create_valid_fixture_for_granule, create_valid_fixture_for_profile, FixtureDirectory,
    FrequencyScope, P05023_PRODUCT_SPECIFICATION_VERSION,
};
use super::nisar_support::DISPLAY_NAME;
use super::nisar_support::{destination, frozen_item, sha256_hex, work, FILE_NAME};
use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetRole, DownloadOutcome, FrozenAssetAccess,
    ProxyPolicy, StableAssetReference,
};
use insar_download::{
    part_path, ArtifactValidationContext, ArtifactValidationError, ArtifactValidatorId,
    ArtifactValidatorRegistry, AssetResolver, AssetResolverRegistry, AuthRefreshPolicyId, Clock,
    CompletedArtifactValidator, CredentialMaterial, CredentialProvider, CredentialProviderError,
    DownloadWorker, HttpTransfer, ProductionDownloadWorker, ResolveContext, ResolveError,
    ResolvedAsset, ResolvedChecksum, ResolvedMethod, ResolvedTransferResult,
    ResolverExecutionRuntime, ShadowAccessIntent, ShadowResolveContext, TransferOptions,
    TransferRequest, WorkerControl, NISAR_RSLC_STRUCTURE_VALIDATOR_ID,
    NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING, NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
};
use insar_provider::catalog::NISAR_RESOLVER_POLICY_ID_V1;
use serde_json::json;

static VALID_BODY: OnceLock<Vec<u8>> = OnceLock::new();

fn body() -> &'static [u8] {
    VALID_BODY
        .get_or_init(|| {
            let fixtures = FixtureDirectory::new("complete-transfer-body");
            let path = fixtures.file("valid-a-only");
            create_valid_fixture_for_profile(
                &path,
                FrequencyScope::AOnly,
                DISPLAY_NAME,
                P05023_PRODUCT_SPECIFICATION_VERSION,
            );
            fs::read(path).expect("read valid synthetic NISAR RSLC HDF5")
        })
        .as_slice()
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-nisar-complete-hdf5-{label}-{}-{nonce}",
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

struct ScriptedServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ScriptedServer {
    fn new(responses: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                captured.lock().unwrap().push(read_request(&mut stream));
                stream.write_all(&response).unwrap();
                stream.flush().unwrap();
            }
        });
        Self {
            url: format!("http://{address}/{FILE_NAME}"),
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

fn wait_until(label: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate() {
        assert!(Instant::now() < deadline, "{label}");
        thread::sleep(Duration::from_millis(10));
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

fn response(status: &str, headers: &[(&str, String)], body: &[u8]) -> Vec<u8> {
    let mut value = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for (name, content) in headers {
        value.push_str(&format!("{name}: {content}\r\n"));
    }
    value.push_str("\r\n");
    let mut bytes = value.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn complete_response(body: &[u8]) -> Vec<u8> {
    response(
        "200 OK",
        &[("Content-Length", body.len().to_string())],
        body,
    )
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

#[derive(Default)]
struct RecordingCredentials {
    calls: AtomicUsize,
}

#[derive(Default)]
struct CountingValidator {
    calls: AtomicUsize,
    paths: Mutex<Vec<PathBuf>>,
}

impl CompletedArtifactValidator for CountingValidator {
    fn id(&self) -> ArtifactValidatorId {
        NISAR_RSLC_STRUCTURE_VALIDATOR_ID
    }

    fn version(&self) -> u32 {
        NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION
    }

    fn validate(
        &self,
        candidate_path: &Path,
        _asset: &ResolvedAsset,
        _context: &ArtifactValidationContext,
    ) -> Result<(), ArtifactValidationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.paths
            .lock()
            .unwrap()
            .push(candidate_path.to_path_buf());
        Ok(())
    }
}

impl CredentialProvider for RecordingCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "earthdata-default");
        self.calls.fetch_add(1, Ordering::SeqCst);
        CredentialMaterial::bearer("NISAR_LOOPBACK_TOKEN")
    }
}

struct LoopbackEarthdataResolver {
    url: String,
}

impl AssetResolver for LoopbackEarthdataResolver {
    fn resolver_id(&self) -> &'static str {
        "earthdata_https"
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::EarthdataHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.scheme == AssetAccessScheme::EarthdataHttps
    }

    fn resolve(
        &self,
        _reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        context.check_cancelled()?;
        let profile = descriptor
            .credential_profile
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let credential = context
            .credential_provider()
            .resolve(profile)
            .map_err(|_| ResolveError::credentials_missing())?;
        let mut asset = ResolvedAsset::new(&self.url, ResolvedMethod::Get)?;
        credential.apply_to(&mut asset)?;
        Ok(asset)
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::failed())
    }
}

fn worker(
    server_url: String,
    credentials: Arc<RecordingCredentials>,
    max_attempts: u32,
) -> ProductionDownloadWorker {
    worker_with_artifact_validators(
        server_url,
        credentials,
        max_attempts,
        ArtifactValidatorRegistry::standard(),
    )
}

fn worker_with_artifact_validators(
    server_url: String,
    credentials: Arc<RecordingCredentials>,
    max_attempts: u32,
    artifact_validators: ArtifactValidatorRegistry,
) -> ProductionDownloadWorker {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(LoopbackEarthdataResolver { url: server_url }))
        .unwrap();
    let runtime = ResolverExecutionRuntime::with_dependencies_and_artifact_validators_for_test(
        registry,
        credentials,
        Arc::new(FixedClock),
        artifact_validators,
    );
    let options = TransferOptions {
        max_attempts,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(2),
        chunk_size: 3,
        ..TransferOptions::default()
    };
    let transfer = Arc::new(transfer(max_attempts));
    ProductionDownloadWorker::new(options)
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, transfer)
}

fn transfer(max_attempts: u32) -> HttpTransfer {
    HttpTransfer::new_with_proxy_policy(
        TransferOptions {
            max_attempts,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            chunk_size: 3,
            ..TransferOptions::default()
        },
        ProxyPolicy::Direct,
    )
    .unwrap()
}

fn resolved_asset(url: &str, body: &[u8]) -> ResolvedAsset {
    ResolvedAsset::new(url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(body.len() as u64))
        .with_checksum(Some(
            ResolvedChecksum::new("SHA256", sha256_hex(body)).unwrap(),
        ))
        .with_asset_role(AssetRole::Primary)
}

fn transfer_request(url: &str, destination: &Path, body: &[u8], label: &str) -> TransferRequest {
    TransferRequest::new(
        format!("nisar-{label}"),
        format!("nisar-{label}-item"),
        url,
        destination.to_path_buf(),
        Some(body.len() as u64),
    )
    .unwrap()
}

fn execute_worker(
    output_root: &Path,
    server: &ScriptedServer,
    checksum: String,
    label: &str,
) -> (insar_core::DownloadItemResult, Arc<RecordingCredentials>) {
    execute_worker_with_body(output_root, server, body(), checksum, label)
}

fn execute_worker_with_body(
    output_root: &Path,
    server: &ScriptedServer,
    expected_body: &[u8],
    checksum: String,
    label: &str,
) -> (insar_core::DownloadItemResult, Arc<RecordingCredentials>) {
    let _test_guard = insar_download::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    insar_download::establish_hdf5_plugin_isolation()
        .expect("app-owned HDF5 plugin isolation is required before real HDF5 validation");
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), Arc::clone(&credentials), 1);
    let result = worker.execute(
        work(output_root, frozen_item(expected_body, checksum), label),
        WorkerControl::default(),
        &mut |_| {},
    );
    (result, credentials)
}

fn execute_worker_with_counting_validator(
    output_root: &Path,
    server: &ScriptedServer,
    checksum: String,
    label: &str,
    validator: Arc<CountingValidator>,
) -> insar_core::DownloadItemResult {
    let credentials = Arc::new(RecordingCredentials::default());
    let artifact_validators = ArtifactValidatorRegistry::with_validator_for_test(
        NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING,
        validator,
    );
    let worker =
        worker_with_artifact_validators(server.url.clone(), credentials, 1, artifact_validators);
    worker.execute(
        work(output_root, frozen_item(body(), checksum), label),
        WorkerControl::default(),
        &mut |_| {},
    )
}

/// L3 gate (2026-10-19): a complete NISAR product whose destination is longer
/// than the Windows MAX_PATH limit (260 characters) is validated and promoted
/// like any other. Before, HDF5 could not open it and it was refused as
/// NISAR_HDF5_STRUCTURE_INVALID, and its promotion failed as well.
#[test]
fn a_complete_product_under_a_path_longer_than_max_path_is_validated_and_promoted() {
    let temp = TempDirectory::new("long-destination");
    let mut output_root = temp.0.clone();
    while destination(&output_root).as_os_str().len() <= 280 {
        output_root.push("o".repeat(30));
    }
    fs::create_dir_all(&output_root).unwrap();
    let server = ScriptedServer::new(vec![complete_response(body())]);
    let (result, _credentials) = execute_worker(
        &output_root,
        &server,
        sha256_hex(body()),
        "long-destination",
    );

    assert_eq!(
        result.outcome,
        DownloadOutcome::Completed,
        "{:?}",
        result.failure
    );
    assert_eq!(result.destination, destination(&output_root));
    assert_eq!(fs::read(&result.destination).unwrap(), body());
    assert!(!part_path(&result.destination).exists());
    server.finish();
}

#[test]
fn complete_hdf5_resolves_credentials_once_and_promotes_atomically() {
    let temp = TempDirectory::new("complete");
    let server = ScriptedServer::new(vec![complete_response(body())]);
    let (result, credentials) = execute_worker(&temp.0, &server, sha256_hex(body()), "complete");

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(result.destination, destination(&temp.0));
    assert_eq!(fs::read(&result.destination).unwrap(), body());
    assert!(!part_path(&result.destination).exists());
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
    let request = server.finish()[0].to_ascii_lowercase();
    assert!(request.contains("authorization: bearer nisar_loopback_token"));
}

#[test]
fn legacy_v1_frozen_complete_product_still_downloads_without_v2_profile_upgrade() {
    let fixtures = FixtureDirectory::new("legacy-v1-body");
    let path = fixtures.file("legacy-v1");
    create_valid_fixture_for_granule(&path, FrequencyScope::AOnly, DISPLAY_NAME);
    let legacy_body = fs::read(path).expect("read legacy @1 synthetic RSLC HDF5");

    let temp = TempDirectory::new("legacy-v1-download");
    let server = ScriptedServer::new(vec![complete_response(&legacy_body)]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), Arc::clone(&credentials), 1);
    let mut legacy_item = frozen_item(&legacy_body, sha256_hex(&legacy_body));
    legacy_item.execution_policy.as_mut().unwrap().policy_id =
        NISAR_RESOLVER_POLICY_ID_V1.to_owned();
    legacy_item.metadata["resolver_policy_id"] = json!(NISAR_RESOLVER_POLICY_ID_V1);
    legacy_item.metadata["properties"]["resolver_policy_id"] = json!(NISAR_RESOLVER_POLICY_ID_V1);
    legacy_item.metadata["properties"]
        .as_object_mut()
        .unwrap()
        .remove("crid");
    legacy_item.metadata["properties"]
        .as_object_mut()
        .unwrap()
        .remove("product_specification_version");

    let result = worker.execute(
        work(&temp.0, legacy_item, "legacy-v1-download"),
        WorkerControl::default(),
        &mut |_| {},
    );
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(&result.destination).unwrap(), legacy_body);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn pause_stalls_the_hdf5_part_then_resumes_without_early_publication() {
    let temp = TempDirectory::new("pause");
    let destination = destination(&temp.0);
    let server = ScriptedServer::new(vec![complete_response(body())]);
    let asset = resolved_asset(&server.url, body());
    let request = transfer_request(&server.url, &destination, body(), "pause");
    let paused = Arc::new(AtomicBool::new(false));
    let paused_worker = Arc::clone(&paused);
    let pause_once = Arc::new(AtomicBool::new(true));
    let pause_once_worker = Arc::clone(&pause_once);
    let (paused_sender, paused_receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let transfer = transfer(1);
        let mut session = transfer.resolved_session();
        transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut session,
            &|| false,
            &|| paused_worker.load(Ordering::Acquire),
            &mut |update| {
                if let crate::TransferUpdate::Progress(progress) = update {
                    if progress.bytes_completed >= 3
                        && pause_once_worker.swap(false, Ordering::AcqRel)
                    {
                        paused_worker.store(true, Ordering::Release);
                        paused_sender.send(()).unwrap();
                    }
                }
            },
        )
    });

    paused_receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    wait_until("paused NISAR part was not written", || {
        fs::metadata(part_path(&destination)).is_ok_and(|metadata| metadata.len() == 3)
    });
    let paused_size = fs::metadata(part_path(&destination)).unwrap().len();
    thread::sleep(Duration::from_millis(120));
    assert_eq!(
        fs::metadata(part_path(&destination)).unwrap().len(),
        paused_size
    );
    assert!(!destination.exists());

    paused.store(false, Ordering::Release);
    let result = handle.join().unwrap();
    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    assert_eq!(fs::read(&destination).unwrap(), body());
    assert!(!part_path(&destination).exists());
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn midstream_cancel_preserves_part_and_restart_uses_range() {
    let temp = TempDirectory::new("cancel-restart");
    let destination = destination(&temp.0);
    let first_server = ScriptedServer::new(vec![complete_response(body())]);
    let first_asset = resolved_asset(&first_server.url, body());
    let first_request = transfer_request(&first_server.url, &destination, body(), "cancel-restart");
    let cancelled = AtomicBool::new(false);
    let cancel_once = AtomicBool::new(true);
    let first_transfer = transfer(1);
    let mut first_session = first_transfer.resolved_session();
    let first_result = first_transfer.download_resolved(
        &first_request,
        &first_asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut first_session,
        &|| cancelled.load(Ordering::Acquire),
        &|| false,
        &mut |update| {
            if let crate::TransferUpdate::Progress(progress) = update {
                if progress.bytes_completed >= 3 && cancel_once.swap(false, Ordering::AcqRel) {
                    cancelled.store(true, Ordering::Release);
                }
            }
        },
    );
    assert!(matches!(
        first_result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Interrupted
    ));
    assert!(!destination.exists());
    let split = fs::metadata(part_path(&destination)).unwrap().len() as usize;
    assert_eq!(split, 3);
    assert_eq!(first_server.finish().len(), 1);

    let second_server = ScriptedServer::new(vec![response(
        "206 Partial Content",
        &[
            ("Content-Length", (body().len() - split).to_string()),
            (
                "Content-Range",
                format!("bytes {split}-{}/{}", body().len() - 1, body().len()),
            ),
        ],
        &body()[split..],
    )]);
    let second_asset = resolved_asset(&second_server.url, body());
    let second_request =
        transfer_request(&second_server.url, &destination, body(), "cancel-restart");
    let second_transfer = transfer(1);
    let mut second_session = second_transfer.resolved_session();
    let second_result = second_transfer.download_resolved(
        &second_request,
        &second_asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut second_session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(
        second_result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Completed
    ));
    assert_eq!(fs::read(&destination).unwrap(), body());
    let request = second_server.finish()[0].to_ascii_lowercase();
    assert!(request.contains(&format!("range: bytes={split}-")));
    assert!(!request.contains("if-range:"));
}

#[test]
fn skip_complete_finalize_part_and_corrupt_part_recovery_are_atomic() {
    let skip_temp = TempDirectory::new("skip-complete");
    let skip_destination = destination(&skip_temp.0);
    fs::write(&skip_destination, body()).unwrap();
    let skip_server = ScriptedServer::new(vec![]);
    let (skip, _) = execute_worker(
        &skip_temp.0,
        &skip_server,
        sha256_hex(body()),
        "skip-complete",
    );
    assert_eq!(skip.outcome, DownloadOutcome::Skipped);
    assert_eq!(fs::read(&skip_destination).unwrap(), body());
    assert!(skip_server.finish().is_empty());

    let finalize_temp = TempDirectory::new("finalize-part");
    let finalize_destination = destination(&finalize_temp.0);
    fs::write(part_path(&finalize_destination), body()).unwrap();
    let finalize_server = ScriptedServer::new(vec![]);
    let (finalized, _) = execute_worker(
        &finalize_temp.0,
        &finalize_server,
        sha256_hex(body()),
        "finalize-part",
    );
    assert_eq!(finalized.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(&finalize_destination).unwrap(), body());
    assert!(!part_path(&finalize_destination).exists());
    assert!(finalize_server.finish().is_empty());

    let corrupt_temp = TempDirectory::new("corrupt-part");
    let corrupt_destination = destination(&corrupt_temp.0);
    let wrong = vec![b'x'; body().len()];
    fs::write(part_path(&corrupt_destination), &wrong).unwrap();
    let corrupt_server = ScriptedServer::new(vec![]);
    let (corrupt, _) = execute_worker(
        &corrupt_temp.0,
        &corrupt_server,
        sha256_hex(body()),
        "corrupt-part",
    );
    assert_eq!(corrupt.outcome, DownloadOutcome::Failed);
    assert!(!corrupt_destination.exists());
    assert_eq!(fs::read(part_path(&corrupt_destination)).unwrap(), wrong);
    assert!(corrupt_server.finish().is_empty());
}

#[test]
fn policy_bound_validator_runs_once_after_integrity_on_every_completion_path() {
    let checksum = sha256_hex(body());

    let skip_temp = TempDirectory::new("count-validator-skip");
    let skip_destination = destination(&skip_temp.0);
    fs::write(&skip_destination, body()).unwrap();
    let skip_before = sha256_hex(&fs::read(&skip_destination).unwrap());
    let skip_server = ScriptedServer::new(vec![]);
    let skip_validator = Arc::new(CountingValidator::default());
    let skip = execute_worker_with_counting_validator(
        &skip_temp.0,
        &skip_server,
        checksum.clone(),
        "count-validator-skip",
        Arc::clone(&skip_validator),
    );
    assert_eq!(skip.outcome, DownloadOutcome::Skipped);
    assert_eq!(skip_validator.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *skip_validator.paths.lock().unwrap(),
        vec![skip_destination.clone()]
    );
    assert_eq!(
        sha256_hex(&fs::read(&skip_destination).unwrap()),
        skip_before
    );
    assert!(skip_server.finish().is_empty());

    let finalize_temp = TempDirectory::new("count-validator-finalize");
    let finalize_destination = destination(&finalize_temp.0);
    let finalize_candidate = part_path(&finalize_destination);
    fs::write(&finalize_candidate, body()).unwrap();
    let finalize_server = ScriptedServer::new(vec![]);
    let finalize_validator = Arc::new(CountingValidator::default());
    let finalized = execute_worker_with_counting_validator(
        &finalize_temp.0,
        &finalize_server,
        checksum.clone(),
        "count-validator-finalize",
        Arc::clone(&finalize_validator),
    );
    assert_eq!(finalized.outcome, DownloadOutcome::Completed);
    assert_eq!(finalize_validator.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *finalize_validator.paths.lock().unwrap(),
        vec![finalize_candidate]
    );
    assert_eq!(fs::read(&finalize_destination).unwrap(), body());
    assert!(finalize_server.finish().is_empty());

    let fresh_temp = TempDirectory::new("count-validator-fresh");
    let fresh_destination = destination(&fresh_temp.0);
    let fresh_candidate = part_path(&fresh_destination);
    let fresh_server = ScriptedServer::new(vec![complete_response(body())]);
    let fresh_validator = Arc::new(CountingValidator::default());
    let fresh = execute_worker_with_counting_validator(
        &fresh_temp.0,
        &fresh_server,
        checksum.clone(),
        "count-validator-fresh",
        Arc::clone(&fresh_validator),
    );
    assert_eq!(fresh.outcome, DownloadOutcome::Completed);
    assert_eq!(fresh_validator.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *fresh_validator.paths.lock().unwrap(),
        vec![fresh_candidate]
    );
    assert_eq!(fs::read(&fresh_destination).unwrap(), body());
    assert_eq!(fresh_server.finish().len(), 1);

    let resume_temp = TempDirectory::new("count-validator-resume");
    let resume_destination = destination(&resume_temp.0);
    let resume_candidate = part_path(&resume_destination);
    let split = body().len() / 2;
    fs::write(&resume_candidate, &body()[..split]).unwrap();
    let resume_server = ScriptedServer::new(vec![response(
        "206 Partial Content",
        &[
            ("Content-Length", (body().len() - split).to_string()),
            (
                "Content-Range",
                format!("bytes {split}-{}/{}", body().len() - 1, body().len()),
            ),
        ],
        &body()[split..],
    )]);
    let resume_validator = Arc::new(CountingValidator::default());
    let resumed = execute_worker_with_counting_validator(
        &resume_temp.0,
        &resume_server,
        checksum,
        "count-validator-resume",
        Arc::clone(&resume_validator),
    );
    assert_eq!(resumed.outcome, DownloadOutcome::Completed);
    assert_eq!(resume_validator.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *resume_validator.paths.lock().unwrap(),
        vec![resume_candidate]
    );
    assert_eq!(fs::read(&resume_destination).unwrap(), body());
    assert_eq!(resume_server.finish().len(), 1);

    let checksum_temp = TempDirectory::new("count-validator-checksum-failure");
    let checksum_destination = destination(&checksum_temp.0);
    let checksum_server = ScriptedServer::new(vec![complete_response(body())]);
    let checksum_validator = Arc::new(CountingValidator::default());
    let checksum_failure = execute_worker_with_counting_validator(
        &checksum_temp.0,
        &checksum_server,
        "0".repeat(64),
        "count-validator-checksum-failure",
        Arc::clone(&checksum_validator),
    );
    assert_eq!(checksum_failure.outcome, DownloadOutcome::Failed);
    assert_eq!(checksum_validator.calls.load(Ordering::SeqCst), 0);
    assert!(checksum_validator.paths.lock().unwrap().is_empty());
    assert!(!checksum_destination.exists());
    assert_eq!(fs::read(part_path(&checksum_destination)).unwrap(), body());
    assert_eq!(checksum_server.finish().len(), 1);
}

#[test]
fn structural_failure_is_non_retryable_and_preserves_every_completed_candidate() {
    const INVALID_STRUCTURE: &[u8] = b"\x89HDF\r\n\x1a\n";
    let expected_checksum = sha256_hex(INVALID_STRUCTURE);

    let skip_temp = TempDirectory::new("invalid-structure-skip");
    let skip_destination = destination(&skip_temp.0);
    fs::write(&skip_destination, INVALID_STRUCTURE).unwrap();
    let skip_server = ScriptedServer::new(vec![]);
    let (skip, _) = execute_worker_with_body(
        &skip_temp.0,
        &skip_server,
        INVALID_STRUCTURE,
        expected_checksum.clone(),
        "invalid-structure-skip",
    );
    assert_eq!(skip.outcome, DownloadOutcome::Failed);
    assert_eq!(
        skip.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("NISAR_HDF5_STRUCTURE_INVALID")
    );
    assert!(skip
        .failure
        .as_ref()
        .is_some_and(|failure| !failure.retryable));
    assert_eq!(fs::read(&skip_destination).unwrap(), INVALID_STRUCTURE);
    assert!(!part_path(&skip_destination).exists());
    assert!(skip_server.finish().is_empty());

    let finalize_temp = TempDirectory::new("invalid-structure-finalize");
    let finalize_destination = destination(&finalize_temp.0);
    fs::write(part_path(&finalize_destination), INVALID_STRUCTURE).unwrap();
    let finalize_server = ScriptedServer::new(vec![]);
    let (finalized, _) = execute_worker_with_body(
        &finalize_temp.0,
        &finalize_server,
        INVALID_STRUCTURE,
        expected_checksum.clone(),
        "invalid-structure-finalize",
    );
    assert_eq!(finalized.outcome, DownloadOutcome::Failed);
    assert_eq!(
        finalized
            .failure
            .as_ref()
            .map(|failure| failure.code.as_str()),
        Some("NISAR_HDF5_STRUCTURE_INVALID")
    );
    assert!(!finalize_destination.exists());
    assert_eq!(
        fs::read(part_path(&finalize_destination)).unwrap(),
        INVALID_STRUCTURE
    );
    assert!(finalize_server.finish().is_empty());

    let fresh_temp = TempDirectory::new("invalid-structure-fresh");
    let fresh_destination = destination(&fresh_temp.0);
    let fresh_server = ScriptedServer::new(vec![complete_response(INVALID_STRUCTURE)]);
    let (fresh, _) = execute_worker_with_body(
        &fresh_temp.0,
        &fresh_server,
        INVALID_STRUCTURE,
        expected_checksum.clone(),
        "invalid-structure-fresh",
    );
    assert_eq!(fresh.outcome, DownloadOutcome::Failed);
    assert_eq!(fresh.attempt_count, 1);
    assert_eq!(
        fresh.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("NISAR_HDF5_STRUCTURE_INVALID")
    );
    assert!(!fresh_destination.exists());
    assert_eq!(
        fs::read(part_path(&fresh_destination)).unwrap(),
        INVALID_STRUCTURE
    );
    assert_eq!(fresh_server.finish().len(), 1);

    let resume_temp = TempDirectory::new("invalid-structure-resume");
    let resume_destination = destination(&resume_temp.0);
    let split = 3;
    fs::write(part_path(&resume_destination), &INVALID_STRUCTURE[..split]).unwrap();
    let resume_server = ScriptedServer::new(vec![response(
        "206 Partial Content",
        &[
            (
                "Content-Length",
                (INVALID_STRUCTURE.len() - split).to_string(),
            ),
            (
                "Content-Range",
                format!(
                    "bytes {split}-{}/{}",
                    INVALID_STRUCTURE.len() - 1,
                    INVALID_STRUCTURE.len()
                ),
            ),
        ],
        &INVALID_STRUCTURE[split..],
    )]);
    let (resumed, _) = execute_worker_with_body(
        &resume_temp.0,
        &resume_server,
        INVALID_STRUCTURE,
        expected_checksum,
        "invalid-structure-resume",
    );
    assert_eq!(resumed.outcome, DownloadOutcome::Failed);
    assert_eq!(resumed.attempt_count, 1);
    assert_eq!(
        resumed
            .failure
            .as_ref()
            .map(|failure| failure.code.as_str()),
        Some("NISAR_HDF5_STRUCTURE_INVALID")
    );
    assert!(!resume_destination.exists());
    assert_eq!(
        fs::read(part_path(&resume_destination)).unwrap(),
        INVALID_STRUCTURE
    );
    let request = resume_server.finish()[0].to_ascii_lowercase();
    assert!(request.contains(&format!("range: bytes={split}-")));
}

/// The partials that were set aside next to `destination`, with their bytes.
fn set_aside_partials(destination: &Path) -> Vec<Vec<u8>> {
    let prefix = format!(
        "{}.part.eostage-conflict-",
        destination.file_name().unwrap().to_string_lossy()
    );
    fs::read_dir(destination.parent().unwrap())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| fs::read(entry.path()).unwrap())
        .collect()
}

#[test]
fn ignored_range_200_sets_the_partial_aside_and_integrity_failures_never_publish_bad_hdf5() {
    let restart_temp = TempDirectory::new("restart-200");
    let restart_destination = destination(&restart_temp.0);
    fs::write(part_path(&restart_destination), b"stale-prefix").unwrap();
    let restart_server =
        ScriptedServer::new(vec![complete_response(body()), complete_response(body())]);
    let (restart, _) = execute_worker(
        &restart_temp.0,
        &restart_server,
        sha256_hex(body()),
        "restart-200",
    );
    // Even with one attempt the refused prefix is kept aside and the product
    // downloads again from zero.
    assert_eq!(restart.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(&restart_destination).unwrap(), body());
    assert!(!part_path(&restart_destination).exists());
    assert_eq!(
        set_aside_partials(&restart_destination),
        vec![b"stale-prefix".to_vec()]
    );
    let restart_requests = restart_server.finish();
    assert_eq!(restart_requests.len(), 2);
    assert!(restart_requests[0]
        .to_ascii_lowercase()
        .contains("range: bytes="));
    assert!(!restart_requests[1].to_ascii_lowercase().contains("range:"));

    let checksum_temp = TempDirectory::new("bad-checksum");
    let checksum_server = ScriptedServer::new(vec![complete_response(body())]);
    let (checksum_failure, _) = execute_worker(
        &checksum_temp.0,
        &checksum_server,
        "0".repeat(64),
        "bad-checksum",
    );
    assert_eq!(checksum_failure.outcome, DownloadOutcome::Failed);
    assert!(!checksum_failure.destination.exists());
    checksum_server.finish();

    let size_temp = TempDirectory::new("bad-size");
    let size_server = ScriptedServer::new(vec![complete_response(body())]);
    let mut item = frozen_item(body(), sha256_hex(body()));
    let expected_size = body().len() as u64 + 1;
    item.asset_access.as_mut().unwrap().expected_size_bytes = Some(expected_size);
    item.metadata["assets"][0]["size_bytes"] = json!(expected_size);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(size_server.url.clone(), credentials, 1);
    let size_failure = worker.execute(
        work(&size_temp.0, item, "bad-size"),
        WorkerControl::default(),
        &mut |_| {},
    );
    assert_eq!(size_failure.outcome, DownloadOutcome::Failed);
    assert!(!size_failure.destination.exists());
    size_server.finish();
}

#[test]
fn changed_etag_on_partial_or_full_retry_never_truncates_the_original_part() {
    for second_status in ["206 Partial Content", "200 OK"] {
        let label = if second_status.starts_with("206") {
            "changed-etag-206"
        } else {
            "changed-etag-200"
        };
        let split = 9;
        let temp = TempDirectory::new(label);
        let destination = destination(&temp.0);
        let second_headers = if second_status.starts_with("206") {
            vec![
                ("Content-Length", (body().len() - split).to_string()),
                (
                    "Content-Range",
                    format!("bytes {split}-{}/{}", body().len() - 1, body().len()),
                ),
                ("ETag", "\"changed\"".to_owned()),
            ]
        } else {
            vec![
                ("Content-Length", body().len().to_string()),
                ("ETag", "\"changed\"".to_owned()),
            ]
        };
        let second_body = if second_status.starts_with("206") {
            &body()[split..]
        } else {
            body()
        };
        let mut script = vec![
            response(
                "200 OK",
                &[
                    ("Content-Length", body().len().to_string()),
                    ("ETag", "\"first\"".to_owned()),
                ],
                &body()[..split],
            ),
            response(second_status, &second_headers, second_body),
        ];
        if second_status == "200 OK" {
            // The refused resume is followed by a transfer from zero.
            script.push(complete_response(body()));
        }
        let server = ScriptedServer::new(script);
        let credentials = Arc::new(RecordingCredentials::default());
        let worker = worker(server.url.clone(), credentials, 2);
        let result = worker.execute(
            work(&temp.0, frozen_item(body(), sha256_hex(body())), label),
            WorkerControl::default(),
            &mut |_| {},
        );

        let requests = server.finish();
        if second_status == "200 OK" {
            // A full 200 refuses the resume: the original prefix is kept aside
            // and the product downloads again from zero.
            assert_eq!(result.outcome, DownloadOutcome::Completed);
            assert_eq!(fs::read(&destination).unwrap(), body());
            assert!(!part_path(&destination).exists());
            assert_eq!(
                set_aside_partials(&destination),
                vec![body()[..split].to_vec()]
            );
            assert_eq!(requests.len(), 3);
            assert!(!requests[2].to_ascii_lowercase().contains("range:"));
        } else {
            assert_eq!(result.outcome, DownloadOutcome::Failed);
            assert!(!destination.exists());
            assert_eq!(fs::read(part_path(&destination)).unwrap(), &body()[..split]);
            assert_eq!(requests.len(), 2);
        }
        let second = requests[1].to_ascii_lowercase();
        assert!(second.contains(&format!("range: bytes={split}-")));
        if second_status == "200 OK" {
            assert!(second.contains("if-range: \"first\""));
        }
    }
}

/// Downloader D3 (local-first dedup, Owner rule 2026-10-20): an existing
/// complete product whose checksum differs from the frozen one is renamed
/// aside and kept; the asset downloads again and the policy-bound validator
/// checks the new candidate once. (A file with the frozen checksum is the
/// service's own bytes: it is reused only after the validator accepts it, and
/// a structural failure stays non-retryable, as tested above.)
#[test]
fn an_existing_product_with_another_checksum_is_kept_aside_and_downloaded() {
    let temp = TempDirectory::new("conflict-other-checksum");
    let destination = destination(&temp.0);
    let other = vec![b'y'; body().len()];
    fs::write(&destination, &other).unwrap();
    let server = ScriptedServer::new(vec![complete_response(body())]);
    let validator = Arc::new(CountingValidator::default());
    let result = execute_worker_with_counting_validator(
        &temp.0,
        &server,
        sha256_hex(body()),
        "conflict-other-checksum",
        Arc::clone(&validator),
    );
    assert_eq!(
        result.outcome,
        DownloadOutcome::Completed,
        "{:?}",
        result.failure
    );
    assert_eq!(validator.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(&destination).unwrap(), body());
    let prefix = format!(
        "{}.eostage-conflict-",
        destination.file_name().unwrap().to_string_lossy()
    );
    let kept = fs::read_dir(destination.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&prefix)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kept.len(),
        1,
        "the differing product is kept under its conflict name"
    );
    assert_eq!(fs::read(&kept[0]).unwrap(), other);
    assert_eq!(server.finish().len(), 1);
}

/// Downloader D3-2 (pre-scan): a product with an artifact validation plan is
/// never reused by the pre-scan, in either phase, even with the frozen
/// checksum: the worker's validated route decides (deferred_reuse).
#[test]
fn the_pre_scan_never_reuses_a_validated_product() {
    use crate::{DownloadWorker, LocalScanPhase, LocalScanVerdict};
    let temp = TempDirectory::new("prescan-validated");
    let destination = destination(&temp.0);
    fs::write(&destination, body()).unwrap();
    let server = ScriptedServer::new(vec![]);
    let worker = worker(
        server.url.clone(),
        Arc::new(RecordingCredentials::default()),
        1,
    );
    let work = work(
        &temp.0,
        frozen_item(body(), sha256_hex(body())),
        "prescan-validated",
    );
    for phase in [LocalScanPhase::Cheap, LocalScanPhase::Digest] {
        assert_eq!(
            worker.scan_local(&work, phase, &WorkerControl::default(), None),
            LocalScanVerdict::Release
        );
    }
    assert_eq!(fs::read(&destination).unwrap(), body());
    assert!(server.finish().is_empty());
}
