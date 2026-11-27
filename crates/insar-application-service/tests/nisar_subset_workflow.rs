use std::fs;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, NisarCreateRemoteSubsetTaskRequest,
};
use insar_core::{
    parse_request_manifest, AsfRemoteSearchRequest, AsfRemoteSearchResponse, AssetAccessDescriptor,
    AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters,
    AssetRole, CatalogSearchQuery, CatalogSearchResponse, ContractError, DownloadItemProgress,
    DownloadItemResult, DownloadOutcome, FrozenAssetAccess, FrozenSearchBatch,
    NisarSubsetArtifactManifestV1, NisarSubsetFrequency, ProviderResult, SearchWorkbenchState,
    SnapshotItem, StableAssetReference, StoreRestoreRequest, StoreSearchBatchRequest,
};
use insar_download::{
    offline_nisar_subset_acceptance_snapshot, offline_nisar_subset_acceptance_writer,
    prepare_offline_nisar_subset_acceptance_case, rebind_offline_nisar_subset_acceptance_request,
    DownloadWork, DownloadWorker, OfflineNisarSubsetAcceptanceCase, WorkerControl,
};
use insar_provider::asf::nisar_subset::{
    normalize_nisar_subset_aoi, plan_nisar_subset, NisarSubsetEvidenceWindowV1,
    NisarSubsetGeolocationEvidenceV1,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

const FILE_NAME: &str =
    "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001.h5";
const PRODUCT_KEY: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF";
const ASSET_KEY: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF::ASSET::science-rslc-hdf5";

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-nisar-subset-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        )))
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

struct TempOutput(PathBuf);

impl TempOutput {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-nisar-subset-output-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct FixtureAsf;

impl LiveAsfSearch for FixtureAsf {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(fixture_error())
    }
}

#[derive(Default)]
struct FixtureCatalog;

impl LiveCatalogSearch for FixtureCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        Err(fixture_error())
    }
}

fn fixture_error() -> ContractError {
    ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap()
}

#[derive(Default)]
struct RecordingWorker {
    items: Mutex<Vec<SnapshotItem>>,
}

impl DownloadWorker for RecordingWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.items.lock().unwrap().push(work.item.clone());
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root.join("fixture-derived.h5"),
            bytes_written: 173_806_009,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-01T00:00:01Z".to_owned(),
        }
    }
}

struct OfflineVerticalPrepared {
    evidence: NisarSubsetGeolocationEvidenceV1,
    item: SnapshotItem,
    command_sender: mpsc::Sender<OfflineVerticalCommand>,
}

struct OfflineVerticalCommand {
    work: DownloadWork,
    control: WorkerControl,
    reply_sender: mpsc::Sender<OfflineVerticalReply>,
}

struct OfflineVerticalReply {
    result: DownloadItemResult,
    artifact: Option<NisarSubsetArtifactManifestV1>,
    artifact_path: Option<PathBuf>,
}

fn spawn_offline_vertical_owner() -> mpsc::Receiver<OfflineVerticalPrepared> {
    let (prepared_sender, prepared_receiver) = mpsc::channel();
    thread::spawn(move || {
        let case = prepare_offline_nisar_subset_acceptance_case(
            "application-vertical-owner",
            1,
            NisarSubsetFrequency::A,
            "HH",
            "/science/LSAR/RSLC/swaths/frequencyA/HH",
            "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask",
        );
        let (command_sender, command_receiver) = mpsc::channel();
        prepared_sender
            .send(OfflineVerticalPrepared {
                evidence: evidence_from_acceptance_case(&case),
                item: offline_nisar_subset_acceptance_snapshot(&case.request),
                command_sender,
            })
            .expect("main test receives the owner-thread fixture preparation");
        let command = command_receiver
            .recv()
            .expect("worker sends the one owner-thread fixture command");
        let OfflineVerticalCommand {
            work,
            control,
            reply_sender,
        } = command;
        if control.is_cancelled() {
            let result = DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id,
                outcome: DownloadOutcome::Cancelled,
                destination: work.task.output_root,
                bytes_written: 0,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-08-01T00:00:01Z".to_owned(),
            };
            drop(case);
            reply_sender
                .send(OfflineVerticalReply {
                    result,
                    artifact: None,
                    artifact_path: None,
                })
                .expect("worker receives the cancelled owner-thread result");
            return;
        }
        let request = parse_request_manifest(&work.item.metadata)
            .expect("owner reads the exact Store-frozen Request");
        let case = rebind_offline_nisar_subset_acceptance_request(case, request.clone());
        let result = offline_nisar_subset_acceptance_writer(&work.task.output_root)
            .write_and_validate(
                &request,
                &case.plan,
                &case.transfer,
                &case.workspace,
                &case.metadata,
                &|| control.is_cancelled(),
            )
            .expect("P1.3 writer validates and atomically promotes the derived artifact");
        let bytes_written = fs::metadata(result.artifact_path()).unwrap().len();
        let destination = result.artifact_path().to_path_buf();
        let reply = OfflineVerticalReply {
            result: DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id,
                outcome: DownloadOutcome::Completed,
                destination: destination.clone(),
                bytes_written,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-08-01T00:00:02Z".to_owned(),
            },
            artifact: Some(result.manifest().clone()),
            artifact_path: Some(destination),
        };
        drop(case);
        reply_sender
            .send(reply)
            .expect("worker receives the completed owner-thread result");
    });
    prepared_receiver
}

struct OfflineVerticalWorker {
    command_sender: Mutex<Option<mpsc::Sender<OfflineVerticalCommand>>>,
    artifact: Mutex<Option<NisarSubsetArtifactManifestV1>>,
    artifact_path: Mutex<Option<PathBuf>>,
}

impl DownloadWorker for OfflineVerticalWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let (reply_sender, reply_receiver) = mpsc::channel();
        self.command_sender
            .lock()
            .unwrap()
            .take()
            .expect("one Store task executes the one offline acceptance case")
            .send(OfflineVerticalCommand {
                work,
                control,
                reply_sender,
            })
            .expect("owner thread receives the one offline acceptance command");
        let reply = reply_receiver
            .recv()
            .expect("owner thread returns the offline acceptance result");
        if let Some(artifact) = reply.artifact {
            *self.artifact.lock().unwrap() = Some(artifact);
        }
        if let Some(artifact_path) = reply.artifact_path {
            *self.artifact_path.lock().unwrap() = Some(artifact_path);
        }
        reply.result
    }
}

fn aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [37.720222204639484, 10.857787104635449],
            [37.721296026021705, 10.857533983552843],
            [37.721144257142576, 10.85690570018474],
            [37.720070447767895, 10.857158813483217],
            [37.720222204639484, 10.857787104635449]
        ]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn snapshot() -> SnapshotItem {
    SnapshotItem {
        item_id: PRODUCT_KEY.to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: FILE_NAME.trim_end_matches(".h5").to_owned(),
        acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
        footprint_geojson: Some(
            json!({
                "type": "Polygon",
                "coordinates": [[[36.0, 9.0], [39.0, 9.0], [39.0, 12.0], [36.0, 12.0], [36.0, 9.0]]]
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        metadata: Map::from_iter([
            (
                "logical_dataset".to_owned(),
                json!("sar.nisar.l.rslc.provisional.v1"),
            ),
            ("band".to_owned(), json!("L")),
            ("processing_level".to_owned(), json!("RSLC")),
            ("science_level".to_owned(), json!("L1")),
            ("production_configuration".to_owned(), json!("PR")),
            ("crid".to_owned(), json!("P05023")),
            ("product_specification_version".to_owned(), json!("1.5.0")),
            (
                "collection_profile_id".to_owned(),
                json!("asf.nisar.l.rslc.provisional.v1"),
            ),
            ("frequency_a_polarization".to_owned(), json!("HH+HV")),
            ("frequency_b_polarization".to_owned(), json!("HH+HV")),
        ]),
        payload: Map::new(),
        asset_access: Some(FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: "asf.nasa.earthdata".to_owned(),
                collection_id: "C2853145197-ASF".to_owned(),
                provider_product_id: "G9900000001-ASF".to_owned(),
                provider_product_key: PRODUCT_KEY.to_owned(),
                canonical_product_key: None,
                asset_key: "science-rslc-hdf5".to_owned(),
                stable_asset_key: ASSET_KEY.to_owned(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: "earthdata_https".to_owned(),
                scheme: AssetAccessScheme::EarthdataHttps,
                stable_href: None,
                requires_credentials: false,
                credential_profile: None,
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(FILE_NAME.to_owned()),
            expected_size_bytes: Some(14_252_244_992),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some(
                "ee3de9b9877b4f8e35153369725dcb4174ef5f2423af167e77f843e08860a9a0".to_owned(),
            ),
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: "asf.nisar.earthdata-complete-hdf5.v2".to_owned(),
            collection_profile_id: Some("asf.nisar.l.rslc.provisional.v1".to_owned()),
        }),
    }
}

fn source_batch_with(item: SnapshotItem) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: "nisar-source-batch".to_owned(),
        provider_id: item.provider_id.clone(),
        created_at: "2026-07-31T00:00:00Z".to_owned(),
        aoi_name: "reviewed fixture AOI".to_owned(),
        aoi_geojson: Some(aoi()),
        query: Map::new(),
        search_stats: None,
        candidates: vec![item],
    }
}

fn freeze_source_with(path: &PathBuf, item: SnapshotItem, checked: bool) {
    let mut store = TaskStore::open(path).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: source_batch_with(item),
            workbench: SearchWorkbenchState {
                batch_id: "nisar-source-batch".to_owned(),
                checked_item_ids: checked
                    .then(|| PRODUCT_KEY.to_owned())
                    .into_iter()
                    .collect(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
}

fn freeze_source(path: &PathBuf) {
    freeze_source_with(path, snapshot(), true);
}

fn evidence(
    frozen_aoi: &Map<String, Value>,
    frequency: NisarSubsetFrequency,
) -> NisarSubsetGeolocationEvidenceV1 {
    NisarSubsetGeolocationEvidenceV1 {
        aoi_sha256: normalize_nisar_subset_aoi(frozen_aoi).unwrap().sha256,
        algorithm: "sparse_grid_polygon_cells_v1".to_owned(),
        algorithm_version: 1,
        evidence_digest: "3d2d4c8ac55d67f64bfeaac619584bd00f3d20eace6e87bd13cc6e4446ca45c0"
            .to_owned(),
        product_specification_version: "1.5.0".to_owned(),
        crid: "P05023".to_owned(),
        frequency,
        swath: "LSAR_RSLC".to_owned(),
        source_shape: [70_000, 30_000],
        chunk_shape: [512, 512],
        dtype: "CFloat32".to_owned(),
        windows: vec![NisarSubsetEvidenceWindowV1 {
            component_id: 0,
            line_start: 26_112,
            line_end_exclusive: 26_624,
            sample_start: 12_800,
            sample_end_exclusive: 13_312,
            margin_lines: 0,
            margin_samples: 0,
        }],
        estimated_remote_bytes: 198_995_157,
        estimated_output_bytes: 173_806_009,
    }
}

fn evidence_from_acceptance_case(
    case: &OfflineNisarSubsetAcceptanceCase,
) -> NisarSubsetGeolocationEvidenceV1 {
    let window = &case.request.mapping.windows[0];
    NisarSubsetGeolocationEvidenceV1 {
        aoi_sha256: case.request.request.aoi_sha256.clone(),
        algorithm: case.request.mapping.algorithm.clone(),
        algorithm_version: case.request.mapping.algorithm_version,
        evidence_digest: case.request.mapping.evidence_digest.clone(),
        product_specification_version: case.request.source.product_specification_version.clone(),
        crid: case.request.source.crid.clone(),
        frequency: case.request.request.frequency.clone(),
        swath: case.request.request.swath.clone(),
        source_shape: window.source_shape,
        chunk_shape: window.chunk_shape,
        dtype: window.dtype.clone(),
        windows: vec![NisarSubsetEvidenceWindowV1 {
            component_id: window.component_id,
            line_start: window.line_start,
            line_end_exclusive: window.line_end_exclusive,
            sample_start: window.sample_start,
            sample_end_exclusive: window.sample_end_exclusive,
            margin_lines: window.margin_lines,
            margin_samples: window.margin_samples,
        }],
        estimated_remote_bytes: case.request.expected_output.estimated_remote_bytes,
        estimated_output_bytes: case.request.expected_output.estimated_output_bytes,
    }
}

fn request(output_root: PathBuf) -> NisarCreateRemoteSubsetTaskRequest {
    NisarCreateRemoteSubsetTaskRequest {
        request_id: "request-a-hh".to_owned(),
        source_batch_id: "nisar-source-batch".to_owned(),
        item_id: PRODUCT_KEY.to_owned(),
        task_id: "nisar-subset-task-a-hh".to_owned(),
        output_root,
        layout: insar_core::DownloadLayout::Direct,
        concurrency: 2,
        frequency: NisarSubsetFrequency::A,
        polarization: "HH".to_owned(),
        created_at: "2026-08-01T00:00:00Z".to_owned(),
    }
}

#[test]
fn normal_business_runtime_is_invisible_disabled_and_fail_closed_before_store_access() {
    let database = TempStore::new("disabled");
    let worker = Arc::new(RecordingWorker::default());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        worker.clone(),
    );
    let capability = service.nisar_remote_subset_capability().unwrap();
    assert!(capability.visible);
    assert!(!capability.enabled);
    assert!(!capability.production_enabled);
    assert!(!capability.default_enabled);
    assert_eq!(
        service
            .create_nisar_remote_subset_task(&request(std::env::temp_dir()))
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_NOT_ENABLED"
    );
    assert!(
        !database.0.exists(),
        "disabled command must not open the Store"
    );
    assert!(worker.items.lock().unwrap().is_empty());
}

#[test]
fn fixture_command_atomically_creates_one_inactive_derived_batch_and_existing_store_task() {
    let database = TempStore::new("enabled");
    freeze_source(&database.0);
    let source_before = TaskStore::open(&database.0)
        .unwrap()
        .load_workbench()
        .unwrap()
        .unwrap();
    let source_aoi = source_before
        .batch
        .aoi_geojson
        .as_ref()
        .expect("fixture batch AOI");
    let fixture_evidence = evidence(source_aoi, NisarSubsetFrequency::A);
    assert_eq!(fixture_evidence.evidence_digest.len(), 64);
    assert_eq!(fixture_evidence.algorithm, "sparse_grid_polygon_cells_v1");
    assert_eq!(fixture_evidence.algorithm_version, 1);
    assert_eq!(fixture_evidence.product_specification_version, "1.5.0");
    assert_eq!(fixture_evidence.crid, "P05023");
    assert_eq!(fixture_evidence.frequency, NisarSubsetFrequency::A);
    assert_eq!(fixture_evidence.swath, "LSAR_RSLC");
    assert_eq!(source_before.batch.candidates[0], snapshot());
    assert_eq!(
        fixture_evidence.aoi_sha256,
        normalize_nisar_subset_aoi(source_aoi).unwrap().sha256
    );
    plan_nisar_subset(
        &source_before.batch.candidates[0],
        source_aoi,
        NisarSubsetFrequency::A,
        "HH",
        "LSAR_RSLC",
        &fixture_evidence,
    )
    .expect("the P1.1 planner accepts the fixture before Store integration");
    let worker = Arc::new(RecordingWorker::default());
    let service = ApplicationService::with_offline_nisar_subset_fixture(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(FixtureCatalog),
        worker.clone(),
        vec![fixture_evidence],
    );
    let capability = service.nisar_remote_subset_capability().unwrap();
    assert!(capability.visible && capability.enabled);
    assert!(capability.live_verified && !capability.production_enabled);
    let output = std::env::temp_dir().join("nisar-subset-output");
    service
        .create_nisar_remote_subset_task(&request(output))
        .expect("one derived task is stored and scheduled");

    let deadline = Instant::now() + Duration::from_secs(5);
    while worker.items.lock().unwrap().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let items = worker.items.lock().unwrap();
    assert_eq!(items.len(), 1);
    let manifest = parse_request_manifest(&items[0].metadata).unwrap();
    assert_eq!(manifest.request.frequency, NisarSubsetFrequency::A);
    assert_eq!(manifest.request.polarization, "HH");
    assert_eq!(items[0].metadata["output_authority"], "client_derived");
    assert_eq!(items[0].metadata["implementation_scope"], "offline_only");
    drop(items);
    service.shutdown().unwrap();

    let reopened = TaskStore::open(&database.0).unwrap();
    assert_eq!(reopened.load_workbench().unwrap().unwrap(), source_before);
    assert!(reopened
        .load_task("nisar-subset-task-a-hh")
        .unwrap()
        .is_some());
    let restored = reopened
        .restore(&StoreRestoreRequest {
            include_terminal: true,
        })
        .unwrap();
    assert_eq!(restored.tasks.len(), 1);
    assert_eq!(
        restored.tasks[0].batch_id,
        "nisar-subset-nisar-subset-task-a-hh"
    );
}

#[test]
fn source_scope_and_checked_state_fail_closed_without_store_residue() {
    for (label, field, value, checked) in [
        ("wrong-provider", "provider", "wrong.provider", true),
        ("wrong-product", "processing_level", "GCOV", true),
        ("wrong-production", "production_configuration", "OD", true),
        ("wrong-crid", "crid", "P99999", true),
        (
            "wrong-specification",
            "product_specification_version",
            "1.4.0",
            true,
        ),
        ("unchecked", "", "", false),
    ] {
        let database = TempStore::new(label);
        let mut item = snapshot();
        if field == "provider" {
            item.provider_id = value.to_owned();
        } else if !field.is_empty() {
            item.metadata.insert(field.to_owned(), json!(value));
        }
        freeze_source_with(&database.0, item, checked);
        let frozen = TaskStore::open(&database.0)
            .unwrap()
            .load_workbench()
            .unwrap()
            .unwrap();
        let service = ApplicationService::with_offline_nisar_subset_fixture(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(FixtureCatalog),
            Arc::new(RecordingWorker::default()),
            vec![evidence(
                frozen
                    .batch
                    .aoi_geojson
                    .as_ref()
                    .expect("fixture batch AOI"),
                NisarSubsetFrequency::A,
            )],
        );
        let error = service
            .create_nisar_remote_subset_task(&request(std::env::temp_dir()))
            .unwrap_err();
        assert_eq!(
            error.code(),
            "NISAR_SUBSET_SOURCE_INVALID",
            "{label}: {error:?}"
        );
        service.shutdown().unwrap();
        let reopened = TaskStore::open(&database.0).unwrap();
        assert_eq!(reopened.load_workbench().unwrap().unwrap(), frozen);
        assert!(reopened
            .restore(&StoreRestoreRequest {
                include_terminal: true,
            })
            .unwrap()
            .tasks
            .is_empty());
    }
}

#[test]
fn unsupported_selection_and_mismatched_ids_do_not_fall_back_to_complete_product() {
    let database = TempStore::new("unsupported-selection");
    freeze_source(&database.0);
    let frozen = TaskStore::open(&database.0)
        .unwrap()
        .load_workbench()
        .unwrap()
        .unwrap();
    let worker = Arc::new(RecordingWorker::default());
    let service = ApplicationService::with_offline_nisar_subset_fixture(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(FixtureCatalog),
        worker.clone(),
        vec![evidence(
            frozen
                .batch
                .aoi_geojson
                .as_ref()
                .expect("fixture batch AOI"),
            NisarSubsetFrequency::A,
        )],
    );

    let mut unsupported = request(std::env::temp_dir());
    unsupported.polarization = "VV".to_owned();
    assert_eq!(
        service
            .create_nisar_remote_subset_task(&unsupported)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_SELECTION_UNSUPPORTED"
    );
    let mut wrong_batch = request(std::env::temp_dir());
    wrong_batch.source_batch_id = "other-batch".to_owned();
    assert_eq!(
        service
            .create_nisar_remote_subset_task(&wrong_batch)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_SOURCE_INVALID"
    );
    let mut wrong_item = request(std::env::temp_dir());
    wrong_item.item_id = "other-item".to_owned();
    assert_eq!(
        service
            .create_nisar_remote_subset_task(&wrong_item)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_SOURCE_INVALID"
    );
    assert!(worker.items.lock().unwrap().is_empty());
    service.shutdown().unwrap();
    let reopened = TaskStore::open(&database.0).unwrap();
    assert_eq!(reopened.load_workbench().unwrap().unwrap(), frozen);
    assert!(reopened
        .restore(&StoreRestoreRequest {
            include_terminal: true,
        })
        .unwrap()
        .tasks
        .is_empty());
}

#[test]
fn application_store_scheduler_range_writer_artifact_and_reopen_form_one_offline_chain() {
    // The writer validates the derived HDF5, which requires app-owned plugin
    // isolation; this test host must establish it (no external env sentinel).
    insar_download::establish_hdf5_plugin_isolation()
        .expect("HDF5 plugin isolation must be established by the test host");
    let prepared = spawn_offline_vertical_owner()
        .recv()
        .expect("owner thread sends the prepared fixture to the main test thread");
    let mut evidence = prepared.evidence;
    let mut item = prepared.item;
    item.metadata.insert(
        "logical_dataset".to_owned(),
        json!("sar.nisar.l.rslc.provisional.v1"),
    );
    let product_key = item.item_id.clone();
    let database = TempStore::new("vertical");
    freeze_source_with(&database.0, item, true);
    let source_before = TaskStore::open(&database.0)
        .unwrap()
        .load_workbench()
        .unwrap()
        .unwrap();
    let source_aoi = source_before
        .batch
        .aoi_geojson
        .as_ref()
        .expect("fixture batch AOI");
    evidence.aoi_sha256 = normalize_nisar_subset_aoi(source_aoi).unwrap().sha256;
    let output = TempOutput::new("vertical");
    let output_root = output.0.join("application-output");
    let worker = Arc::new(OfflineVerticalWorker {
        command_sender: Mutex::new(Some(prepared.command_sender)),
        artifact: Mutex::new(None),
        artifact_path: Mutex::new(None),
    });
    let service = ApplicationService::with_offline_nisar_subset_fixture(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(FixtureCatalog),
        worker.clone(),
        vec![evidence],
    );
    service
        .create_nisar_remote_subset_task(&NisarCreateRemoteSubsetTaskRequest {
            request_id: "vertical-request".to_owned(),
            source_batch_id: "nisar-source-batch".to_owned(),
            item_id: product_key,
            task_id: "vertical-task".to_owned(),
            output_root: output_root.clone(),
            layout: insar_core::DownloadLayout::Direct,
            concurrency: 2,
            frequency: NisarSubsetFrequency::A,
            polarization: "HH".to_owned(),
            created_at: "2026-08-01T00:00:00Z".to_owned(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while worker.artifact.lock().unwrap().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let artifact = worker
        .artifact
        .lock()
        .unwrap()
        .clone()
        .expect("Scheduler completed the real offline Range/writer chain");
    assert!(artifact.complete);
    assert_eq!(artifact.output_authority, "client_derived");
    assert!(artifact.remote_bytes < artifact.full_product_bytes);
    let artifact_path = worker
        .artifact_path
        .lock()
        .unwrap()
        .clone()
        .expect("worker records the atomically promoted artifact path");
    assert!(artifact_path.is_file());
    let terminal_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = service.snapshot(true).unwrap();
        if snapshot
            .restore
            .runtimes
            .iter()
            .all(|runtime| runtime.active_item_ids.is_empty())
        {
            break;
        }
        assert!(
            Instant::now() < terminal_deadline,
            "Store did not reach terminal state"
        );
        thread::sleep(Duration::from_millis(20));
    }
    service.shutdown().unwrap();

    let reopened = TaskStore::open(&database.0).unwrap();
    assert_eq!(reopened.load_workbench().unwrap().unwrap(), source_before);
    let restored = reopened
        .restore(&StoreRestoreRequest {
            include_terminal: true,
        })
        .unwrap();
    assert_eq!(restored.tasks.len(), 1);
    assert_eq!(restored.runtimes.len(), 1);
    assert!(restored.runtimes[0].active_item_ids.is_empty());
    let serialized = serde_json::to_string(&restored).unwrap();
    for forbidden in ["Authorization", "Cookie", "Bearer ", "https://", "password"] {
        assert!(
            !serialized.contains(forbidden),
            "persisted secret: {forbidden}"
        );
    }
}
