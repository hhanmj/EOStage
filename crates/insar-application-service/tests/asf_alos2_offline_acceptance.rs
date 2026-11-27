use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationCatalogSearchAndFreezeRequest, ApplicationService,
    ApplicationServiceConfig,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, AssetAccessDescriptor, AssetAccessScheme,
    CatalogSearchQuery, CatalogSearchResponse, ContractError, DownloadLayout, DownloadOutcome,
    ProviderResult, ProxyPolicy, SearchWorkbenchState, StableAssetReference,
    StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    StoreUpdateWorkbenchRequest, TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_download::{
    part_path, AssetResolver, AssetResolverRegistry, Clock, CredentialMaterial, CredentialProvider,
    CredentialProviderError, DownloadWorker, HttpTransfer, ProductionDownloadWorker,
    ResolveContext, ResolveError, ResolvedAsset, ResolvedMethod, ResolverExecutionRuntime,
    ShadowAccessIntent, ShadowResolveContext, TransferOptions,
};
use insar_provider::asf::catalog_client::{AsfCatalogClient, CatalogSearchLimits};
use insar_provider::asf::client::{AsfHttpClient, HttpLimits, RemoteEndpoints};
use insar_provider::catalog::{
    CollectionProfileRegistry, ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID,
    ALOS2_PROVIDER_ID,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use serde_json::{json, Map, Value};

mod common;

const BODY: &[u8] = b"alos2-complete-zip";
const BODY_MD5: &str = "7e262ffcde136432b1a341942d1c8b04";
const FILE_NAME: &str = "ALOS2478123150-230331-WBDR1.1__D.zip";
const PRODUCT_ID: &str = "G3463311647-ASF";
const TRANSFER_CHUNK_SIZE: usize = 3;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempRoot {
    root: PathBuf,
    database: PathBuf,
}

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-alos2-offline-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("tasks.sqlite3");
        Self { root, database }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct ScriptedServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ScriptedServer {
    fn start(path: &str, responses: Vec<Vec<u8>>) -> Self {
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
            url: format!("http://{address}{path}"),
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
    fn start(body: &[u8], split: usize) -> Self {
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
            let _ = stream.write_all(&body[split..]);
            let _ = stream.flush();
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
            .recv_timeout(Duration::from_secs(4))
            .expect("the transfer server sent its first chunk");
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

fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            return request;
        }
        request.push_str(&line);
    }
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

fn catalog_response() -> Vec<u8> {
    let body = serde_json::to_vec(&json!({
        "hits": 1,
        "items": [granule()]
    }))
    .unwrap();
    response(
        "200 OK",
        &[
            ("Content-Length", body.len().to_string()),
            ("Content-Type", "application/json".to_owned()),
        ],
        &body,
    )
}

fn partial_response(split: usize) -> Vec<u8> {
    response(
        "206 Partial Content",
        &[
            ("Content-Length", (BODY.len() - split).to_string()),
            (
                "Content-Range",
                format!("bytes {split}-{}/{}", BODY.len() - 1, BODY.len()),
            ),
        ],
        &BODY[split..],
    )
}

fn granule() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-alos2-p1/granule-wbd-cmr-umm.json");
    let mut value: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("ALOS-2 CMR fixture exists"))
            .expect("ALOS-2 CMR fixture is valid JSON");
    value["umm"]["DataGranule"]["ArchiveAndDistributionInformation"][0]["SizeInBytes"] =
        json!(BODY.len());
    value["umm"]["DataGranule"]["ArchiveAndDistributionInformation"][0]["Checksum"]["Value"] =
        json!(BODY_MD5);
    value
}

#[derive(Default)]
struct FixtureAsf;

impl LiveAsfSearch for FixtureAsf {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
    }
}

struct AsfCatalogPort {
    client: AsfCatalogClient,
}

impl LiveCatalogSearch for AsfCatalogPort {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let profile = CollectionProfileRegistry
            .get(ALOS2_PROFILE_ID)
            .expect("the ALOS-2 profile is registered");
        self.client.search_alos2(query, profile, cancelled)
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

#[derive(Default)]
struct PlaceholderCredentials {
    calls: AtomicUsize,
}

impl CredentialProvider for PlaceholderCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "earthdata-default");
        self.calls.fetch_add(1, Ordering::SeqCst);
        CredentialMaterial::bearer("ALOS2_OFFLINE_PLACEHOLDER")
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
        _frozen: &insar_core::FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::failed())
    }
}

fn catalog_port(cmr_url: &str) -> AsfCatalogPort {
    let endpoints = RemoteEndpoints::loopback_for_tests(cmr_url, cmr_url).unwrap();
    let http = AsfHttpClient::with_test_endpoints(endpoints, HttpLimits::default()).unwrap();
    let client =
        AsfCatalogClient::with_http_client_for_tests(http, CatalogSearchLimits::default()).unwrap();
    AsfCatalogPort { client }
}

fn production_worker(
    transfer_url: &str,
    max_attempts: u32,
    credentials: Arc<PlaceholderCredentials>,
) -> ProductionDownloadWorker {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(LoopbackEarthdataResolver {
            url: transfer_url.to_owned(),
        }))
        .unwrap();
    let runtime =
        ResolverExecutionRuntime::with_dependencies(registry, credentials, Arc::new(FixedClock));
    let options = TransferOptions {
        max_attempts,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(3),
        chunk_size: TRANSFER_CHUNK_SIZE,
        ..TransferOptions::default()
    };
    let transfer = HttpTransfer::new_with_proxy_policy(options, ProxyPolicy::Direct).unwrap();
    ProductionDownloadWorker::new(options)
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, Arc::new(transfer))
}

fn build_service(
    database: &Path,
    cmr_url: &str,
    transfer_url: &str,
    max_attempts: u32,
    credentials: Arc<PlaceholderCredentials>,
) -> ApplicationService {
    let worker: Arc<dyn DownloadWorker> =
        Arc::new(production_worker(transfer_url, max_attempts, credentials));
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(database),
        Arc::new(FixtureAsf),
        Arc::new(catalog_port(cmr_url)),
        worker,
    );
    bind_aoi(&service, "fixture");
    service
}

fn bind_aoi(service: &ApplicationService, label: &str) {
    let region_id = common::ensure_fixture_region(service, label);
    let request = ApplicationBindRegionAoiRequest {
        request_id: format!("{label}-bind-aoi"),
        region_id,
        expected_revision: None,
        source_kind: "manual_bbox".to_owned(),
        geometry: aoi(),
        bbox: vec![-114.0, 21.0, -109.0, 26.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "ALOS-2 offline fixture AOI".to_owned(),
        bound_at: "2026-07-29T00:00:00Z".to_owned(),
    };
    service.bind_region_aoi(&request).unwrap();
}

fn aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[-114.0, 21.0], [-109.0, 21.0], [-109.0, 26.0], [-114.0, 26.0], [-114.0, 21.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn catalog_query() -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(aoi()),
        cursor: None,
        result_limit: Some(25),
        filters: json!({
            "datetime": {"start":"2023-03-31T00:00:00Z", "end":"2023-04-01T00:00:00Z"},
            "beam_mode": "WBD",
            "polarizations": ["HH", "HV"],
            "flight_direction": "DESCENDING",
            "look_direction": "RIGHT",
            "path": 161,
            "frame": 3150
        })
        .as_object()
        .unwrap()
        .clone(),
    }
}

fn freeze_and_select(service: &ApplicationService, label: &str) -> (String, String) {
    let batch_id = format!("alos2-{label}-batch");
    let frozen = service
        .catalog_search_and_freeze_legacy(&ApplicationCatalogSearchAndFreezeRequest {
            request_id: format!("alos2-{label}-search"),
            search: catalog_query(),
            batch_id: batch_id.clone(),
            created_at: "2026-07-29T00:00:00Z".to_owned(),
            aoi_name: "ALOS-2 offline fixture AOI".to_owned(),
            aoi_geojson: Some(aoi()),
            workbench_page_size: 24,
        })
        .expect("the complete CMR batch freezes atomically");
    let batch = frozen
        .restore
        .workbench_batch
        .expect("one frozen ALOS-2 batch");
    assert_eq!(batch.batch_id, batch_id);
    assert_eq!(batch.candidates.len(), 1);
    let item = &batch.candidates[0];
    assert_eq!(item.provider_id, ALOS2_PROVIDER_ID);
    assert_eq!(item.metadata["logical_dataset"], ALOS2_LOGICAL_DATASET);
    assert_eq!(
        item.asset_access
            .as_ref()
            .and_then(|access| access.expected_size_bytes),
        Some(BODY.len() as u64)
    );
    assert_eq!(
        item.asset_access
            .as_ref()
            .and_then(|access| access.checksum_value.as_deref()),
        Some(BODY_MD5)
    );
    let item_id = item.item_id.clone();
    service
        .update_search_workbench(&StoreUpdateWorkbenchRequest {
            state: SearchWorkbenchState {
                batch_id: batch_id.clone(),
                checked_item_ids: vec![item_id.clone()],
                page_index: 0,
                page_size: 24,
                revision: 1,
            },
            expected_revision: 0,
            updated_at: "2026-07-29T00:00:01Z".to_owned(),
        })
        .unwrap();
    (batch_id, item_id)
}

fn create_task(
    service: &ApplicationService,
    task_id: &str,
    batch_id: &str,
    item_id: &str,
    output_root: &Path,
) -> insar_core::FrozenTaskSnapshot {
    let snapshot = service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id: batch_id.to_owned(),
            selected_item_ids: vec![item_id.to_owned()],
            checked_item_ids: vec![item_id.to_owned()],
            page_index: 0,
            page_size: 24,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::SarDatasetProcessing,
            concurrency: 1,
            created_at: "2026-07-29T00:00:02Z".to_owned(),
        })
        .unwrap();
    snapshot
        .restore
        .tasks
        .into_iter()
        .find(|task| task.task_id == task_id)
        .expect("the immutable ALOS-2 task is visible")
}

fn destination(output_root: &Path) -> PathBuf {
    output_root
        .join(ALOS2_LOGICAL_DATASET.to_ascii_uppercase())
        .join("1_1")
        .join(FILE_NAME)
}

fn runtime(service: &ApplicationService, task_id: &str) -> Option<insar_core::TaskRuntimeSnapshot> {
    service
        .snapshot(true)
        .ok()?
        .restore
        .runtimes
        .into_iter()
        .find(|runtime| runtime.task_id == task_id)
}

fn wait_until(label: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {label}");
}

fn update_task(
    service: &ApplicationService,
    task_id: &str,
    kind: TaskStateCommandKind,
    item_ids: Vec<String>,
) {
    for attempt in 0..40 {
        let expected_revision = runtime(service, task_id).unwrap().revision;
        let request = StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision,
            command: StoreTaskCommand {
                kind: kind.clone(),
                item_ids: item_ids.clone(),
            },
            event_id: format!("alos2-offline-{task_id}-{attempt}-{expected_revision}"),
            occurred_at: "2026-07-29T00:00:03Z".to_owned(),
        };
        match service.update_download_task(&request) {
            Ok(_) => return,
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(3));
            }
            Err(error) => panic!("task update failed: {error:?}"),
        }
    }
    panic!("task update remained revision-conflicted");
}

fn assert_catalog_request(request: &str) {
    assert!(request.starts_with("GET /search/granules.umm_json?"));
    assert!(request.contains("collection_concept_id=C3315903479-ASF"));
    assert!(request.contains("page_num=1"));
    assert!(request.contains("polygon="));
}

#[test]
fn loopback_cmr_resume_md5_archive_and_restart_restore_the_frozen_task_and_output() {
    let temp = TempRoot::new("resume-history");
    let split = 7;
    let catalog = ScriptedServer::start("/search/granules.umm_json", vec![catalog_response()]);
    let transfer = ScriptedServer::start(&format!("/{FILE_NAME}"), vec![partial_response(split)]);
    let catalog_url = catalog.url.clone();
    let transfer_url = transfer.url.clone();
    let credentials = Arc::new(PlaceholderCredentials::default());
    let service = build_service(
        &temp.database,
        &catalog_url,
        &transfer_url,
        1,
        Arc::clone(&credentials),
    );
    let (batch_id, item_id) = freeze_and_select(&service, "resume-history");
    let output_root = temp.root.join("output");
    let final_output = destination(&output_root);
    fs::create_dir_all(final_output.parent().unwrap()).unwrap();
    fs::write(part_path(&final_output), &BODY[..split]).unwrap();

    let frozen_task = create_task(
        &service,
        "alos2-resume-history-task",
        &batch_id,
        &item_id,
        &output_root,
    );
    assert_eq!(frozen_task.provider_id, ALOS2_PROVIDER_ID);
    assert_eq!(frozen_task.layout, DownloadLayout::SarDatasetProcessing);
    assert_eq!(frozen_task.initial_selected_item_ids, vec![item_id.clone()]);
    assert_eq!(frozen_task.candidates.len(), 1);
    assert_eq!(
        frozen_task.candidates[0]
            .asset_access
            .as_ref()
            .unwrap()
            .reference
            .provider_product_id,
        PRODUCT_ID
    );

    wait_until("resumed ALOS-2 transfer completion", || {
        runtime(&service, "alos2-resume-history-task")
            .is_some_and(|runtime| runtime.status == TaskLifecycleStatus::Completed)
    });
    let result = service
        .download_snapshot("alos2-resume-history-task")
        .unwrap()
        .results
        .into_iter()
        .next()
        .expect("one ALOS-2 transfer result");
    assert_eq!(
        result.outcome,
        DownloadOutcome::Completed,
        "{:?}",
        result.failure
    );
    assert_eq!(result.destination, final_output);
    assert_eq!(result.bytes_written, BODY.len() as u64);
    assert_eq!(fs::read(&final_output).unwrap(), BODY);
    assert!(!part_path(&final_output).exists());
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);

    let terminal = runtime(&service, "alos2-resume-history-task").unwrap();
    service
        .archive_download_task(&StoreArchiveTaskRequest {
            task_id: "alos2-resume-history-task".to_owned(),
            expected_revision: terminal.revision,
            archived_at: "2026-07-29T00:00:04Z".to_owned(),
        })
        .unwrap();
    assert!(service.snapshot(false).unwrap().restore.tasks.is_empty());
    service.shutdown().unwrap();

    let catalog_requests = catalog.finish();
    assert_eq!(catalog_requests.len(), 1);
    assert_catalog_request(&catalog_requests[0]);
    let transfer_requests = transfer.finish();
    assert_eq!(transfer_requests.len(), 1);
    let request = transfer_requests[0].to_ascii_lowercase();
    assert!(request.contains(&format!("range: bytes={split}-")));
    assert!(!request.contains("if-range:"));
    assert!(request.contains("authorization: bearer alos2_offline_placeholder"));

    let restarted = build_service(
        &temp.database,
        &catalog_url,
        &transfer_url,
        1,
        Arc::new(PlaceholderCredentials::default()),
    );
    let restored = restarted.snapshot(true).unwrap().restore;
    let restored_task = restored
        .tasks
        .iter()
        .find(|task| task.task_id == "alos2-resume-history-task")
        .expect("archived task is present in include-terminal restore");
    assert_eq!(restored_task, &frozen_task);
    let restored_runtime = restored
        .runtimes
        .iter()
        .find(|runtime| runtime.task_id == "alos2-resume-history-task")
        .unwrap();
    assert_eq!(restored_runtime.status, TaskLifecycleStatus::Completed);
    let restored_result = restarted
        .download_snapshot("alos2-resume-history-task")
        .unwrap()
        .results
        .into_iter()
        .next()
        .expect("the completed transfer result survives restart");
    assert_eq!(restored_result, result);
    assert_eq!(fs::read(final_output).unwrap(), BODY);
    assert!(restarted.snapshot(false).unwrap().restore.tasks.is_empty());
    restarted.shutdown().unwrap();
}

#[test]
fn application_cancel_preserves_part_and_a_new_task_resumes_after_restart() {
    let temp = TempRoot::new("cancel-restart");
    let split = TRANSFER_CHUNK_SIZE;
    let catalog = ScriptedServer::start("/search/granules.umm_json", vec![catalog_response()]);
    let first_transfer = GatedServer::start(BODY, split);
    let catalog_url = catalog.url.clone();
    let service = build_service(
        &temp.database,
        &catalog_url,
        &first_transfer.url,
        1,
        Arc::new(PlaceholderCredentials::default()),
    );
    let (batch_id, item_id) = freeze_and_select(&service, "cancel-restart");
    let output_root = temp.root.join("output");
    let final_output = destination(&output_root);
    create_task(
        &service,
        "alos2-cancelled-task",
        &batch_id,
        &item_id,
        &output_root,
    );

    first_transfer.wait_for_first_chunk();
    wait_until("the first ALOS-2 part chunk", || {
        fs::metadata(part_path(&final_output)).is_ok_and(|metadata| metadata.len() == split as u64)
    });
    update_task(
        &service,
        "alos2-cancelled-task",
        TaskStateCommandKind::CancelItems,
        vec![item_id.clone()],
    );
    // Allow the wake-driven coordinator to project Store cancellation into the
    // live WorkerControl before the blocked response body becomes readable.
    thread::sleep(Duration::from_millis(150));
    first_transfer.release();
    wait_until("cancelled ALOS-2 worker result", || {
        service
            .download_snapshot("alos2-cancelled-task")
            .is_ok_and(|snapshot| {
                snapshot
                    .results
                    .first()
                    .is_some_and(|result| result.outcome == DownloadOutcome::Cancelled)
            })
    });
    let cancelled = service
        .download_snapshot("alos2-cancelled-task")
        .unwrap()
        .results
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cancelled.outcome, DownloadOutcome::Cancelled);
    assert_eq!(
        runtime(&service, "alos2-cancelled-task").unwrap().status,
        TaskLifecycleStatus::Cancelled
    );
    assert!(!final_output.exists());
    let preserved_part = fs::read(part_path(&final_output)).unwrap();
    assert!(!preserved_part.is_empty());
    assert!(preserved_part.len() < BODY.len());
    assert_eq!(preserved_part, BODY[..preserved_part.len()]);
    let resume_offset = preserved_part.len();
    service.shutdown().unwrap();
    assert_eq!(first_transfer.finish().len(), 1);
    let catalog_requests = catalog.finish();
    assert_eq!(catalog_requests.len(), 1);
    assert_catalog_request(&catalog_requests[0]);

    let second_transfer = ScriptedServer::start(
        &format!("/{FILE_NAME}"),
        vec![partial_response(resume_offset)],
    );
    let restarted = build_service(
        &temp.database,
        &catalog_url,
        &second_transfer.url,
        1,
        Arc::new(PlaceholderCredentials::default()),
    );
    let resumed_task = create_task(
        &restarted,
        "alos2-resumed-task",
        &batch_id,
        &item_id,
        &output_root,
    );
    assert_eq!(resumed_task.initial_selected_item_ids, vec![item_id]);
    wait_until("new ALOS-2 task resumed after restart", || {
        runtime(&restarted, "alos2-resumed-task")
            .is_some_and(|runtime| runtime.status == TaskLifecycleStatus::Completed)
    });
    assert_eq!(fs::read(&final_output).unwrap(), BODY);
    assert!(!part_path(&final_output).exists());
    let resumed = restarted
        .download_snapshot("alos2-resumed-task")
        .unwrap()
        .results
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(resumed.outcome, DownloadOutcome::Completed);
    restarted.shutdown().unwrap();

    let requests = second_transfer.finish();
    assert_eq!(requests.len(), 1);
    let request = requests[0].to_ascii_lowercase();
    assert!(request.contains(&format!("range: bytes={resume_offset}-")));
    assert!(!request.contains("if-range:"));
}

#[test]
fn changed_validator_fails_closed_preserves_part_and_restores_failed_state() {
    let temp = TempRoot::new("changed-validator");
    let split = 7;
    let catalog = ScriptedServer::start("/search/granules.umm_json", vec![catalog_response()]);
    let transfer = ScriptedServer::start(
        &format!("/{FILE_NAME}"),
        vec![
            response(
                "200 OK",
                &[
                    ("Content-Length", BODY.len().to_string()),
                    ("ETag", "\"first\"".to_owned()),
                ],
                &BODY[..split],
            ),
            response(
                "206 Partial Content",
                &[
                    ("Content-Length", (BODY.len() - split).to_string()),
                    (
                        "Content-Range",
                        format!("bytes {split}-{}/{}", BODY.len() - 1, BODY.len()),
                    ),
                    ("ETag", "\"changed\"".to_owned()),
                ],
                &BODY[split..],
            ),
        ],
    );
    let catalog_url = catalog.url.clone();
    let transfer_url = transfer.url.clone();
    let service = build_service(
        &temp.database,
        &catalog_url,
        &transfer_url,
        2,
        Arc::new(PlaceholderCredentials::default()),
    );
    let (batch_id, item_id) = freeze_and_select(&service, "changed-validator");
    let output_root = temp.root.join("output");
    let final_output = destination(&output_root);
    let frozen_task = create_task(
        &service,
        "alos2-changed-validator-task",
        &batch_id,
        &item_id,
        &output_root,
    );

    wait_until("changed-validator ALOS-2 failure", || {
        runtime(&service, "alos2-changed-validator-task")
            .is_some_and(|runtime| runtime.status == TaskLifecycleStatus::Failed)
    });
    let failed = service
        .download_snapshot("alos2-changed-validator-task")
        .unwrap()
        .results
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(failed.outcome, DownloadOutcome::Failed);
    assert_eq!(failed.attempt_count, 2);
    assert_eq!(failed.failure.as_ref().unwrap().code, "DL002");
    assert!(!final_output.exists());
    assert_eq!(fs::read(part_path(&final_output)).unwrap(), &BODY[..split]);
    service.shutdown().unwrap();

    let catalog_requests = catalog.finish();
    assert_eq!(catalog_requests.len(), 1);
    assert_catalog_request(&catalog_requests[0]);
    let requests = transfer.finish();
    assert_eq!(requests.len(), 2);
    let retry = requests[1].to_ascii_lowercase();
    assert!(retry.contains(&format!("range: bytes={split}-")));
    assert!(retry.contains("if-range: \"first\""));

    let restarted = build_service(
        &temp.database,
        &catalog_url,
        &transfer_url,
        2,
        Arc::new(PlaceholderCredentials::default()),
    );
    let restored = restarted.snapshot(true).unwrap().restore;
    assert_eq!(
        restored
            .tasks
            .iter()
            .find(|task| task.task_id == "alos2-changed-validator-task"),
        Some(&frozen_task)
    );
    let failed_runtime = restored
        .runtimes
        .iter()
        .find(|runtime| runtime.task_id == "alos2-changed-validator-task")
        .unwrap();
    assert_eq!(failed_runtime.status, TaskLifecycleStatus::Failed);
    assert_eq!(failed_runtime.failed_item_ids, vec![item_id]);
    let restored_failure = restarted
        .download_snapshot("alos2-changed-validator-task")
        .unwrap()
        .results
        .into_iter()
        .next()
        .expect("the failed transfer result survives restart");
    assert_eq!(restored_failure, failed);
    assert!(!final_output.exists());
    assert_eq!(fs::read(part_path(&final_output)).unwrap(), &BODY[..split]);
    restarted.shutdown().unwrap();
}
