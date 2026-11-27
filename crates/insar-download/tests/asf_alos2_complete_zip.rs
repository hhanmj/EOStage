use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, AssetRole,
    DownloadOutcome, FrozenAssetAccess, FrozenTaskSnapshot, ProxyPolicy, SnapshotItem,
    StableAssetReference,
};
use insar_download::{
    part_path, AssetResolver, AssetResolverRegistry, AuthRefreshPolicyId, Clock,
    CredentialMaterial, CredentialProvider, CredentialProviderError, DestinationLeaseController,
    DownloadWork, DownloadWorker, HttpTransfer, ProductionDownloadWorker, ResolveContext,
    ResolveError, ResolvedAsset, ResolvedChecksum, ResolvedMethod, ResolvedTransferResult,
    ResolverExecutionRuntime, ShadowAccessIntent, ShadowResolveContext, TransferOptions,
    TransferRequest, WorkerControl,
};
use insar_provider::catalog::{
    ALOS2_FILENAME_RULE_ID, ALOS2_IDENTITY_RULE_ID, ALOS2_LOGICAL_DATASET,
    ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID, ALOS2_RESOLVER_POLICY_ID,
};
use md5::{Digest, Md5};
use serde_json::{json, Map, Value};

const PRODUCT_ID: &str = "G3463311647-ASF";
const DISPLAY_NAME: &str = "ALOS2478123150-230331-WBDR1.1__D";
const FILE_NAME: &str = "ALOS2478123150-230331-WBDR1.1__D.zip";
const OFFICIAL_HREF: &str =
    "https://datapool.asf.alaska.edu/ALOS2/ALOS2478123150-230331-WBDR1.1__D.zip";

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-alos2-complete-zip-{label}-{}-{nonce}",
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

struct GatedServer {
    url: String,
    first_chunk: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl GatedServer {
    fn new(body: &[u8], split: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_vec();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let (first_chunk_sender, first_chunk) = mpsc::channel();
        let (release, release_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            captured.lock().unwrap().push(read_request(&mut stream));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body[..split]).unwrap();
            stream.flush().unwrap();
            first_chunk_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
            stream.write_all(&body[split..]).unwrap();
            stream.flush().unwrap();
        });
        Self {
            url: format!("http://{address}/{FILE_NAME}"),
            first_chunk,
            release,
            requests,
            worker: Some(worker),
        }
    }

    fn wait_for_first_chunk(&self) {
        self.first_chunk
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
    }

    fn release(&self) {
        self.release.send(()).unwrap();
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

fn md5_hex(body: &[u8]) -> String {
    format!("{:x}", Md5::digest(body))
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

impl CredentialProvider for RecordingCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "earthdata-default");
        self.calls.fetch_add(1, Ordering::SeqCst);
        CredentialMaterial::bearer("ALOS2_LOOPBACK_TOKEN")
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

fn item(body: &[u8], checksum: String) -> SnapshotItem {
    let product_key =
        provider_product_key(ALOS2_PROVIDER_ID, ALOS2_NATIVE_COLLECTION_ID, PRODUCT_ID).unwrap();
    let asset_key = FILE_NAME.to_owned();
    let stable_key = stable_asset_key(&product_key, &asset_key).unwrap();
    let descriptor = AssetAccessDescriptor {
        resolver_id: "earthdata_https".to_owned(),
        scheme: AssetAccessScheme::EarthdataHttps,
        stable_href: Some(OFFICIAL_HREF.to_owned()),
        requires_credentials: true,
        credential_profile: Some("earthdata-default".to_owned()),
        resolver_parameters: AssetResolverParameters {},
    };
    let access = FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: ALOS2_PROVIDER_ID.to_owned(),
            collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
            provider_product_id: PRODUCT_ID.to_owned(),
            provider_product_key: product_key.clone(),
            canonical_product_key: None,
            asset_key: asset_key.clone(),
            stable_asset_key: stable_key.clone(),
        },
        descriptor: descriptor.clone(),
        role: AssetRole::Primary,
        file_name: Some(FILE_NAME.to_owned()),
        expected_size_bytes: Some(body.len() as u64),
        checksum_algorithm: Some("MD5".to_owned()),
        checksum_value: Some(checksum.clone()),
    };
    let properties = json!({
        "provider_product_key": product_key,
        "provider_product_id": PRODUCT_ID,
        "cmr_granule_concept_id": PRODUCT_ID,
        "cmr_native_id": DISPLAY_NAME,
        "producer_granule_id": "ALOS2478123150-230331",
        "logical_dataset": ALOS2_LOGICAL_DATASET,
        "collection_concept_id": ALOS2_NATIVE_COLLECTION_ID,
        "collection_short_name": "ALOS2_L1_PSR2",
        "collection_version": "1",
        "collection_profile_id": ALOS2_PROFILE_ID,
        "identity_rule_id": ALOS2_IDENTITY_RULE_ID,
        "filename_rule_id": ALOS2_FILENAME_RULE_ID,
        "resolver_policy_id": ALOS2_RESOLVER_POLICY_ID,
        "platform": "ALOS-2",
        "instrument": "PALSAR-2",
        "processing_level": "1.1",
        "beam_mode": "WBD",
        "polarizations": ["HH", "HV"],
        "asset_integrity": {
            FILE_NAME: {
                "file_name": FILE_NAME,
                "size_bytes": body.len(),
                "algorithm": "MD5",
                "value": checksum
            }
        },
        "asset_routes": {
            FILE_NAME: [{"scheme":"https", "locator":OFFICIAL_HREF}]
        },
        "preview_available": false
    });
    SnapshotItem {
        item_id: access.reference.provider_product_key.clone(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        display_name: DISPLAY_NAME.to_owned(),
        acquisition_datetime: Some("2023-03-31T19:20:46Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::from_iter([
            (
                "dataset".to_owned(),
                Value::String(ALOS2_LOGICAL_DATASET.to_owned()),
            ),
            (
                "logical_dataset".to_owned(),
                Value::String(ALOS2_LOGICAL_DATASET.to_owned()),
            ),
            (
                "native_collection_id".to_owned(),
                Value::String(ALOS2_NATIVE_COLLECTION_ID.to_owned()),
            ),
            (
                "collection_profile_id".to_owned(),
                Value::String(ALOS2_PROFILE_ID.to_owned()),
            ),
            (
                "identity_rule_id".to_owned(),
                Value::String(ALOS2_IDENTITY_RULE_ID.to_owned()),
            ),
            (
                "filename_rule_id".to_owned(),
                Value::String(ALOS2_FILENAME_RULE_ID.to_owned()),
            ),
            (
                "delivery_model".to_owned(),
                Value::String("archive_first".to_owned()),
            ),
            (
                "resolver_policy_id".to_owned(),
                Value::String(ALOS2_RESOLVER_POLICY_ID.to_owned()),
            ),
            (
                "provider_product_id".to_owned(),
                Value::String(PRODUCT_ID.to_owned()),
            ),
            (
                "provider_product_key".to_owned(),
                Value::String(access.reference.provider_product_key.clone()),
            ),
            (
                "product_family".to_owned(),
                Value::String("ALOS2_L1_PSR2".to_owned()),
            ),
            (
                "processing_level".to_owned(),
                Value::String("1.1".to_owned()),
            ),
            ("properties".to_owned(), properties),
            (
                "assets".to_owned(),
                json!([{
                    "asset_key": asset_key,
                    "stable_asset_key": stable_key,
                    "access_descriptor": descriptor,
                    "role": "primary",
                    "native_roles": ["archive"],
                    "url": null,
                    "file_name": FILE_NAME,
                    "media_type": "application/zip",
                    "size_bytes": body.len()
                }]),
            ),
        ]),
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: ALOS2_RESOLVER_POLICY_ID.to_owned(),
            collection_profile_id: Some(ALOS2_PROFILE_ID.to_owned()),
        }),
    }
}

fn work(output_root: &Path, item: SnapshotItem, label: &str) -> DownloadWork {
    let task: FrozenTaskSnapshot = serde_json::from_value(json!({
        "schema_version": 1,
        "task_id": format!("alos2-{label}"),
        "batch_id": format!("alos2-{label}-batch"),
        "provider_id": ALOS2_PROVIDER_ID,
        "created_at": "2026-07-29T00:00:00Z",
        "aoi_name": "ALOS-2 fixture AOI",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,0.0]]]
        },
        "query": {"dataset": ALOS2_LOGICAL_DATASET},
        "candidates": [item.clone()],
        "initial_selected_item_ids": [item.item_id.clone()],
        "output_root": output_root.to_string_lossy(),
        "layout": "sar_dataset_processing"
    }))
    .expect("frozen ALOS-2 task");
    DownloadWork { task, item }
}

fn destination(output_root: &Path) -> PathBuf {
    let destination = output_root
        .join(ALOS2_LOGICAL_DATASET.to_ascii_uppercase())
        .join("1_1")
        .join(FILE_NAME);
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    destination
}

fn worker(
    server_url: String,
    credentials: Arc<RecordingCredentials>,
    max_attempts: u32,
) -> ProductionDownloadWorker {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(LoopbackEarthdataResolver { url: server_url }))
        .unwrap();
    let runtime =
        ResolverExecutionRuntime::with_dependencies(registry, credentials, Arc::new(FixedClock));
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
        .with_checksum(Some(ResolvedChecksum::new("MD5", md5_hex(body)).unwrap()))
        .with_asset_role(AssetRole::Primary)
}

fn transfer_request(url: &str, destination: &Path, body: &[u8], label: &str) -> TransferRequest {
    TransferRequest::new(
        format!("alos2-{label}"),
        format!("alos2-{label}-item"),
        url,
        destination.to_path_buf(),
        Some(body.len() as u64),
    )
    .unwrap()
}

#[test]
fn complete_zip_download_resolves_credentials_once_and_promotes_atomically() {
    let body = b"alos2-complete-zip";
    let temp = TempDirectory::new("complete");
    let server = ScriptedServer::new(vec![complete_response(body)]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), Arc::clone(&credentials), 1);
    let result = worker.execute(
        work(&temp.0, item(body, md5_hex(body)), "complete"),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(&result.destination).unwrap(), body);
    assert!(!part_path(&result.destination).exists());
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
    let request = server.finish()[0].to_ascii_lowercase();
    assert!(request.contains("authorization: bearer alos2_loopback_token"));
}

#[test]
fn alos2_pause_stalls_the_part_then_resumes_without_publishing_early() {
    let body = b"alos2-complete-zip";
    let temp = TempDirectory::new("pause");
    let destination = destination(&temp.0);
    let server = ScriptedServer::new(vec![complete_response(body)]);
    let asset = resolved_asset(&server.url, body);
    let request = transfer_request(&server.url, &destination, body, "pause");
    let paused = Arc::new(AtomicBool::new(false));
    let paused_worker = Arc::clone(&paused);
    let pause_once = Arc::new(AtomicBool::new(true));
    let pause_once_worker = Arc::clone(&pause_once);
    let (paused_sender, paused_receiver) = mpsc::channel();
    let transfer = transfer(1);
    let handle = thread::spawn(move || {
        let mut session = transfer.resolved_session();
        transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut session,
            &|| false,
            &|| paused_worker.load(Ordering::Acquire),
            &mut |update| {
                if let insar_download::TransferUpdate::Progress(progress) = update {
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
        .expect("ALOS-2 transfer reaches the pause boundary");
    wait_until("paused ALOS-2 part was not written", || {
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
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert!(!part_path(&destination).exists());
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn alos2_midstream_cancel_preserves_part_and_restart_completes() {
    let body = b"alos2-complete-zip";
    let temp = TempDirectory::new("cancel-restart");
    let destination = destination(&temp.0);
    let first_server = ScriptedServer::new(vec![complete_response(body)]);
    let first_asset = resolved_asset(&first_server.url, body);
    let first_request = transfer_request(&first_server.url, &destination, body, "cancel-restart");
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
            if let insar_download::TransferUpdate::Progress(progress) = update {
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
            ("Content-Length", (body.len() - split).to_string()),
            (
                "Content-Range",
                format!("bytes {split}-{}/{}", body.len() - 1, body.len()),
            ),
        ],
        &body[split..],
    )]);
    let second_asset = resolved_asset(&second_server.url, body);
    let second_request = transfer_request(&second_server.url, &destination, body, "cancel-restart");
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
    assert_eq!(fs::read(&destination).unwrap(), body);
    let request = second_server.finish()[0].to_ascii_lowercase();
    assert!(request.contains(&format!("range: bytes={split}-")));
    assert!(!request.contains("if-range:"));
}

#[test]
fn alos2_same_destination_workers_serialize_and_successor_skips() {
    let body = b"alos2-complete-zip";
    let temp = TempDirectory::new("destination-lease");
    let destination = destination(&temp.0);
    let server = GatedServer::new(body, 3);
    let credentials = Arc::new(RecordingCredentials::default());
    let controller = DestinationLeaseController::new_for_qualification_test();
    let worker = Arc::new(
        worker(server.url.clone(), Arc::clone(&credentials), 1)
            .with_destination_lease_controller_for_test(Arc::clone(&controller)),
    );

    let first_worker = Arc::clone(&worker);
    let first_root = temp.0.clone();
    let first = thread::spawn(move || {
        first_worker.execute(
            work(
                &first_root,
                item(body, md5_hex(body)),
                "destination-lease-first",
            ),
            WorkerControl::default(),
            &mut |_| {},
        )
    });
    server.wait_for_first_chunk();
    wait_until("first ALOS-2 worker did not publish its part", || {
        fs::metadata(part_path(&destination)).is_ok_and(|metadata| metadata.len() == 3)
    });
    assert!(!destination.exists());

    let second_worker = Arc::clone(&worker);
    let second_root = temp.0.clone();
    let second = thread::spawn(move || {
        second_worker.execute(
            work(
                &second_root,
                item(body, md5_hex(body)),
                "destination-lease-second",
            ),
            WorkerControl::default(),
            &mut |_| {},
        )
    });
    wait_until(
        "second ALOS-2 worker did not queue for the destination",
        || {
            controller
                .snapshot_for_qualification_test()
                .queued_waiters()
                == 1
        },
    );
    assert!(!destination.exists());
    assert_eq!(fs::metadata(part_path(&destination)).unwrap().len(), 3);

    server.release();
    let first_result = first.join().unwrap();
    let second_result = second.join().unwrap();
    assert_eq!(first_result.outcome, DownloadOutcome::Completed);
    assert_eq!(second_result.outcome, DownloadOutcome::Skipped);
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert!(!part_path(&destination).exists());
    assert_eq!(server.finish().len(), 1);
    let lease = controller.snapshot_for_qualification_test();
    assert_eq!(lease.active_leases(), 0);
    assert_eq!(lease.queued_waiters(), 0);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn verified_existing_destination_skips_without_request() {
    let body = b"alos2-complete-zip";
    let temp = TempDirectory::new("skip-complete");
    let destination = destination(&temp.0);
    fs::write(&destination, body).unwrap();
    let server = ScriptedServer::new(vec![]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), Arc::clone(&credentials), 1);
    let result = worker.execute(
        work(&temp.0, item(body, md5_hex(body)), "skip-complete"),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Skipped);
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert!(!part_path(&destination).exists());
    assert_eq!(
        credentials.calls.load(Ordering::SeqCst),
        0,
        "a checksum-verified local final must skip before credential resolution"
    );
    assert!(server.finish().is_empty());
}

#[test]
fn resume_prefix_hashing_observes_pause_and_cancel_before_append() {
    let body = b"alos2-complete-zip";
    let split = 6;
    let temp = TempDirectory::new("resume-prefix-controls");
    let destination = destination(&temp.0);
    fs::write(part_path(&destination), &body[..split]).unwrap();
    let server = ScriptedServer::new(vec![response(
        "206 Partial Content",
        &[
            ("Content-Length", (body.len() - split).to_string()),
            (
                "Content-Range",
                format!("bytes {split}-{}/{}", body.len() - 1, body.len()),
            ),
        ],
        &body[split..],
    )]);
    let asset = resolved_asset(&server.url, body);
    let request = transfer_request(&server.url, &destination, body, "resume-prefix-controls");
    let paused = Arc::new(AtomicBool::new(true));
    let cancelled = Arc::new(AtomicBool::new(false));
    let pause_checks = Arc::new(AtomicUsize::new(0));
    let worker_paused = Arc::clone(&paused);
    let worker_cancelled = Arc::clone(&cancelled);
    let worker_checks = Arc::clone(&pause_checks);
    let transfer = transfer(1);
    let handle = thread::spawn(move || {
        let mut session = transfer.resolved_session();
        transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut session,
            &|| worker_cancelled.load(Ordering::Acquire),
            &|| {
                worker_checks.fetch_add(1, Ordering::AcqRel);
                worker_paused.load(Ordering::Acquire)
            },
            &mut |_| {},
        )
    });

    wait_until("resume prefix hashing did not observe pause", || {
        pause_checks.load(Ordering::Acquire) > 0
    });
    assert_eq!(
        fs::metadata(part_path(&destination)).unwrap().len(),
        split as u64
    );
    assert!(!destination.exists());
    cancelled.store(true, Ordering::Release);
    let result = handle.join().unwrap();
    assert!(matches!(
        result,
        ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Interrupted
    ));
    assert_eq!(
        fs::metadata(part_path(&destination)).unwrap().len(),
        split as u64
    );
    assert!(!destination.exists());
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn verified_complete_part_finalizes_without_request() {
    let body = b"alos2-complete-zip";
    let temp = TempDirectory::new("finalize-part");
    let destination = destination(&temp.0);
    fs::write(part_path(&destination), body).unwrap();
    let server = ScriptedServer::new(vec![]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), Arc::clone(&credentials), 1);
    let result = worker.execute(
        work(&temp.0, item(body, md5_hex(body)), "finalize-part"),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert!(!part_path(&destination).exists());
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);
    assert!(server.finish().is_empty());
}

#[test]
fn recovered_complete_part_with_wrong_md5_never_promotes() {
    let body = b"alos2-complete-zip";
    let wrong_body = vec![b'x'; body.len()];
    let temp = TempDirectory::new("wrong-complete-part");
    let destination = destination(&temp.0);
    fs::write(part_path(&destination), &wrong_body).unwrap();
    let server = ScriptedServer::new(vec![]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), credentials, 1);
    let result = worker.execute(
        work(&temp.0, item(body, md5_hex(body)), "wrong-complete-part"),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert!(!destination.exists());
    assert_eq!(fs::read(part_path(&destination)).unwrap(), wrong_body);
    assert!(server.finish().is_empty());
}

#[test]
fn recovered_checksum_part_resumes_without_if_range() {
    let body = b"alos2-complete-zip";
    let split = 7;
    let temp = TempDirectory::new("resume");
    let destination = destination(&temp.0);
    fs::write(part_path(&destination), &body[..split]).unwrap();
    let server = ScriptedServer::new(vec![response(
        "206 Partial Content",
        &[
            ("Content-Length", (body.len() - split).to_string()),
            (
                "Content-Range",
                format!("bytes {split}-{}/{}", body.len() - 1, body.len()),
            ),
        ],
        &body[split..],
    )]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), Arc::clone(&credentials), 1);
    let result = worker.execute(
        work(&temp.0, item(body, md5_hex(body)), "resume"),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(&destination).unwrap(), body);
    let request = server.finish()[0].to_ascii_lowercase();
    assert!(request.contains(&format!("range: bytes={split}-")));
    assert!(!request.contains("if-range:"));
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
fn ignored_range_200_sets_the_partial_aside_and_downloads_again() {
    let body = b"alos2-complete-zip";
    let temp = TempDirectory::new("restart-200");
    let destination = destination(&temp.0);
    fs::write(part_path(&destination), b"stale-prefix").unwrap();
    let server = ScriptedServer::new(vec![complete_response(body), complete_response(body)]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), credentials, 2);
    let result = worker.execute(
        work(&temp.0, item(body, md5_hex(body)), "restart-200"),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert!(!part_path(&destination).exists());
    assert_eq!(
        set_aside_partials(&destination),
        vec![b"stale-prefix".to_vec()]
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].to_ascii_lowercase().contains("range: bytes="));
    assert!(!requests[1].to_ascii_lowercase().contains("range:"));
}

#[test]
fn checksum_failure_never_promotes_the_alos2_archive() {
    let body = b"alos2-complete-zip";
    let temp = TempDirectory::new("bad-md5");
    let server = ScriptedServer::new(vec![complete_response(body)]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), credentials, 1);
    let result = worker.execute(
        work(
            &temp.0,
            item(body, "00000000000000000000000000000000".to_owned()),
            "bad-md5",
        ),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert!(!result.destination.exists());
    server.finish();
}

#[test]
fn size_failure_never_promotes_the_alos2_archive() {
    let body = b"alos2-complete-zip";
    let expected_size = body.len() as u64 + 1;
    let temp = TempDirectory::new("bad-size");
    let server = ScriptedServer::new(vec![complete_response(body)]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), credentials, 1);
    let mut frozen = item(body, md5_hex(body));
    frozen.asset_access.as_mut().unwrap().expected_size_bytes = Some(expected_size);
    frozen.metadata["properties"]["asset_integrity"][FILE_NAME]["size_bytes"] =
        json!(expected_size);
    frozen.metadata["assets"][0]["size_bytes"] = json!(expected_size);
    let result = worker.execute(
        work(&temp.0, frozen, "bad-size"),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert!(!result.destination.exists());
    server.finish();
}

#[test]
fn changed_etag_fails_closed_before_append_or_promotion() {
    let body = b"alos2-complete-zip";
    let split = 7;
    let temp = TempDirectory::new("changed-etag");
    let destination = destination(&temp.0);
    let server = ScriptedServer::new(vec![
        response(
            "200 OK",
            &[
                ("Content-Length", body.len().to_string()),
                ("ETag", "\"first\"".to_owned()),
            ],
            &body[..split],
        ),
        response(
            "206 Partial Content",
            &[
                ("Content-Length", (body.len() - split).to_string()),
                (
                    "Content-Range",
                    format!("bytes {split}-{}/{}", body.len() - 1, body.len()),
                ),
                ("ETag", "\"changed\"".to_owned()),
            ],
            &body[split..],
        ),
    ]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), credentials, 2);
    let result = worker.execute(
        work(&temp.0, item(body, md5_hex(body)), "changed-etag"),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert!(!destination.exists());
    assert_eq!(fs::read(part_path(&destination)).unwrap(), &body[..split]);
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[1]
        .to_ascii_lowercase()
        .contains(&format!("range: bytes={split}-")));
}

#[test]
fn changed_etag_on_200_sets_the_prefix_aside_instead_of_truncating() {
    let body = b"alos2-complete-zip";
    let split = 7;
    let temp = TempDirectory::new("changed-etag-200");
    let destination = destination(&temp.0);
    let server = ScriptedServer::new(vec![
        response(
            "200 OK",
            &[
                ("Content-Length", body.len().to_string()),
                ("ETag", "\"first\"".to_owned()),
            ],
            &body[..split],
        ),
        response(
            "200 OK",
            &[
                ("Content-Length", body.len().to_string()),
                ("ETag", "\"changed\"".to_owned()),
            ],
            body,
        ),
        complete_response(body),
    ]);
    let credentials = Arc::new(RecordingCredentials::default());
    let worker = worker(server.url.clone(), credentials, 2);
    let result = worker.execute(
        work(&temp.0, item(body, md5_hex(body)), "changed-etag-200"),
        WorkerControl::default(),
        &mut |_| {},
    );

    // The changed object is a refused resume: the prefix of the first object is
    // kept under a conflict name, never truncated, and the object downloads again.
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert!(!part_path(&destination).exists());
    assert_eq!(
        set_aside_partials(&destination),
        vec![body[..split].to_vec()]
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    let second = requests[1].to_ascii_lowercase();
    assert!(second.contains(&format!("range: bytes={split}-")));
    assert!(second.contains("if-range: \"first\""));
    assert!(!requests[2].to_ascii_lowercase().contains("range:"));
}
