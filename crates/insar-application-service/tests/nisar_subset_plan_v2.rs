use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, DemOperationPort,
    NisarConfirmSubsetTaskV2Request, NisarPrepareSubsetIntentV2Request,
    NisarSubsetDependencyEvidenceV2Fixture,
};
use insar_core::{
    canonicalize_nisar_subset_geojson_v2, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
    AssetResolverParameters, AssetRole, CatalogSearchQuery, CatalogSearchResponse, ContractError,
    DownloadItemProgress, DownloadItemResult, DownloadLayout, DownloadOutcome, FrozenAssetAccess,
    FrozenSearchBatch, NisarSubsetDependencyEvidenceAuthorityV2, NisarSubsetDependencyRelationV2,
    NisarSubsetGeometryRoleV2, NisarSubsetPlanMetadataEvidenceV2, NisarSubsetPlanV2,
    NisarSubsetSourceDependencyEdgeV2, NisarSubsetSourceDependencyGraphV2,
    NisarSubsetSourceDependencyNodeKindV2, NisarSubsetSourceDependencyNodeV2, ProviderResult,
    SearchWorkbenchState, SnapshotItem, StableAssetReference, StoreSearchBatchRequest,
    StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
};
use insar_download::{
    DownloadWork, DownloadWorker, NisarExecutionPlanFreezeAuthority, WorkerControl,
};
use insar_provider::asf::nisar_subset::{
    prepare_nisar_subset_intent_v2 as provider_prepare_nisar_subset_intent_v2,
    AsfNisarSubsetPreparedIntentDraftV2,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::{
    parse_nisar_subset_plan_v2_envelope, parse_nisar_subset_prepared_intent_v2_envelope,
    StoreAttachNisarSubsetExecutionPlanV2Request, TaskStore,
};
use serde_json::{json, Map, Value};

const ITEM_ID: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF";
const ASSET_KEY: &str = "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF::ASSET::science-rslc-hdf5";

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-nisar-v2-{label}-{}-{nonce}.sqlite3",
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

#[derive(Default)]
struct FixtureSearch;

impl LiveAsfSearch for FixtureSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(fixture_error())
    }
}

impl LiveCatalogSearch for FixtureSearch {
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
struct CountingWorker(AtomicUsize);

impl DownloadWorker for CountingWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root.join("unexpected-v2-execution.h5"),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-29T00:00:01Z".to_owned(),
        }
    }
}

#[derive(Default)]
struct AuthorityRecordingWorker {
    direct_execute_count: AtomicUsize,
    authority_execute_count: AtomicUsize,
}

impl AuthorityRecordingWorker {
    fn result(work: DownloadWork) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root.join("authority-forwarded-v2.h5"),
            bytes_written: 1,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-31T00:00:01Z".to_owned(),
        }
    }
}

impl DownloadWorker for AuthorityRecordingWorker {
    fn execute_with_authority(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _plan_freeze: &NisarExecutionPlanFreezeAuthority,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.authority_execute_count.fetch_add(1, Ordering::SeqCst);
        Self::result(work)
    }

    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.direct_execute_count.fetch_add(1, Ordering::SeqCst);
        Self::result(work)
    }
}

struct UnusedDemPort;

impl DemOperationPort for UnusedDemPort {
    fn call(&self, _operation: &str, _payload: Value) -> Result<Value, String> {
        panic!("NISAR V2 authority forwarding must not enter the DEM port")
    }
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn rectangle(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Map<String, Value> {
    map(json!({
        "type": "Polygon",
        "coordinates": [[
            [min_x, min_y], [max_x, min_y], [max_x, max_y],
            [min_x, max_y], [min_x, min_y]
        ]]
    }))
}

fn draft_geometry() -> Map<String, Value> {
    map(json!({
        "type": "MultiPolygon",
        "coordinates": [
            [
                [[-1.0, -1.0], [6.0, -1.0], [6.0, 6.0], [-1.0, 6.0], [-1.0, -1.0]],
                [[1.0, 1.0], [1.0, 2.0], [2.0, 2.0], [2.0, 1.0], [1.0, 1.0]]
            ],
            [[[8.0, 8.0], [12.0, 8.0], [12.0, 12.0], [8.0, 12.0], [8.0, 8.0]]]
        ]
    }))
}

fn source_item() -> SnapshotItem {
    SnapshotItem {
        item_id: ITEM_ID.to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001".to_owned(),
        acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
        footprint_geojson: Some(rectangle(0.0, 0.0, 10.0, 10.0)),
        metadata: Map::from_iter([
            ("logical_dataset".to_owned(), json!("sar.nisar.l.rslc.provisional.v1")),
            ("band".to_owned(), json!("L")),
            ("processing_level".to_owned(), json!("RSLC")),
            ("science_level".to_owned(), json!("L1")),
            ("production_configuration".to_owned(), json!("PR")),
            ("crid".to_owned(), json!("P05023")),
            ("product_specification_version".to_owned(), json!("1.5.0")),
            ("collection_profile_id".to_owned(), json!("asf.nisar.l.rslc.provisional.v1")),
            ("quality_status".to_owned(), json!("Provisional")),
            (
                "frequency_a_polarization".to_owned(),
                json!("HH+HV+VH+VV"),
            ),
            ("frequency_b_polarization".to_owned(), json!("HH+HV")),
        ]),
        payload: Map::new(),
        asset_access: Some(FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: "asf.nasa.earthdata".to_owned(),
                collection_id: "C2853145197-ASF".to_owned(),
                provider_product_id: "G9900000001-ASF".to_owned(),
                provider_product_key: ITEM_ID.to_owned(),
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
            file_name: Some(format!("{}.h5", source_item_name())),
            expected_size_bytes: Some(14_252_244_992),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some(
                "ee3de9b9877b4f8e35153369725dcb4174ef5f2423af167e77f843e08860a9a0"
                    .to_owned(),
            ),
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: "asf.nisar.earthdata-complete-hdf5.v2".to_owned(),
            collection_profile_id: Some("asf.nisar.l.rslc.provisional.v1".to_owned()),
        }),
    }
}

fn source_item_name() -> &'static str {
    "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001"
}

fn freeze_source(path: &Path, checked: bool, item: SnapshotItem) -> FrozenSearchBatch {
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "nisar-source-batch".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        created_at: "2026-08-29T00:00:00Z".to_owned(),
        aoi_name: "Working AOI A".to_owned(),
        aoi_geojson: Some(rectangle(3.0, 3.0, 4.0, 4.0)),
        query: Map::new(),
        search_stats: None,
        candidates: vec![item],
    };
    TaskStore::open(path)
        .unwrap()
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: checked.then(|| ITEM_ID.to_owned()).into_iter().collect(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    batch
}

fn node(
    kind: NisarSubsetSourceDependencyNodeKindV2,
    source_path: impl Into<String>,
) -> NisarSubsetSourceDependencyNodeV2 {
    NisarSubsetSourceDependencyNodeV2 {
        source_path: source_path.into(),
        kind,
        attribute_name: (kind == NisarSubsetSourceDependencyNodeKindV2::Attribute)
            .then(|| "Conventions".to_owned()),
    }
}

fn edge(
    dependent_path: impl Into<String>,
    dependency_path: impl Into<String>,
    relation: NisarSubsetDependencyRelationV2,
) -> NisarSubsetSourceDependencyEdgeV2 {
    NisarSubsetSourceDependencyEdgeV2 {
        dependent_path: dependent_path.into(),
        dependency_path: dependency_path.into(),
        relation,
    }
}

fn fixture_graph() -> NisarSubsetSourceDependencyGraphV2 {
    use NisarSubsetDependencyRelationV2 as Relation;
    use NisarSubsetSourceDependencyNodeKindV2 as Kind;

    let identification = "/science/LSAR/identification/granuleId";
    let provenance = "/science/LSAR/identification/productVersion";
    let attribute = "/";
    let other = "/science/LSAR/RSLC/metadata/orbit";
    let mut nodes = vec![
        node(Kind::IdentificationMetadata, identification),
        node(Kind::ProvenanceMetadata, provenance),
        node(Kind::Attribute, attribute),
        node(Kind::OtherScientificDependency, other),
    ];
    let mut edges = Vec::new();
    for (frequency, polarizations) in [("A", &["HH", "HV"][..]), ("B", &["HH"][..])] {
        let coordinate =
            format!("/science/LSAR/RSLC/metadata/geolocationGrid/frequency{frequency}/coordinateX");
        let dimension = format!("/science/LSAR/RSLC/swaths/frequency{frequency}/zeroDopplerTime");
        let mask = format!("/science/LSAR/RSLC/swaths/frequency{frequency}/inputDataExceptionMask");
        nodes.extend([
            node(Kind::CoordinateDataset, &coordinate),
            node(Kind::DimensionScale, &dimension),
            node(Kind::ValidityMask, &mask),
        ]);
        for polarization in polarizations {
            let science = format!("/science/LSAR/RSLC/swaths/frequency{frequency}/{polarization}");
            nodes.push(node(Kind::ScienceDataset, &science));
            edges.extend([
                edge(&science, &coordinate, Relation::Coordinate),
                edge(&science, &dimension, Relation::DimensionScale),
                edge(&science, &mask, Relation::ValidityMask),
                edge(&science, identification, Relation::RequiredMetadata),
                edge(&science, provenance, Relation::Provenance),
                edge(&science, attribute, Relation::RequiredAttribute),
                edge(&science, other, Relation::OtherScientificDependency),
            ]);
        }
    }
    NisarSubsetSourceDependencyGraphV2 {
        graph_contract: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT.to_owned(),
        schema_version: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
        evidence_authority: NisarSubsetDependencyEvidenceAuthorityV2::ReviewedSourceEvidenceV1,
        evidence_sha256: "c".repeat(64),
        nodes,
        edges,
    }
}

fn prepare_request() -> NisarPrepareSubsetIntentV2Request {
    NisarPrepareSubsetIntentV2Request {
        source_batch_id: "nisar-source-batch".to_owned(),
        source_item_id: ITEM_ID.to_owned(),
        expected_workbench_revision: 0,
        subset_draft_geometry: draft_geometry(),
        selected_frequency: "A".to_owned(),
        selected_polarizations: vec!["HV".to_owned(), "HH".to_owned()],
    }
}

fn confirm_request(
    prepared_intent: insar_core::NisarSubsetPreparedIntentV2,
    task_id: &str,
) -> NisarConfirmSubsetTaskV2Request {
    NisarConfirmSubsetTaskV2Request {
        source_batch_id: "nisar-source-batch".to_owned(),
        source_item_id: ITEM_ID.to_owned(),
        expected_workbench_revision: 0,
        prepared_intent,
        task_id: task_id.to_owned(),
        output_root: std::env::temp_dir().join(format!("{task_id}-output")),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-08-29T00:00:00Z".to_owned(),
    }
}

fn fixture_service(
    path: &Path,
    graph: NisarSubsetSourceDependencyGraphV2,
    worker: Arc<CountingWorker>,
) -> ApplicationService {
    ApplicationService::with_offline_nisar_subset_v2_fixture(
        ApplicationServiceConfig::business_store(path),
        Arc::new(FixtureSearch),
        Arc::new(FixtureSearch),
        worker,
        vec![NisarSubsetDependencyEvidenceV2Fixture {
            source_item_id: ITEM_ID.to_owned(),
            graph,
        }],
    )
}

#[test]
fn prepare_freezes_source_identity_and_independent_draft_without_store_mutation() {
    let database = TempStore::new("prepare");
    let batch = freeze_source(&database.0, true, source_item());
    let before = TaskStore::open(&database.0)
        .unwrap()
        .load_workbench()
        .unwrap()
        .unwrap();
    let worker = Arc::new(CountingWorker::default());
    let service = fixture_service(&database.0, fixture_graph(), worker.clone());
    let prepared = service
        .prepare_nisar_subset_intent_v2(&prepare_request())
        .unwrap();

    assert_eq!(prepared.source_batch_id, "nisar-source-batch");
    assert_eq!(prepared.source_item_id, ITEM_ID);
    assert_eq!(prepared.workbench_revision, 0);
    assert_eq!(
        prepared.intent.source_product_identity.provider_product_key,
        ITEM_ID
    );
    assert_eq!(
        prepared.intent.requested_geometry,
        canonicalize_nisar_subset_geojson_v2(
            &draft_geometry(),
            NisarSubsetGeometryRoleV2::Requested,
        )
        .unwrap()
    );
    assert_ne!(
        prepared.intent.requested_geometry.canonical_geojson,
        prepared.intent.effective_geometry.canonical_geojson
    );
    assert_eq!(prepared.intent.selected_polarizations, ["HH", "HV"]);
    assert_eq!(prepared.intent.requested_geometry.component_count, 2);
    assert_eq!(prepared.intent.requested_geometry.ring_count, 3);
    prepared.intent.validate().unwrap();
    let after = TaskStore::open(&database.0)
        .unwrap()
        .load_workbench()
        .unwrap()
        .unwrap();
    assert_eq!(after, before);
    assert!(TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("nisar-v2-task")
        .unwrap()
        .is_none());
    assert_eq!(after.batch, batch);
    assert_eq!(after.batch.aoi_geojson, Some(rectangle(3.0, 3.0, 4.0, 4.0)));
    assert_eq!(worker.0.load(Ordering::SeqCst), 0);
    service.shutdown().unwrap();
}

#[test]
fn destination_confirmation_persists_exact_intent_as_paused_task_and_reopens() {
    let database = TempStore::new("confirm");
    freeze_source(&database.0, true, source_item());
    let before = TaskStore::open(&database.0)
        .unwrap()
        .load_workbench()
        .unwrap()
        .unwrap();
    let worker = Arc::new(CountingWorker::default());
    let service = fixture_service(&database.0, fixture_graph(), worker.clone());
    let expected = service
        .prepare_nisar_subset_intent_v2(&prepare_request())
        .unwrap()
        .intent;
    assert!(TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("nisar-v2-task")
        .unwrap()
        .is_none());

    let prepare = prepare_request();
    service
        .confirm_nisar_subset_task_v2(&NisarConfirmSubsetTaskV2Request {
            source_batch_id: prepare.source_batch_id,
            source_item_id: prepare.source_item_id,
            expected_workbench_revision: prepare.expected_workbench_revision,
            prepared_intent: expected.clone(),
            task_id: "nisar-v2-task".to_owned(),
            output_root: std::env::temp_dir().join("nisar-v2-confirmed-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-29T00:00:00Z".to_owned(),
        })
        .unwrap();
    service.shutdown().unwrap();

    let reopened = TaskStore::open(&database.0).unwrap();
    let after = reopened.load_workbench().unwrap().unwrap();
    assert_eq!(after, before);
    let task = reopened.load_task_record("nisar-v2-task").unwrap().unwrap();
    assert_eq!(task.snapshot.batch_id, "nisar-source-batch");
    assert_eq!(task.runtime.status, TaskLifecycleStatus::Paused);
    assert!(task.runtime.task_paused);
    assert!(task.runtime.queued_item_ids.is_empty());
    assert_eq!(task.runtime.paused_item_ids, [ITEM_ID]);
    let envelope =
        parse_nisar_subset_prepared_intent_v2_envelope(&task.snapshot.candidates[0].metadata)
            .unwrap()
            .unwrap();
    assert_eq!(
        envelope.canonical_intent_json,
        expected.canonical_json().unwrap()
    );
    assert_eq!(
        envelope.canonical_intent_sha256,
        expected.canonical_intent_sha256
    );
    assert_eq!(envelope.intent().unwrap(), expected);
    assert!(
        parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[0].metadata)
            .unwrap()
            .is_none()
    );
    assert_eq!(worker.0.load(Ordering::SeqCst), 0);
}

#[test]
fn native_dem_wrapper_forwards_nisar_v2_scheduler_authority_without_direct_execute() {
    let database = TempStore::new("native-dem-authority-forward");
    freeze_source(&database.0, true, source_item());
    let worker = Arc::new(AuthorityRecordingWorker::default());
    let service = ApplicationService::with_offline_nisar_subset_v2_fixture(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureSearch),
        Arc::new(FixtureSearch),
        Arc::clone(&worker) as Arc<dyn DownloadWorker>,
        vec![NisarSubsetDependencyEvidenceV2Fixture {
            source_item_id: ITEM_ID.to_owned(),
            graph: fixture_graph(),
        }],
    )
    .attach_dem_operation_port(Arc::new(UnusedDemPort));
    let intent = service
        .prepare_nisar_subset_intent_v2(&prepare_request())
        .unwrap()
        .intent;
    service
        .confirm_nisar_subset_task_v2(&confirm_request(
            intent,
            "nisar-v2-native-dem-authority-forward",
        ))
        .unwrap();
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: "nisar-v2-native-dem-authority-forward".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeTask,
                item_ids: Vec::new(),
            },
            event_id: "nisar-v2-native-dem-authority-forward:0".to_owned(),
            occurred_at: "2026-08-31T00:00:00Z".to_owned(),
        })
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while worker.authority_execute_count.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "Native DEM wrapper never forwarded Scheduler V2 authority"
        );
        std::thread::yield_now();
    }
    assert_eq!(worker.authority_execute_count.load(Ordering::SeqCst), 1);
    assert_eq!(worker.direct_execute_count.load(Ordering::SeqCst), 0);
    service.shutdown().unwrap();
}

#[test]
fn reconfirm_rejects_existing_task_before_and_after_execution_plan_attach() {
    let database = TempStore::new("reconfirm");
    freeze_source(&database.0, true, source_item());
    let service = fixture_service(
        &database.0,
        fixture_graph(),
        Arc::new(CountingWorker::default()),
    );
    let intent = service
        .prepare_nisar_subset_intent_v2(&prepare_request())
        .unwrap()
        .intent;
    let request = confirm_request(intent.clone(), "nisar-v2-reconfirm-task");
    service.confirm_nisar_subset_task_v2(&request).unwrap();
    assert_eq!(
        service
            .confirm_nisar_subset_task_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_TASK_CONFLICT"
    );
    service.shutdown().unwrap();

    let mut store = TaskStore::open(&database.0).unwrap();
    let stored = store
        .load_task_record("nisar-v2-reconfirm-task")
        .unwrap()
        .unwrap();
    assert_eq!(stored.runtime.revision, 0);
    for (expected_revision, kind, item_ids, event_id) in [
        (0, TaskStateCommandKind::ResumeTask, vec![], "resume-task"),
        (
            1,
            TaskStateCommandKind::ResumeItems,
            vec![ITEM_ID.to_owned()],
            "resume-item",
        ),
        (
            2,
            TaskStateCommandKind::ActivateItems,
            vec![ITEM_ID.to_owned()],
            "activate-item",
        ),
    ] {
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "nisar-v2-reconfirm-task".to_owned(),
                expected_revision,
                command: StoreTaskCommand { kind, item_ids },
                event_id: format!("nisar-v2-reconfirm-task:{event_id}"),
                occurred_at: "2026-08-29T00:00:01Z".to_owned(),
            })
            .unwrap();
    }
    let plan = NisarSubsetPlanV2::build_from_prepared_intent(
        intent.clone(),
        fixture_graph(),
        NisarSubsetPlanMetadataEvidenceV2 {
            source_estimate: None,
            transfer_estimate: None,
            derived_estimate: None,
            warnings: Vec::new(),
        },
    )
    .unwrap();
    store
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: "nisar-v2-reconfirm-task".to_owned(),
            item_id: ITEM_ID.to_owned(),
            prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
            expected_revision: 3,
            occurred_at: "2026-08-29T00:00:02Z".to_owned(),
            plan,
        })
        .unwrap();
    drop(store);

    let service = fixture_service(
        &database.0,
        fixture_graph(),
        Arc::new(CountingWorker::default()),
    );
    assert_eq!(
        service
            .confirm_nisar_subset_task_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_TASK_CONFLICT"
    );
    service.shutdown().unwrap();
}

#[test]
fn production_shaped_service_prepares_without_dependency_evidence() {
    let database = TempStore::new("no-evidence");
    freeze_source(&database.0, true, source_item());
    let before = TaskStore::open(&database.0)
        .unwrap()
        .load_workbench()
        .unwrap()
        .unwrap();
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureSearch),
        Arc::new(FixtureSearch),
        Arc::new(CountingWorker::default()),
    );
    let prepared = service
        .prepare_nisar_subset_intent_v2(&prepare_request())
        .unwrap();
    prepared.intent.validate().unwrap();
    let canonical = prepared.intent.canonical_json().unwrap();
    let round_trip: insar_core::NisarSubsetPreparedIntentV2 =
        serde_json::from_str(&canonical).unwrap();
    assert_eq!(round_trip, prepared.intent);
    assert!(TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("nisar-v2-task")
        .unwrap()
        .is_none());
    assert_eq!(
        TaskStore::open(&database.0)
            .unwrap()
            .load_workbench()
            .unwrap()
            .unwrap(),
        before
    );
    service.shutdown().unwrap();
}

#[test]
fn prepare_accepts_actual_vh_and_a_vv_capability_and_normalizes_duplicates() {
    let database = TempStore::new("actual-capability");
    freeze_source(&database.0, true, source_item());
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureSearch),
        Arc::new(FixtureSearch),
        Arc::new(CountingWorker::default()),
    );
    let mut request = prepare_request();
    request.selected_polarizations = vec!["VV".to_owned(), "VH".to_owned(), "VV".to_owned()];
    let prepared = service.prepare_nisar_subset_intent_v2(&request).unwrap();

    assert_eq!(prepared.intent.selected_polarizations, ["VH", "VV"]);
    prepared.intent.validate().unwrap();
    service.shutdown().unwrap();
}

#[test]
fn prepare_frequency_b_capability_accepts_hh_and_hv_but_rejects_vv() {
    let database = TempStore::new("frequency-b-capability");
    freeze_source(&database.0, true, source_item());
    let service = fixture_service(
        &database.0,
        fixture_graph(),
        Arc::new(CountingWorker::default()),
    );
    for polarization in ["HH", "HV"] {
        let mut request = prepare_request();
        request.selected_frequency = "B".to_owned();
        request.selected_polarizations = vec![polarization.to_owned()];
        let prepared = service
            .prepare_nisar_subset_intent_v2(&request)
            .expect("authoritative Frequency B capability accepts advertised polarization");
        assert_eq!(
            prepared.intent.selected_frequency,
            insar_core::NisarSubsetFrequency::B
        );
        assert_eq!(prepared.intent.selected_polarizations, [polarization]);
    }
    let mut unsupported = prepare_request();
    unsupported.selected_frequency = "B".to_owned();
    unsupported.selected_polarizations = vec!["VV".to_owned()];
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&unsupported)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_POLARIZATION_UNSUPPORTED"
    );
    service.shutdown().unwrap();
}

#[test]
fn user_and_geometry_failures_have_stable_v2_taxonomy() {
    let database = TempStore::new("typed");
    freeze_source(&database.0, true, source_item());
    let service = fixture_service(
        &database.0,
        fixture_graph(),
        Arc::new(CountingWorker::default()),
    );
    let mut request = prepare_request();
    request.selected_frequency = "C".to_owned();
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_FREQUENCY_UNSUPPORTED"
    );
    let mut request = prepare_request();
    request.selected_polarizations = vec!["XX".to_owned()];
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_POLARIZATION_UNSUPPORTED"
    );
    let mut request = prepare_request();
    request.subset_draft_geometry = rectangle(20.0, 20.0, 21.0, 21.0);
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_EFFECTIVE_GEOMETRY_EMPTY"
    );
    let mut request = prepare_request();
    request.subset_draft_geometry = map(json!({"type":"Point", "coordinates":[1.0, 1.0]}));
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_DRAFT_INVALID"
    );
    let mut request = prepare_request();
    request.subset_draft_geometry = map(json!({
        "type":"Polygon",
        "coordinates":[[[170.0, 0.0], [-170.0, 0.0], [-170.0, 1.0], [170.0, 1.0], [170.0, 0.0]]]
    }));
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_ANTIMERIDIAN_UNSUPPORTED"
    );
    let mut request = prepare_request();
    request.expected_workbench_revision = 1;
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_SEARCH_BATCH_STALE"
    );
    service.shutdown().unwrap();
}

#[test]
fn source_batch_item_selection_and_identity_failures_are_distinct() {
    let database = TempStore::new("source-errors");
    freeze_source(&database.0, true, source_item());
    let service = fixture_service(
        &database.0,
        fixture_graph(),
        Arc::new(CountingWorker::default()),
    );
    let mut request = prepare_request();
    request.source_batch_id = "missing-batch".to_owned();
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_SOURCE_BATCH_UNAVAILABLE"
    );
    let mut request = prepare_request();
    request.source_item_id = "missing-item".to_owned();
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&request)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_SOURCE_ITEM_UNAVAILABLE"
    );
    service.shutdown().unwrap();

    let unchecked = TempStore::new("unchecked");
    freeze_source(&unchecked.0, false, source_item());
    let service = fixture_service(
        &unchecked.0,
        fixture_graph(),
        Arc::new(CountingWorker::default()),
    );
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&prepare_request())
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_ITEM_NOT_SELECTED"
    );
    service.shutdown().unwrap();

    let mismatch = TempStore::new("identity");
    let mut item = source_item();
    item.metadata
        .insert("quality_status".to_owned(), json!("FINAL"));
    freeze_source(&mismatch.0, true, item);
    let service = fixture_service(
        &mismatch.0,
        fixture_graph(),
        Arc::new(CountingWorker::default()),
    );
    assert_eq!(
        service
            .prepare_nisar_subset_intent_v2(&prepare_request())
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_SOURCE_IDENTITY_MISMATCH"
    );
    service.shutdown().unwrap();
}

#[test]
fn confirmation_rejects_relative_root_and_prepared_intent_tamper_or_mismatch() {
    let database = TempStore::new("confirm-reject");
    freeze_source(&database.0, true, source_item());
    let service = fixture_service(
        &database.0,
        fixture_graph(),
        Arc::new(CountingWorker::default()),
    );
    let expected = service
        .prepare_nisar_subset_intent_v2(&prepare_request())
        .unwrap()
        .intent;
    let confirm = |intent, output_root| NisarConfirmSubsetTaskV2Request {
        source_batch_id: "nisar-source-batch".to_owned(),
        source_item_id: ITEM_ID.to_owned(),
        expected_workbench_revision: 0,
        prepared_intent: intent,
        task_id: "nisar-v2-task".to_owned(),
        output_root,
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-08-29T00:00:00Z".to_owned(),
    };

    assert_eq!(
        service
            .confirm_nisar_subset_task_v2(&confirm(
                expected.clone(),
                PathBuf::from("relative-output")
            ))
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_REQUEST_INVALID"
    );

    let mut tampered = expected.clone();
    tampered.canonical_intent_sha256 = "a".repeat(64);
    assert_eq!(
        service
            .confirm_nisar_subset_task_v2(&confirm(
                tampered,
                std::env::temp_dir().join("nisar-v2-tampered-output")
            ))
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_PREPARED_INTENT_INVALID"
    );

    let mut different_source = source_item();
    different_source
        .asset_access
        .as_mut()
        .unwrap()
        .checksum_value = Some("f".repeat(64));
    let mismatch = provider_prepare_nisar_subset_intent_v2(
        &different_source,
        AsfNisarSubsetPreparedIntentDraftV2 {
            requested_geometry: expected.requested_geometry.canonical_geojson.clone(),
            selected_frequency: expected.selected_frequency.clone(),
            selected_polarizations: expected.selected_polarizations.clone(),
        },
    )
    .unwrap();
    mismatch.validate().unwrap();
    assert_eq!(
        service
            .confirm_nisar_subset_task_v2(&confirm(
                mismatch,
                std::env::temp_dir().join("nisar-v2-mismatch-output")
            ))
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_PREPARED_INTENT_IDENTITY_MISMATCH"
    );
    assert!(TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("nisar-v2-task")
        .unwrap()
        .is_none());
    service.shutdown().unwrap();
}

#[test]
fn request_schema_separates_user_draft_from_exact_prepared_intent() {
    let base = json!({
        "source_batch_id": "nisar-source-batch",
        "source_item_id": ITEM_ID,
        "expected_workbench_revision": 0,
        "subset_draft_geometry": draft_geometry(),
        "selected_frequency": "A",
        "selected_polarizations": ["HH", "HV"]
    });
    for field in [
        "request_id",
        "source_footprint",
        "source_product_identity",
        "source_asset_identity",
        "source_capability",
        "effective_geometry",
        "prepared_intent",
        "source_dependency_graph",
        "dependency_closure",
        "provenance",
        "transfer_mode",
        "canonical_intent_sha256",
        "canonical_semantic_plan_digest",
    ] {
        let mut value = base.clone();
        value
            .as_object_mut()
            .unwrap()
            .insert(field.to_owned(), json!({}));
        assert!(
            serde_json::from_value::<NisarPrepareSubsetIntentV2Request>(value).is_err(),
            "authority field {field} must be rejected during preparation"
        );
    }

    let intent = provider_prepare_nisar_subset_intent_v2(
        &source_item(),
        AsfNisarSubsetPreparedIntentDraftV2 {
            requested_geometry: draft_geometry(),
            selected_frequency: insar_core::NisarSubsetFrequency::A,
            selected_polarizations: vec!["HH".to_owned(), "HV".to_owned()],
        },
    )
    .unwrap();
    let confirmed = json!({
        "source_batch_id": "nisar-source-batch",
        "source_item_id": ITEM_ID,
        "expected_workbench_revision": 0,
        "prepared_intent": intent,
        "task_id": "nisar-v2-task",
        "output_root": std::env::temp_dir().join("nisar-v2-output"),
        "layout": "direct",
        "concurrency": 2,
        "created_at": "2026-08-29T00:00:00Z"
    });
    serde_json::from_value::<NisarConfirmSubsetTaskV2Request>(confirmed.clone()).unwrap();
    for field in [
        "request_id",
        "subset_draft_geometry",
        "selected_frequency",
        "selected_polarizations",
        "source_dependency_graph",
        "dependency_closure",
        "canonical_semantic_plan_digest",
    ] {
        let mut value = confirmed.clone();
        value
            .as_object_mut()
            .unwrap()
            .insert(field.to_owned(), json!({}));
        assert!(
            serde_json::from_value::<NisarConfirmSubsetTaskV2Request>(value).is_err(),
            "raw/late authority field {field} must be rejected during confirmation"
        );
    }
}
