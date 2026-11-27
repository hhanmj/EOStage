use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
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
    CollectionProfileRegistry, NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID,
    NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION, NISAR_PROFILE_ID, NISAR_PROVIDER_ID,
    NISAR_RESOLVER_POLICY_ID_V2,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

mod common;
use sha2::{Digest, Sha256};

#[path = "../../insar-download/tests/support/nisar_hdf5.rs"]
#[allow(dead_code)]
mod nisar_hdf5_support;

use nisar_hdf5_support::{
    create_valid_fixture_for_profile, FixtureDirectory, FrequencyScope,
    LEGACY_PRODUCT_SPECIFICATION_VERSION, P05023_PRODUCT_SPECIFICATION_VERSION,
};

const GRANULE_ID: &str =
    "NISAR_L1_PR_RSLC_026_045_D_056_0500_SHNA_M_20260723T074816_20260723T074822_P05023_N_P_J_001";
const FILE_NAME: &str = "NISAR_L1_PR_RSLC_026_045_D_056_0500_SHNA_M_20260723T074816_20260723T074822_P05023_N_P_J_001.h5";
const PRODUCT_ID: &str = "G4260317833-ASF";
const SECOND_GRANULE_ID: &str =
    "NISAR_L1_PR_RSLC_026_045_D_056_0500_SHNA_M_20260723T074816_20260723T074822_P05023_N_P_J_002";
const SECOND_PRODUCT_ID: &str = "G4260317834-ASF";
const TRANSFER_CHUNK_SIZE: usize = 4_096;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
static TEST_SERIAL: Mutex<()> = Mutex::new(());
static HDF5_ISOLATION: std::sync::Once = std::sync::Once::new();

fn serial_test() -> MutexGuard<'static, ()> {
    // The NISAR HDF5 validator requires app-owned plugin isolation. These
    // tests must establish it themselves so they pass without an externally
    // injected HDF5_PLUGIN_PRELOAD sentinel.
    HDF5_ISOLATION.call_once(|| {
        insar_download::establish_hdf5_plugin_isolation()
            .expect("HDF5 plugin isolation must be established by the test host");
    });
    TEST_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct ProductFixture {
    body: Vec<u8>,
    sha256: String,
}

impl ProductFixture {
    fn valid(label: &str) -> Self {
        let directory = FixtureDirectory::new(label);
        let path = directory.file("product");
        create_valid_fixture_for_profile(
            &path,
            FrequencyScope::AOnly,
            GRANULE_ID,
            P05023_PRODUCT_SPECIFICATION_VERSION,
        );
        Self::from_path(&path)
    }

    fn specification_wrong(label: &str) -> Self {
        let directory = FixtureDirectory::new(label);
        let path = directory.file("product");
        create_valid_fixture_for_profile(
            &path,
            FrequencyScope::AOnly,
            GRANULE_ID,
            LEGACY_PRODUCT_SPECIFICATION_VERSION,
        );
        Self::from_path(&path)
    }

    fn from_path(path: &Path) -> Self {
        let body = fs::read(path).expect("read synthetic NISAR HDF5");
        let sha256 = format!("{:x}", Sha256::digest(&body));
        Self { body, sha256 }
    }

    fn split(&self) -> usize {
        TRANSFER_CHUNK_SIZE.min(self.body.len() / 2).max(1)
    }
}

struct TempRoot {
    root: PathBuf,
    database: PathBuf,
}

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-nisar-offline-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create isolated NISAR acceptance root");
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
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback fixture server");
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept fixture request");
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
        Self::start_with_tail(body, split, false)
    }

    fn start_resumable(body: &[u8], split: usize) -> Self {
        Self::start_with_tail(body, split, true)
    }

    fn start_with_tail(body: &[u8], split: usize, send_remaining: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind gated loopback server");
        let address = listener.local_addr().unwrap();
        let body = body.to_vec();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let (first_chunk_sender, first_chunk) = mpsc::channel();
        let (release, release_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept gated transfer");
            let request = read_request(&mut stream);
            captured.lock().unwrap().push(request.clone());
            if send_remaining {
                let expected_range = format!("range: bytes={split}-");
                assert!(
                    request.to_ascii_lowercase().contains(&expected_range),
                    "resumable fixture requires the exact Range request"
                );
                let remaining = body.len() - split;
                write!(
                    stream,
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {remaining}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                    split,
                    body.len() - 1,
                    body.len()
                )
                .unwrap();
                let first_end = split + remaining.min(split);
                stream.write_all(&body[split..first_end]).unwrap();
                stream.flush().unwrap();
                first_chunk_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
                if first_end < body.len() {
                    let _ = stream.write_all(&body[first_end..]);
                    let _ = stream.flush();
                }
            } else {
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
                // Make one more byte readable so the cancellation fixture can
                // observe its live cancellation without completing in one read.
                if split < body.len() {
                    let _ = stream.write_all(&body[split..split + 1]);
                    let _ = stream.flush();
                }
            }
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

fn full_response(body: &[u8]) -> Vec<u8> {
    response(
        "200 OK",
        &[("Content-Length", body.len().to_string())],
        body,
    )
}

fn partial_response(body: &[u8], split: usize) -> Vec<u8> {
    response(
        "206 Partial Content",
        &[
            ("Content-Length", (body.len() - split).to_string()),
            (
                "Content-Range",
                format!("bytes {split}-{}/{}", body.len() - 1, body.len()),
            ),
        ],
        &body[split..],
    )
}

fn captured_item(fixture: &ProductFixture) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asf/nisar/provisional-rslc/granule-partial.json");
    let captured: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("captured NISAR CMR fixture exists"))
            .expect("captured NISAR CMR fixture is valid JSON");
    let mut item = captured["cmr_response"]["items"][0].clone();
    let primary = item["umm"]["DataGranule"]["ArchiveAndDistributionInformation"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|archive| archive["Name"] == FILE_NAME)
        .expect("captured fixture has the primary HDF5 archive");
    primary["SizeInBytes"] = json!(fixture.body.len());
    primary["Checksum"] = json!({
        "Value": fixture.sha256,
        "Algorithm": "SHA-256"
    });
    item
}

fn replace_string_values(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::String(text) => *text = text.replace(from, to),
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| replace_string_values(value, from, to)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| replace_string_values(value, from, to)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn second_captured_item(fixture: &ProductFixture) -> Value {
    let mut item = captured_item(fixture);
    replace_string_values(&mut item, GRANULE_ID, SECOND_GRANULE_ID);
    item["meta"]["concept-id"] = json!(SECOND_PRODUCT_ID);
    item
}

fn beta_item(fixture: &ProductFixture) -> Value {
    let mut item = captured_item(fixture);
    item["umm"]["CollectionReference"]["ShortName"] = json!("NISAR_L1_RSLC_BETA_V1");
    item
}

fn ambiguous_primary_item(fixture: &ProductFixture) -> Value {
    let mut item = captured_item(fixture);
    let second_name = format!("{GRANULE_ID}_SECOND.h5");
    item["umm"]["DataGranule"]["ArchiveAndDistributionInformation"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "Name": second_name,
            "SizeInBytes": fixture.body.len(),
            "Format": "HDF5",
            "Checksum": {"Value": fixture.sha256, "Algorithm": "SHA-256"}
        }));
    item["umm"]["RelatedUrls"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "URL": format!(
                "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/NISAR_L1_RSLC_PROVISIONAL_V1/{GRANULE_ID}/{second_name}"
            ),
            "Type": "GET DATA",
            "Description": format!("Download {second_name}"),
            "Format": "HDF5"
        }));
    item
}

fn without_browse_item(fixture: &ProductFixture) -> Value {
    let mut item = captured_item(fixture);
    item["umm"]["DataGranule"]["ArchiveAndDistributionInformation"]
        .as_array_mut()
        .unwrap()
        .retain(|archive| {
            !archive["Format"].as_str().is_some_and(|format| {
                matches!(format.to_ascii_uppercase().as_str(), "PNG" | "JPEG" | "JPG")
            })
        });
    item["umm"]["RelatedUrls"]
        .as_array_mut()
        .unwrap()
        .retain(|related| related["Type"] != "GET RELATED VISUALIZATION");
    item
}

fn catalog_response(item: Value) -> Vec<u8> {
    catalog_response_items(vec![item])
}

fn catalog_response_items(items: Vec<Value>) -> Vec<u8> {
    let body = serde_json::to_vec(&json!({"hits": items.len(), "items": items})).unwrap();
    response(
        "200 OK",
        &[
            ("Content-Length", body.len().to_string()),
            ("Content-Type", "application/json".to_owned()),
        ],
        &body,
    )
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

struct NisarCatalogPort {
    client: AsfCatalogClient,
}

impl LiveCatalogSearch for NisarCatalogPort {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let profile = CollectionProfileRegistry
            .get(NISAR_PROFILE_ID)
            .expect("the NISAR profile is registered");
        self.client.search_nisar(query, profile, cancelled)
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
        CredentialMaterial::bearer("NISAR_OFFLINE_PLACEHOLDER")
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

fn catalog_port(cmr_url: &str) -> NisarCatalogPort {
    let endpoints = RemoteEndpoints::loopback_for_tests(cmr_url, cmr_url).unwrap();
    let http = AsfHttpClient::with_test_endpoints(endpoints, HttpLimits::default()).unwrap();
    let client =
        AsfCatalogClient::with_http_client_for_tests(http, CatalogSearchLimits::default()).unwrap();
    NisarCatalogPort { client }
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
        timeout: Duration::from_secs(4),
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
        bbox: vec![-179.0, 63.0, -173.0, 66.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "NISAR offline fixture AOI".to_owned(),
        bound_at: "2026-07-29T00:00:00Z".to_owned(),
    };
    service.bind_region_aoi(&request).unwrap();
}

fn aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[-179.0, 63.0], [-173.0, 63.0], [-173.0, 66.0], [-179.0, 66.0], [-179.0, 63.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn catalog_query() -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: NISAR_LOGICAL_DATASET.to_owned(),
        provider_id: NISAR_PROVIDER_ID.to_owned(),
        native_collection_id: NISAR_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(aoi()),
        cursor: None,
        result_limit: Some(25),
        filters: json!({
            "datetime": {"start":"2026-07-23T07:48:00Z", "end":"2026-07-23T07:49:00Z"},
            "production_configuration": "PR",
            "frame_coverage": "PARTIAL",
            "frequency_a_polarization": "HH",
            "range_bandwidth": "5",
            "joint_observation": false,
            "flight_direction": "DESCENDING",
            "look_direction": "LEFT",
            "track": 45,
            "frame": 56
        })
        .as_object()
        .unwrap()
        .clone(),
    }
}

fn operation(label: &str) -> ApplicationCatalogSearchAndFreezeRequest {
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: format!("nisar-{label}-search"),
        search: catalog_query(),
        batch_id: format!("nisar-{label}-batch"),
        created_at: "2026-07-29T00:00:00Z".to_owned(),
        aoi_name: "NISAR offline fixture AOI".to_owned(),
        aoi_geojson: Some(aoi()),
        workbench_page_size: 24,
    }
}

fn freeze_and_select(
    service: &ApplicationService,
    label: &str,
    fixture: &ProductFixture,
) -> (String, String) {
    let operation = operation(label);
    let batch_id = operation.batch_id.clone();
    let frozen = service
        .catalog_search_and_freeze_legacy(&operation)
        .expect("the complete NISAR CMR batch freezes atomically");
    let batch = frozen
        .restore
        .workbench_batch
        .expect("one frozen NISAR batch");
    assert_eq!(batch.batch_id, batch_id);
    assert!(!batch.candidates.is_empty());
    let item = batch
        .candidates
        .iter()
        .find(|item| item.display_name == GRANULE_ID)
        .expect("the selected P05023 product is in the frozen batch");
    assert_eq!(item.provider_id, NISAR_PROVIDER_ID);
    assert_eq!(item.display_name, GRANULE_ID);
    assert_eq!(item.metadata["logical_dataset"], NISAR_LOGICAL_DATASET);
    assert_eq!(item.metadata["properties"]["quality_status"], "Provisional");
    assert_eq!(item.metadata["properties"]["crid"], "P05023");
    assert_eq!(
        item.metadata["properties"]["product_specification_version"],
        P05023_PRODUCT_SPECIFICATION_VERSION
    );
    assert_eq!(
        item.metadata["properties"]["product_specification_profile_registry_version"],
        NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION
    );
    assert_eq!(
        item.metadata["resolver_policy_id"],
        NISAR_RESOLVER_POLICY_ID_V2
    );
    assert_eq!(
        item.metadata["properties"]["frequency_a_polarization"],
        "HH"
    );
    assert!(item.metadata["properties"]
        .get("frequency_b_polarization")
        .is_none());
    let access = item
        .asset_access
        .as_ref()
        .expect("the unique complete HDF5 is executable");
    assert_eq!(access.expected_size_bytes, Some(fixture.body.len() as u64));
    assert_eq!(access.checksum_algorithm.as_deref(), Some("SHA256"));
    assert_eq!(
        access.checksum_value.as_deref(),
        Some(fixture.sha256.as_str())
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
        .expect("the immutable NISAR task is visible")
}

fn destination(output_root: &Path) -> PathBuf {
    output_root
        .join(NISAR_LOGICAL_DATASET.to_ascii_uppercase())
        .join("RSLC")
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
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {label}");
}

/// Waits for a task to complete within the same deadline as `wait_until`; a
/// timeout also reports the task's lifecycle status and download results, so
/// a failed transfer is not reported only as a missing completion.
fn wait_until_task_completed(service: &ApplicationService, task_id: &str, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if runtime(service, task_id)
            .is_some_and(|runtime| runtime.status == TaskLifecycleStatus::Completed)
        {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "timed out waiting for {label}: status={:?} results={:?}",
        runtime(service, task_id).map(|runtime| runtime.status),
        service
            .download_snapshot(task_id)
            .map(|snapshot| snapshot.results)
    );
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
            event_id: format!("nisar-offline-{task_id}-{attempt}-{expected_revision}"),
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
    assert!(request.contains("collection_concept_id=C2853145197-ASF"));
    assert!(request.contains("page_num=1"));
    assert!(request.contains("polygon="));
}

fn assert_empty_store(service: &ApplicationService) {
    let snapshot = service.snapshot(true).unwrap();
    assert!(snapshot.restore.workbench_batch.is_none());
    assert!(snapshot.restore.workbench.is_none());
    assert!(snapshot.restore.tasks.is_empty());
}

#[test]
fn loopback_cmr_multi_asset_range_sha256_hdf5_archive_and_restart_restore_history() {
    let _serial = serial_test();
    let fixture = ProductFixture::valid("resume-history");
    let temp = TempRoot::new("resume-history");
    let split = fixture.split();
    let catalog = ScriptedServer::start(
        "/search/granules.umm_json",
        vec![catalog_response_items(vec![
            captured_item(&fixture),
            second_captured_item(&fixture),
        ])],
    );
    let transfer = GatedServer::start_resumable(&fixture.body, split);
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
    let (batch_id, item_id) = freeze_and_select(&service, "resume-history", &fixture);
    assert_eq!(
        service
            .snapshot(true)
            .unwrap()
            .restore
            .workbench_batch
            .unwrap()
            .candidates
            .len(),
        2
    );
    let output_root = temp.root.join("output");
    let final_output = destination(&output_root);
    fs::create_dir_all(final_output.parent().unwrap()).unwrap();
    fs::write(part_path(&final_output), &fixture.body[..split]).unwrap();

    let frozen_task = create_task(
        &service,
        "nisar-resume-history-task",
        &batch_id,
        &item_id,
        &output_root,
    );
    transfer.wait_for_first_chunk();
    assert_eq!(frozen_task.provider_id, NISAR_PROVIDER_ID);
    assert_eq!(frozen_task.initial_selected_item_ids, vec![item_id.clone()]);
    assert_eq!(frozen_task.candidates.len(), 2);
    assert_eq!(
        frozen_task.candidates[0]
            .asset_access
            .as_ref()
            .unwrap()
            .reference
            .provider_product_id,
        PRODUCT_ID
    );
    {
        let compact_store = TaskStore::open(&temp.database).unwrap();
        let compact = compact_store
            .load_scheduler_task("nisar-resume-history-task")
            .unwrap()
            .expect("scheduler restores the selected-only compact task");
        assert_eq!(compact.snapshot.candidates.len(), 1);
        assert_eq!(compact.snapshot.candidates[0].item_id, item_id);
        assert_eq!(
            compact.snapshot.candidates[0].metadata["properties"]["crid"],
            "P05023"
        );
        assert_eq!(
            compact.snapshot.candidates[0].metadata["properties"]["product_specification_version"],
            P05023_PRODUCT_SPECIFICATION_VERSION
        );
        assert_eq!(
            compact.snapshot.candidates[0].metadata["properties"]
                ["product_specification_profile_registry_version"],
            NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION
        );
        assert_eq!(
            compact.snapshot.candidates[0]
                .execution_policy
                .as_ref()
                .unwrap()
                .policy_id,
            NISAR_RESOLVER_POLICY_ID_V2
        );
    }

    transfer.release();
    wait_until_task_completed(
        &service,
        "nisar-resume-history-task",
        "resumed NISAR transfer completion",
    );
    let result = service
        .download_snapshot("nisar-resume-history-task")
        .unwrap()
        .results
        .into_iter()
        .next()
        .expect("one NISAR transfer result");
    assert_eq!(
        result.outcome,
        DownloadOutcome::Completed,
        "{:?}",
        result.failure
    );
    assert_eq!(result.destination, final_output);
    assert_eq!(result.bytes_written, fixture.body.len() as u64);
    assert_eq!(fs::read(&final_output).unwrap(), fixture.body);
    assert!(!part_path(&final_output).exists());
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);

    let terminal = runtime(&service, "nisar-resume-history-task").unwrap();
    service
        .archive_download_task(&StoreArchiveTaskRequest {
            task_id: "nisar-resume-history-task".to_owned(),
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
    assert!(request.contains("authorization: bearer nisar_offline_placeholder"));

    let restarted = build_service(
        &temp.database,
        &catalog_url,
        &transfer_url,
        1,
        Arc::new(PlaceholderCredentials::default()),
    );
    let restored = restarted.snapshot(true).unwrap().restore;
    assert_eq!(
        restored
            .tasks
            .iter()
            .find(|task| task.task_id == "nisar-resume-history-task"),
        Some(&frozen_task)
    );
    assert_eq!(
        restored
            .runtimes
            .iter()
            .find(|runtime| runtime.task_id == "nisar-resume-history-task")
            .unwrap()
            .status,
        TaskLifecycleStatus::Completed
    );
    assert_eq!(
        restarted
            .download_snapshot("nisar-resume-history-task")
            .unwrap()
            .results[0],
        result
    );
    assert_eq!(fs::read(final_output).unwrap(), fixture.body);
    assert!(restarted.snapshot(false).unwrap().restore.tasks.is_empty());
    restarted.shutdown().unwrap();
}

#[test]
fn beta_and_ambiguous_primary_reject_without_store_writes_while_missing_browse_freezes() {
    let _serial = serial_test();
    let fixture = ProductFixture::valid("catalog-boundaries");
    for (label, item, expected_code) in [
        ("beta", beta_item(&fixture), "NISAR_QUALITY_STATUS_MISMATCH"),
        (
            "ambiguous-primary",
            ambiguous_primary_item(&fixture),
            "ASF_PRIMARY_ASSET_AMBIGUOUS",
        ),
    ] {
        let temp = TempRoot::new(label);
        let catalog =
            ScriptedServer::start("/search/granules.umm_json", vec![catalog_response(item)]);
        let service = build_service(
            &temp.database,
            &catalog.url,
            "http://127.0.0.1:9/unreachable.h5",
            1,
            Arc::new(PlaceholderCredentials::default()),
        );
        let error = service
            .catalog_search_and_freeze_legacy(&operation(label))
            .unwrap_err();
        assert_eq!(error.code(), expected_code, "{label}: {error:?}");
        assert_empty_store(&service);
        service.shutdown().unwrap();
        let requests = catalog.finish();
        assert_eq!(requests.len(), 1);
        assert_catalog_request(&requests[0]);
    }

    let temp = TempRoot::new("missing-browse");
    let catalog = ScriptedServer::start(
        "/search/granules.umm_json",
        vec![catalog_response(without_browse_item(&fixture))],
    );
    let service = build_service(
        &temp.database,
        &catalog.url,
        "http://127.0.0.1:9/unreachable.h5",
        1,
        Arc::new(PlaceholderCredentials::default()),
    );
    let frozen = service
        .catalog_search_and_freeze_legacy(&operation("missing-browse"))
        .expect("missing optional Browse does not invalidate the complete HDF5");
    let candidate = &frozen.restore.workbench_batch.unwrap().candidates[0];
    assert_eq!(candidate.metadata["properties"]["preview_available"], false);
    assert!(candidate.asset_access.is_some());
    assert!(candidate.metadata["assets"]
        .as_array()
        .unwrap()
        .iter()
        .all(|asset| !matches!(asset["role"].as_str(), Some("browse" | "thumbnail"))));
    service.shutdown().unwrap();
    let requests = catalog.finish();
    assert_eq!(requests.len(), 1);
    assert_catalog_request(&requests[0]);
}

#[test]
fn application_cancel_preserves_part_and_a_new_task_resumes_after_restart() {
    let _serial = serial_test();
    let fixture = ProductFixture::valid("cancel-restart");
    let temp = TempRoot::new("cancel-restart");
    let split = fixture.split();
    let catalog = ScriptedServer::start(
        "/search/granules.umm_json",
        vec![catalog_response(captured_item(&fixture))],
    );
    let first_transfer = GatedServer::start(&fixture.body, split);
    let catalog_url = catalog.url.clone();
    let service = build_service(
        &temp.database,
        &catalog_url,
        &first_transfer.url,
        1,
        Arc::new(PlaceholderCredentials::default()),
    );
    let (batch_id, item_id) = freeze_and_select(&service, "cancel-restart", &fixture);
    let output_root = temp.root.join("output");
    let final_output = destination(&output_root);
    create_task(
        &service,
        "nisar-cancelled-task",
        &batch_id,
        &item_id,
        &output_root,
    );

    first_transfer.wait_for_first_chunk();
    wait_until("the first NISAR part chunk", || {
        fs::metadata(part_path(&final_output)).is_ok_and(|metadata| metadata.len() == split as u64)
    });
    update_task(
        &service,
        "nisar-cancelled-task",
        TaskStateCommandKind::CancelItems,
        vec![item_id.clone()],
    );
    thread::sleep(Duration::from_millis(150));
    first_transfer.release();
    wait_until("cancelled NISAR worker result", || {
        service
            .download_snapshot("nisar-cancelled-task")
            .is_ok_and(|snapshot| {
                snapshot
                    .results
                    .first()
                    .is_some_and(|result| result.outcome == DownloadOutcome::Cancelled)
            })
    });
    assert_eq!(
        runtime(&service, "nisar-cancelled-task").unwrap().status,
        TaskLifecycleStatus::Cancelled
    );
    assert!(!final_output.exists());
    let preserved_part = fs::read(part_path(&final_output)).unwrap();
    assert!(!preserved_part.is_empty());
    assert!(preserved_part.len() < fixture.body.len());
    assert_eq!(preserved_part, fixture.body[..preserved_part.len()]);
    let resume_offset = preserved_part.len();
    service.shutdown().unwrap();
    assert_eq!(first_transfer.finish().len(), 1);
    let catalog_requests = catalog.finish();
    assert_eq!(catalog_requests.len(), 1);
    assert_catalog_request(&catalog_requests[0]);

    let second_transfer = ScriptedServer::start(
        &format!("/{FILE_NAME}"),
        vec![partial_response(&fixture.body, resume_offset)],
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
        "nisar-resumed-task",
        &batch_id,
        &item_id,
        &output_root,
    );
    assert_eq!(resumed_task.initial_selected_item_ids, vec![item_id]);
    wait_until_task_completed(
        &restarted,
        "nisar-resumed-task",
        "new NISAR task resumed after restart",
    );
    assert_eq!(fs::read(&final_output).unwrap(), fixture.body);
    assert!(!part_path(&final_output).exists());
    assert_eq!(
        restarted
            .download_snapshot("nisar-resumed-task")
            .unwrap()
            .results[0]
            .outcome,
        DownloadOutcome::Completed
    );
    restarted.shutdown().unwrap();

    let requests = second_transfer.finish();
    assert_eq!(requests.len(), 1);
    let request = requests[0].to_ascii_lowercase();
    assert!(request.contains(&format!("range: bytes={resume_offset}-")));
    assert!(!request.contains("if-range:"));
}

#[test]
fn changed_validator_fails_closed_preserves_part_and_restores_failed_state() {
    let _serial = serial_test();
    let fixture = ProductFixture::valid("changed-validator");
    let temp = TempRoot::new("changed-validator");
    let split = fixture.split();
    let catalog = ScriptedServer::start(
        "/search/granules.umm_json",
        vec![catalog_response(captured_item(&fixture))],
    );
    let transfer = ScriptedServer::start(
        &format!("/{FILE_NAME}"),
        vec![
            response(
                "200 OK",
                &[
                    ("Content-Length", fixture.body.len().to_string()),
                    ("ETag", "\"first\"".to_owned()),
                ],
                &fixture.body[..split],
            ),
            response(
                "206 Partial Content",
                &[
                    ("Content-Length", (fixture.body.len() - split).to_string()),
                    (
                        "Content-Range",
                        format!(
                            "bytes {split}-{}/{}",
                            fixture.body.len() - 1,
                            fixture.body.len()
                        ),
                    ),
                    ("ETag", "\"changed\"".to_owned()),
                ],
                &fixture.body[split..],
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
    let (batch_id, item_id) = freeze_and_select(&service, "changed-validator", &fixture);
    let output_root = temp.root.join("output");
    let final_output = destination(&output_root);
    let frozen_task = create_task(
        &service,
        "nisar-changed-validator-task",
        &batch_id,
        &item_id,
        &output_root,
    );

    wait_until("changed-validator NISAR failure", || {
        runtime(&service, "nisar-changed-validator-task")
            .is_some_and(|runtime| runtime.status == TaskLifecycleStatus::Failed)
    });
    let failed = service
        .download_snapshot("nisar-changed-validator-task")
        .unwrap()
        .results
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(failed.outcome, DownloadOutcome::Failed);
    assert_eq!(failed.attempt_count, 2);
    assert_eq!(failed.failure.as_ref().unwrap().code, "DL002");
    assert!(!final_output.exists());
    assert_eq!(
        fs::read(part_path(&final_output)).unwrap(),
        fixture.body[..split]
    );
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
            .find(|task| task.task_id == "nisar-changed-validator-task"),
        Some(&frozen_task)
    );
    assert_eq!(
        restored
            .runtimes
            .iter()
            .find(|runtime| runtime.task_id == "nisar-changed-validator-task")
            .unwrap()
            .status,
        TaskLifecycleStatus::Failed
    );
    assert_eq!(
        restarted
            .download_snapshot("nisar-changed-validator-task")
            .unwrap()
            .results[0],
        failed
    );
    assert!(!final_output.exists());
    assert_eq!(
        fs::read(part_path(&final_output)).unwrap(),
        fixture.body[..split]
    );
    restarted.shutdown().unwrap();
}

#[test]
fn checksum_correct_specification_wrong_hdf5_is_not_promoted() {
    let _serial = serial_test();
    let fixture = ProductFixture::specification_wrong("specification-wrong");
    let temp = TempRoot::new("specification-wrong");
    let catalog = ScriptedServer::start(
        "/search/granules.umm_json",
        vec![catalog_response(captured_item(&fixture))],
    );
    let transfer =
        ScriptedServer::start(&format!("/{FILE_NAME}"), vec![full_response(&fixture.body)]);
    let service = build_service(
        &temp.database,
        &catalog.url,
        &transfer.url,
        1,
        Arc::new(PlaceholderCredentials::default()),
    );
    let (batch_id, item_id) = freeze_and_select(&service, "specification-wrong", &fixture);
    let output_root = temp.root.join("output");
    let final_output = destination(&output_root);
    create_task(
        &service,
        "nisar-specification-wrong-task",
        &batch_id,
        &item_id,
        &output_root,
    );

    wait_until("NISAR structural rejection", || {
        runtime(&service, "nisar-specification-wrong-task")
            .is_some_and(|runtime| runtime.status == TaskLifecycleStatus::Failed)
    });
    let result = &service
        .download_snapshot("nisar-specification-wrong-task")
        .unwrap()
        .results[0];
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(
        result.failure.as_ref().unwrap().code,
        "NISAR_HDF5_STRUCTURE_INVALID"
    );
    assert!(!final_output.exists());
    assert_eq!(fs::read(part_path(&final_output)).unwrap(), fixture.body);
    service.shutdown().unwrap();
    assert_eq!(catalog.finish().len(), 1);
    assert_eq!(transfer.finish().len(), 1);
}
