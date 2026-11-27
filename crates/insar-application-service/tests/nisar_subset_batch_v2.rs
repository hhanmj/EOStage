use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, BhoonidhiAssetSizeRequest,
    NisarAppendSubsetBatchTaskV2Request, NisarAppendSubsetContextV2Request,
    NisarConfirmSubsetBatchItemV2, NisarConfirmSubsetBatchTaskV2Request,
    NisarCreateSubsetSuccessorTaskV2Request, NisarPreflightSubsetAppendV2Request,
    NisarPreflightSubsetBatchV2Request, NisarPrepareSubsetBatchV2Request,
    NisarSubsetBatchExclusionReason, NisarSubsetDependencyEvidenceV2Fixture,
};
use insar_core::{
    canonicalize_nisar_subset_geojson_v2, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
    AssetResolverParameters, AssetRole, CatalogSearchQuery, CatalogSearchResponse, ContractError,
    DownloadItemProgress, DownloadItemResult, DownloadLayout, DownloadOutcome, FrozenAssetAccess,
    FrozenSearchBatch, NisarSubsetDependencyEvidenceAuthorityV2, NisarSubsetDependencyRelationV2,
    NisarSubsetGeometryRoleV2, NisarSubsetSourceDependencyEdgeV2,
    NisarSubsetSourceDependencyGraphV2, NisarSubsetSourceDependencyNodeKindV2,
    NisarSubsetSourceDependencyNodeV2, ProviderResult, SearchWorkbenchState, SnapshotItem,
    StableAssetReference, StoreArchiveTaskRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::catalog::{
    BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROFILE_ID,
    BHOONIDHI_PROVIDER_ID, BHOONIDHI_RESOLVER_POLICY_ID,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::{parse_nisar_subset_prepared_intent_v2_envelope, TaskStore};
use serde_json::{json, Map, Value};

const BATCH_ID: &str = "nisar-source-batch";
const ITEM_1: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF";
const ITEM_2: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000002-ASF";
const ITEM_3: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000003-ASF";

const SBAND_BATCH_ID: &str = "nisar-sband-source-batch";
const SBAND_PRODUCT_1: &str =
    "NISAR_S1_PR_RSLC_029_063_D_079_2500_CRNA_A_20260829T135915_20260829T135945_P00500_M_P_I_001";
const SBAND_PRODUCT_2: &str =
    "NISAR_S1_PR_RSLC_029_053_A_140_2500_CRNA_A_20260831T053450_20260831T053520_P00500_N_P_I_001";

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-nisar-batch-v2-{label}-{}-{nonce}.sqlite3",
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
struct NoopWorker;

impl DownloadWorker for NoopWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root.join("unexpected-batch-execution.h5"),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-09-04T00:00:01Z".to_owned(),
        }
    }
}

struct ClaimingWorker {
    claims: mpsc::Sender<String>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl DownloadWorker for ClaimingWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.claims.send(work.item.item_id.clone()).unwrap();
        let (released, changed) = &*self.release;
        let mut can_finish = released.lock().unwrap();
        while !*can_finish {
            can_finish = changed.wait(can_finish).unwrap();
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id.clone(),
            outcome: DownloadOutcome::Completed,
            destination: work
                .task
                .output_root
                .join(format!("{}.h5", work.item.item_id)),
            bytes_written: 1,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-09-04T00:00:02Z".to_owned(),
        }
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

fn source_item(item_id: &str, product_id: &str, footprint: Map<String, Value>) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: format!("NISAR_L1_PR_RSLC_{product_id}_P05023_N_F_J_001"),
        acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
        footprint_geojson: Some(footprint),
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
            ("quality_status".to_owned(), json!("Provisional")),
            ("frequency_a_polarization".to_owned(), json!("HH+HV")),
            ("frequency_b_polarization".to_owned(), json!("HH")),
        ]),
        payload: Map::new(),
        asset_access: Some(FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: "asf.nasa.earthdata".to_owned(),
                collection_id: "C2853145197-ASF".to_owned(),
                provider_product_id: product_id.to_owned(),
                provider_product_key: item_id.to_owned(),
                canonical_product_key: None,
                asset_key: "science-rslc-hdf5".to_owned(),
                stable_asset_key: format!("{item_id}::ASSET::science-rslc-hdf5"),
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
            file_name: Some(format!("{product_id}.h5")),
            expected_size_bytes: Some(14_252_244_992),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some("e".repeat(64)),
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: "asf.nisar.earthdata-complete-hdf5.v2".to_owned(),
            collection_profile_id: Some("asf.nisar.l.rslc.provisional.v1".to_owned()),
        }),
    }
}

fn source_items() -> Vec<SnapshotItem> {
    vec![
        source_item(ITEM_1, "G9900000001-ASF", rectangle(0.0, 0.0, 10.0, 10.0)),
        source_item(ITEM_2, "G9900000002-ASF", rectangle(20.0, 20.0, 30.0, 30.0)),
        source_item(ITEM_3, "G9900000003-ASF", rectangle(40.0, 40.0, 50.0, 50.0)),
    ]
}

fn sband_item_id(product_id: &str) -> String {
    format!(
        "PROVIDER::{BHOONIDHI_PROVIDER_ID}::COLLECTION::{BHOONIDHI_NATIVE_COLLECTION_ID}::ITEM::{product_id}"
    )
}

fn sband_source_item(product_id: &str, footprint: Map<String, Value>) -> SnapshotItem {
    let item_id = sband_item_id(product_id);
    let file_name = format!("{product_id}.h5");
    SnapshotItem {
        item_id: item_id.clone(),
        provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
        display_name: product_id.to_owned(),
        acquisition_datetime: Some("2026-08-29T13:59:15Z".to_owned()),
        footprint_geojson: Some(footprint),
        metadata: Map::from_iter([
            (
                "logical_dataset".to_owned(),
                Value::String(BHOONIDHI_LOGICAL_DATASET.to_owned()),
            ),
            (
                "native_collection_id".to_owned(),
                Value::String(BHOONIDHI_NATIVE_COLLECTION_ID.to_owned()),
            ),
            (
                "collection_profile_id".to_owned(),
                Value::String(BHOONIDHI_PROFILE_ID.to_owned()),
            ),
            (
                "processing_level".to_owned(),
                Value::String("RSLC".to_owned()),
            ),
            ("band".to_owned(), Value::String("S".to_owned())),
            (
                "primary_asset_key".to_owned(),
                Value::String("data".to_owned()),
            ),
            (
                "provider_product_id".to_owned(),
                Value::String(product_id.to_owned()),
            ),
            (
                "provider_product_key".to_owned(),
                Value::String(item_id.clone()),
            ),
            (
                "primary_file_name".to_owned(),
                Value::String(file_name.clone()),
            ),
            (
                "properties".to_owned(),
                json!({
                    "ID": product_id,
                    "Online": "Y",
                    "Coverage": "P",
                    "SoftwareVersion": "1.4.00",
                    "Mode": "W",
                    "Polarization": "CP"
                }),
            ),
        ]),
        payload: Map::new(),
        asset_access: Some(FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
                collection_id: BHOONIDHI_NATIVE_COLLECTION_ID.to_owned(),
                provider_product_id: product_id.to_owned(),
                provider_product_key: item_id.clone(),
                canonical_product_key: None,
                asset_key: "data".to_owned(),
                stable_asset_key: format!("{item_id}::ASSET::data"),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
                scheme: AssetAccessScheme::OauthBearerHttps,
                stable_href: Some("https://bhoonidhi-api.nrsc.gov.in/download".to_owned()),
                requires_credentials: true,
                credential_profile: Some("bhoonidhi-default".to_owned()),
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(file_name),
            expected_size_bytes: None,
            checksum_algorithm: None,
            checksum_value: None,
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
            collection_profile_id: Some(BHOONIDHI_PROFILE_ID.to_owned()),
        }),
    }
}

fn freeze_sband_source(path: &Path) -> Vec<String> {
    let item_ids = vec![
        sband_item_id(SBAND_PRODUCT_1),
        sband_item_id(SBAND_PRODUCT_2),
    ];
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: SBAND_BATCH_ID.to_owned(),
        provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
        created_at: "2026-09-21T10:00:00Z".to_owned(),
        aoi_name: "S-band AOI".to_owned(),
        aoi_geojson: Some(rectangle(74.0, 5.0, 79.0, 8.0)),
        query: Map::new(),
        search_stats: None,
        candidates: vec![
            sband_source_item(SBAND_PRODUCT_1, rectangle(74.0, 5.0, 78.0, 7.0)),
            sband_source_item(SBAND_PRODUCT_2, rectangle(75.0, 5.0, 79.0, 8.0)),
        ],
    };
    TaskStore::open(path)
        .unwrap()
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id,
                checked_item_ids: vec![item_ids[0].clone()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    item_ids
}

fn sband_fixture_service(path: &Path) -> ApplicationService {
    ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(path),
        Arc::new(FixtureSearch),
        Arc::new(FixtureSearch),
        Arc::new(NoopWorker),
    )
}

fn freeze_source_items(path: &Path, candidates: Vec<SnapshotItem>) {
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: BATCH_ID.to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        created_at: "2026-09-04T00:00:00Z".to_owned(),
        aoi_name: "Workspace AOI copy".to_owned(),
        aoi_geojson: Some(rectangle(-5.0, -5.0, 35.0, 35.0)),
        query: Map::new(),
        search_stats: None,
        candidates,
    };
    TaskStore::open(path)
        .unwrap()
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id,
                checked_item_ids: vec![ITEM_1.to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
}

fn freeze_source(path: &Path) {
    freeze_source_items(path, source_items());
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

    let science = "/science/LSAR/RSLC/swaths/frequencyA/HH";
    let coordinate = "/science/LSAR/RSLC/metadata/geolocationGrid/frequencyA/coordinateX";
    let dimension = "/science/LSAR/RSLC/swaths/frequencyA/zeroDopplerTime";
    let mask = "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask";
    let identification = "/science/LSAR/identification/granuleId";
    let provenance = "/science/LSAR/identification/productVersion";
    let attribute = "/";
    let other = "/science/LSAR/RSLC/metadata/orbit";
    NisarSubsetSourceDependencyGraphV2 {
        graph_contract: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT.to_owned(),
        schema_version: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
        evidence_authority: NisarSubsetDependencyEvidenceAuthorityV2::ReviewedSourceEvidenceV1,
        evidence_sha256: "c".repeat(64),
        nodes: vec![
            node(Kind::ScienceDataset, science),
            node(Kind::CoordinateDataset, coordinate),
            node(Kind::DimensionScale, dimension),
            node(Kind::ValidityMask, mask),
            node(Kind::IdentificationMetadata, identification),
            node(Kind::ProvenanceMetadata, provenance),
            node(Kind::Attribute, attribute),
            node(Kind::OtherScientificDependency, other),
        ],
        edges: vec![
            edge(science, coordinate, Relation::Coordinate),
            edge(science, dimension, Relation::DimensionScale),
            edge(science, mask, Relation::ValidityMask),
            edge(science, identification, Relation::RequiredMetadata),
            edge(science, provenance, Relation::Provenance),
            edge(science, attribute, Relation::RequiredAttribute),
            edge(science, other, Relation::OtherScientificDependency),
        ],
    }
}

fn fixture_service(path: &Path) -> ApplicationService {
    fixture_service_with_worker(path, Arc::new(NoopWorker))
}

fn fixture_service_with_worker(path: &Path, worker: Arc<dyn DownloadWorker>) -> ApplicationService {
    ApplicationService::with_offline_nisar_subset_v2_fixture(
        ApplicationServiceConfig::business_store(path),
        Arc::new(FixtureSearch),
        Arc::new(FixtureSearch),
        worker,
        [ITEM_1, ITEM_2, ITEM_3]
            .into_iter()
            .map(|source_item_id| NisarSubsetDependencyEvidenceV2Fixture {
                source_item_id: source_item_id.to_owned(),
                graph: fixture_graph(),
            })
            .collect(),
    )
}

fn prepare_request(
    source_item_ids: Vec<String>,
    geometry: Map<String, Value>,
) -> NisarPrepareSubsetBatchV2Request {
    NisarPrepareSubsetBatchV2Request {
        source_batch_id: BATCH_ID.to_owned(),
        source_item_ids,
        expected_workbench_revision: 0,
        subset_draft_geometry: geometry,
        selected_frequency: "A".to_owned(),
        selected_polarizations: vec!["HH".to_owned()],
    }
}

fn preflight_request(
    source_item_ids: Vec<String>,
    geometry: Map<String, Value>,
) -> NisarPreflightSubsetBatchV2Request {
    NisarPreflightSubsetBatchV2Request {
        source_batch_id: BATCH_ID.to_owned(),
        source_item_ids,
        expected_workbench_revision: 0,
        subset_draft_geometry: geometry,
    }
}

fn create_paused_subset_task(service: &ApplicationService, task_id: &str) {
    let prepared = service
        .prepare_nisar_subset_batch_v2(&prepare_request(
            vec![ITEM_1.to_owned()],
            rectangle(-1.0, -1.0, 25.0, 25.0),
        ))
        .unwrap();
    service
        .confirm_nisar_subset_batch_task_v2(&NisarConfirmSubsetBatchTaskV2Request {
            source_batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: prepared.workbench_revision,
            items: prepared
                .intents
                .into_iter()
                .map(|value| NisarConfirmSubsetBatchItemV2 {
                    source_item_id: value.source_item_id,
                    prepared_intent: value.intent,
                })
                .collect(),
            task_id: task_id.to_owned(),
            output_root: std::env::temp_dir().join(format!("{task_id}-output")),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-05T00:00:00Z".to_owned(),
        })
        .unwrap();
}

#[test]
fn bhoonidhi_sband_asset_size_fails_closed_before_fixture_auth_or_network() {
    let database = TempStore::new("sband-asset-size-fail-closed");
    let item_ids = freeze_sband_source(&database.0);
    let service = sband_fixture_service(&database.0);

    let invalid = service
        .bhoonidhi_asset_size(BhoonidhiAssetSizeRequest {
            request_id: "".to_owned(),
            batch_id: SBAND_BATCH_ID.to_owned(),
            item_id: item_ids[0].clone(),
        })
        .unwrap_err();
    assert_eq!(invalid.code(), "V3_APPLICATION_INVALID_REQUEST");

    let nonproduction = service
        .bhoonidhi_asset_size(BhoonidhiAssetSizeRequest {
            request_id: "sband-size-fixture-route".to_owned(),
            batch_id: SBAND_BATCH_ID.to_owned(),
            item_id: item_ids[0].clone(),
        })
        .unwrap_err();
    assert_eq!(
        nonproduction.code(),
        "BHOONIDHI_ASSET_FROZEN_EVIDENCE_INVALID",
        "fixture catalogue routes must never reach credential or Range probing"
    );

    let snapshot = service
        .snapshot(true)
        .expect("size probe rejection leaves Store readable");
    assert_eq!(
        snapshot
            .restore
            .workbench_batch
            .as_ref()
            .map(|batch| batch.batch_id.as_str()),
        Some(SBAND_BATCH_ID)
    );
    assert!(snapshot.restore.tasks.is_empty());
    service.shutdown().unwrap();
}

#[test]
fn bhoonidhi_sband_batch_confirm_and_append_preserve_ssar_authority() {
    let database = TempStore::new("sband-confirm-append");
    let item_ids = freeze_sband_source(&database.0);
    assert!(
        item_ids[0].len() > 128,
        "real S-band frozen item ID exercises the NISAR-specific bound"
    );
    let service = sband_fixture_service(&database.0);
    let requested_geometry = rectangle(75.0, 5.5, 76.0, 6.5);

    let prepared = service
        .prepare_nisar_subset_batch_v2(&NisarPrepareSubsetBatchV2Request {
            source_batch_id: SBAND_BATCH_ID.to_owned(),
            source_item_ids: vec![item_ids[0].clone()],
            expected_workbench_revision: 0,
            subset_draft_geometry: requested_geometry,
            selected_frequency: "A".to_owned(),
            selected_polarizations: vec!["RH".to_owned()],
        })
        .expect("reviewed Bhoonidhi S-band batch prepares");
    assert_eq!(prepared.intents.len(), 1);
    let first_intent = prepared.intents[0].intent.clone();
    assert_eq!(
        first_intent.source_product_identity.provider_id,
        BHOONIDHI_PROVIDER_ID
    );
    assert_eq!(
        first_intent.source_product_identity.collection_id,
        BHOONIDHI_NATIVE_COLLECTION_ID
    );
    assert_eq!(
        first_intent.source_product_identity.collection_profile_id,
        BHOONIDHI_PROFILE_ID
    );
    assert_eq!(
        first_intent
            .source_product_identity
            .product_specification_version,
        "1.2.1"
    );
    assert_eq!(first_intent.selected_polarizations, ["RH"]);

    service
        .confirm_nisar_subset_batch_task_v2(&NisarConfirmSubsetBatchTaskV2Request {
            source_batch_id: SBAND_BATCH_ID.to_owned(),
            expected_workbench_revision: prepared.workbench_revision,
            items: vec![NisarConfirmSubsetBatchItemV2 {
                source_item_id: item_ids[0].clone(),
                prepared_intent: first_intent.clone(),
            }],
            task_id: "sband-subset-task".to_owned(),
            output_root: std::env::temp_dir().join("sband-subset-task-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-09-21T10:00:01Z".to_owned(),
        })
        .expect("reviewed Bhoonidhi S-band subset confirms");

    let initial = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("sband-subset-task")
        .unwrap()
        .unwrap();
    assert_eq!(initial.snapshot.provider_id, BHOONIDHI_PROVIDER_ID);
    assert_eq!(initial.runtime.status, TaskLifecycleStatus::Paused);
    assert!(initial.runtime.task_paused);
    assert_eq!(initial.runtime.paused_item_ids, [item_ids[0].clone()]);
    let frozen_first =
        parse_nisar_subset_prepared_intent_v2_envelope(&initial.snapshot.candidates[0].metadata)
            .unwrap()
            .unwrap()
            .intent()
            .unwrap();
    assert_eq!(frozen_first, first_intent);

    let context = service
        .read_nisar_subset_append_context_v2(&NisarAppendSubsetContextV2Request {
            task_id: "sband-subset-task".to_owned(),
        })
        .expect("S-band append context reopens");
    assert_eq!(context.selected_frequency, "A");
    assert_eq!(context.selected_polarizations, ["RH"]);

    let preflight = service
        .preflight_nisar_subset_append_v2(&NisarPreflightSubsetAppendV2Request {
            task_id: "sband-subset-task".to_owned(),
            expected_task_revision: context.task_revision,
            source_item_ids: vec![item_ids[1].clone()],
        })
        .expect("second S-band scene is eligible for append");
    assert_eq!(preflight.eligible_items.len(), 1);
    assert!(preflight.excluded_items.is_empty());

    service
        .append_nisar_subset_batch_task_v2(&NisarAppendSubsetBatchTaskV2Request {
            task_id: "sband-subset-task".to_owned(),
            expected_task_revision: context.task_revision,
            source_item_ids: vec![item_ids[1].clone()],
            event_id: "sband-subset-task:append-2".to_owned(),
            occurred_at: "2026-09-21T10:00:02Z".to_owned(),
        })
        .expect("second S-band scene appends with SSAR intent");

    let appended = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("sband-subset-task")
        .unwrap()
        .unwrap();
    assert_eq!(appended.snapshot.provider_id, BHOONIDHI_PROVIDER_ID);
    assert_eq!(appended.snapshot.candidates.len(), 2);
    let appended_intent = appended
        .snapshot
        .candidates
        .iter()
        .find(|item| item.item_id == item_ids[1])
        .and_then(|item| {
            parse_nisar_subset_prepared_intent_v2_envelope(&item.metadata)
                .ok()
                .flatten()
        })
        .and_then(|envelope| envelope.intent().ok())
        .expect("appended S-band scene keeps prepared intent");
    assert_eq!(
        appended_intent.source_product_identity.provider_id,
        BHOONIDHI_PROVIDER_ID
    );
    assert_eq!(
        appended_intent
            .source_product_identity
            .collection_profile_id,
        BHOONIDHI_PROFILE_ID
    );
    assert_eq!(
        appended_intent.selected_frequency,
        first_intent.selected_frequency
    );
    assert_eq!(
        appended_intent.selected_polarizations,
        first_intent.selected_polarizations
    );
    assert_eq!(appended.runtime.status, TaskLifecycleStatus::Paused);
    assert!(appended.runtime.task_paused);
    // The real product branch also supports an archived two-scene S-band
    // source. Fresh recreation is not checkpoint inheritance.
    TaskStore::open(&database.0)
        .unwrap()
        .update_task(&StoreUpdateTaskRequest {
            task_id: "sband-subset-task".to_owned(),
            expected_revision: appended.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: Vec::new(),
            },
            event_id: "sband-subset-task:cancel-for-recreation".to_owned(),
            occurred_at: "2026-09-21T10:00:03Z".to_owned(),
        })
        .unwrap();
    let cancelled = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("sband-subset-task")
        .unwrap()
        .unwrap();
    TaskStore::open(&database.0)
        .unwrap()
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "sband-subset-task".to_owned(),
            expected_revision: cancelled.runtime.revision,
            archived_at: "2026-09-21T10:00:04Z".to_owned(),
        })
        .unwrap();
    let original = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("sband-subset-task")
        .unwrap()
        .unwrap();
    let fresh_context = service
        .read_nisar_subset_append_context_v2(&NisarAppendSubsetContextV2Request {
            task_id: "sband-subset-task".to_owned(),
        })
        .unwrap();
    assert_eq!(fresh_context.task_status, TaskLifecycleStatus::Cancelled);
    assert_eq!(fresh_context.original_item_ids.len(), 2);
    let replacement_id = "sband-recreated-task";
    service
        .create_nisar_subset_successor_task_v2(&NisarCreateSubsetSuccessorTaskV2Request {
            source_task_id: "sband-subset-task".to_owned(),
            expected_task_revision: fresh_context.task_revision,
            source_item_ids: fresh_context.original_item_ids,
            new_task_id: replacement_id.to_owned(),
            output_root: appended.snapshot.output_root.join(replacement_id),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-09-21T10:00:05Z".to_owned(),
            recreate_original_items: true,
        })
        .unwrap();
    let rebuilt = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record(replacement_id)
        .unwrap()
        .unwrap();
    assert_eq!(rebuilt.snapshot.provider_id, BHOONIDHI_PROVIDER_ID);
    assert_eq!(rebuilt.snapshot.candidates.len(), 2);
    assert!(rebuilt.runtime.task_paused);
    assert_eq!(rebuilt.runtime.paused_item_ids.len(), 2);
    assert_eq!(
        TaskStore::open(&database.0)
            .unwrap()
            .load_task_record("sband-subset-task")
            .unwrap()
            .unwrap(),
        original
    );
    service.shutdown().unwrap();
}

#[test]
fn batch_prepare_preserves_requested_order_allows_unchecked_items_and_rejects_cross_batch_items() {
    let database = TempStore::new("prepare-order");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);
    let request = prepare_request(
        vec![ITEM_2.to_owned(), ITEM_1.to_owned()],
        rectangle(-1.0, -1.0, 25.0, 25.0),
    );

    let prepared = service.prepare_nisar_subset_batch_v2(&request).unwrap();
    assert_eq!(
        prepared
            .intents
            .iter()
            .map(|value| value.source_item_id.as_str())
            .collect::<Vec<_>>(),
        vec![ITEM_2, ITEM_1]
    );
    assert!(prepared.excluded_items.is_empty());

    let mut cross_batch = request;
    cross_batch.source_batch_id = "different-frozen-batch".to_owned();
    assert_eq!(
        service
            .prepare_nisar_subset_batch_v2(&cross_batch)
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_SOURCE_INVALID"
    );
}

#[test]
fn batch_preflight_excludes_no_overlap_before_common_capability_intersection() {
    let database = TempStore::new("preflight-capability-after-overlap");
    let mut candidates = source_items();
    candidates[1].metadata.remove("frequency_a_polarization");
    candidates[1]
        .metadata
        .insert("frequency_b_polarization".to_owned(), json!("HH"));
    freeze_source_items(&database.0, candidates);
    let service = fixture_service(&database.0);
    let geometry = rectangle(-2.0, -2.0, 2.0, 2.0);

    let preflight = service
        .preflight_nisar_subset_batch_v2(&preflight_request(
            vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
            geometry.clone(),
        ))
        .unwrap();

    assert_eq!(preflight.eligible_items.len(), 1);
    assert_eq!(preflight.eligible_items[0].source_item_id, ITEM_1);
    assert_eq!(preflight.excluded_items.len(), 1);
    assert_eq!(preflight.excluded_items[0].source_item_id, ITEM_2);
    assert_eq!(
        preflight.excluded_items[0].reason,
        NisarSubsetBatchExclusionReason::NoOverlap
    );
    assert_eq!(preflight.common_selections.len(), 2);
    assert_eq!(preflight.common_selections[0].frequency, "A");
    assert_eq!(preflight.common_selections[0].polarizations, ["HH", "HV"]);

    let prepared = service
        .prepare_nisar_subset_batch_v2(&prepare_request(
            vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
            geometry,
        ))
        .unwrap();
    assert_eq!(prepared.intents.len(), 1);
    assert_eq!(prepared.intents[0].source_item_id, ITEM_1);
    assert_eq!(prepared.excluded_items[0].source_item_id, ITEM_2);
}

#[test]
fn batch_preflight_fails_closed_when_every_item_has_no_overlap() {
    let database = TempStore::new("preflight-all-excluded");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);

    assert_eq!(
        service
            .preflight_nisar_subset_batch_v2(&preflight_request(
                vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
                rectangle(40.0, 40.0, 41.0, 41.0),
            ))
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_EFFECTIVE_GEOMETRY_EMPTY"
    );
}

#[test]
fn batch_prepare_uses_exact_core_intersection_and_excludes_no_overlap_items() {
    let database = TempStore::new("prepare-overlap");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);

    let prepared = service
        .prepare_nisar_subset_batch_v2(&prepare_request(
            vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
            rectangle(-2.0, -2.0, 2.0, 2.0),
        ))
        .unwrap();

    assert_eq!(prepared.intents.len(), 1);
    assert_eq!(prepared.intents[0].source_item_id, ITEM_1);
    assert_eq!(prepared.excluded_items.len(), 1);
    assert_eq!(prepared.excluded_items[0].source_item_id, ITEM_2);
    assert_eq!(
        prepared.excluded_items[0].reason,
        NisarSubsetBatchExclusionReason::NoOverlap
    );
    assert_eq!(
        prepared.intents[0].intent.effective_geometry,
        canonicalize_nisar_subset_geojson_v2(
            &rectangle(0.0, 0.0, 2.0, 2.0),
            NisarSubsetGeometryRoleV2::Effective,
        )
        .unwrap()
    );
}

#[test]
fn batch_prepare_rejects_duplicate_missing_excess_and_stale_selection() {
    let database = TempStore::new("prepare-invalid");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);
    let geometry = rectangle(-1.0, -1.0, 25.0, 25.0);

    for ids in [
        vec![ITEM_1.to_owned(), ITEM_1.to_owned()],
        vec!["missing-item".to_owned()],
        (0..101).map(|index| format!("item-{index}")).collect(),
    ] {
        assert!(service
            .prepare_nisar_subset_batch_v2(&prepare_request(ids, geometry.clone()))
            .is_err());
    }

    let mut store = TaskStore::open(&database.0).unwrap();
    let mut workbench = store.load_workbench().unwrap().unwrap().state;
    workbench.revision = 1;
    store
        .update_workbench(&workbench, 0, "2026-09-04T00:00:01Z")
        .unwrap();
    drop(store);
    assert_eq!(
        service
            .prepare_nisar_subset_batch_v2(&prepare_request(vec![ITEM_1.to_owned()], geometry,))
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_SEARCH_BATCH_STALE"
    );
}

#[test]
fn batch_prepare_fails_closed_when_every_item_has_no_overlap() {
    let database = TempStore::new("prepare-all-excluded");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);

    assert_eq!(
        service
            .prepare_nisar_subset_batch_v2(&prepare_request(
                vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
                rectangle(40.0, 40.0, 41.0, 41.0),
            ))
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_V2_EFFECTIVE_GEOMETRY_EMPTY"
    );
}

#[test]
fn batch_confirm_creates_one_paused_task_with_one_explicit_item_per_intent() {
    let database = TempStore::new("confirm");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);
    let prepared = service
        .prepare_nisar_subset_batch_v2(&prepare_request(
            vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
            rectangle(-1.0, -1.0, 25.0, 25.0),
        ))
        .unwrap();

    service
        .confirm_nisar_subset_batch_task_v2(&NisarConfirmSubsetBatchTaskV2Request {
            source_batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: prepared.workbench_revision,
            items: prepared
                .intents
                .into_iter()
                .map(|value| NisarConfirmSubsetBatchItemV2 {
                    source_item_id: value.source_item_id,
                    prepared_intent: value.intent,
                })
                .collect(),
            task_id: "subset-batch-1".to_owned(),
            output_root: std::env::temp_dir().join("subset-batch-1-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-04T00:00:00Z".to_owned(),
        })
        .unwrap();

    let task = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("subset-batch-1")
        .unwrap()
        .unwrap();
    assert_eq!(task.snapshot.candidates.len(), 2);
    assert_eq!(
        task.snapshot
            .candidates
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        vec![ITEM_1, ITEM_2]
    );
    assert!(task.snapshot.candidates.iter().all(|item| {
        item.metadata.get("delivery_kind").and_then(Value::as_str)
            == Some("remote_derived_subset_v2")
    }));
    assert_eq!(task.runtime.status, TaskLifecycleStatus::Paused);
    assert!(task.runtime.task_paused);
    assert!(task.runtime.queued_item_ids.is_empty());
    assert_eq!(task.runtime.paused_item_ids, [ITEM_1, ITEM_2]);
}

#[test]
fn newly_confirmed_subset_start_resolves_the_latest_store_revision_and_returns_ready_state() {
    let database = TempStore::new("authoritative-start");
    freeze_source(&database.0);
    let (claims, claimed) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let service = fixture_service_with_worker(
        &database.0,
        Arc::new(ClaimingWorker {
            claims,
            release: Arc::clone(&release),
        }),
    );
    let prepared = service
        .prepare_nisar_subset_batch_v2(&prepare_request(
            vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
            rectangle(-1.0, -1.0, 25.0, 25.0),
        ))
        .unwrap();
    let request = NisarConfirmSubsetBatchTaskV2Request {
        source_batch_id: BATCH_ID.to_owned(),
        expected_workbench_revision: prepared.workbench_revision,
        items: prepared
            .intents
            .into_iter()
            .map(|value| NisarConfirmSubsetBatchItemV2 {
                source_item_id: value.source_item_id,
                prepared_intent: value.intent,
            })
            .collect(),
        task_id: "subset-authoritative-start".to_owned(),
        output_root: std::env::temp_dir().join("subset-authoritative-start-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-04T00:00:00Z".to_owned(),
    };
    service
        .confirm_nisar_subset_batch_task_v2(&request)
        .unwrap();

    let confirmed = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record(&request.task_id)
        .unwrap()
        .unwrap();
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: request.task_id.clone(),
            expected_revision: confirmed.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: Vec::new(),
            },
            event_id: "subset-authoritative-start:store-advanced".to_owned(),
            occurred_at: "2026-09-04T00:00:01Z".to_owned(),
        })
        .unwrap();

    let started = service
        .start_newly_confirmed_task(&request.task_id)
        .unwrap();
    let claimed_item_ids = (0..2)
        .map(|_| claimed.recv_timeout(Duration::from_secs(5)).unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        claimed_item_ids,
        BTreeSet::from([ITEM_1.to_owned(), ITEM_2.to_owned()])
    );
    let summary = started
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == request.task_id)
        .unwrap();
    assert!(matches!(
        summary.status,
        TaskLifecycleStatus::Queued | TaskLifecycleStatus::Running
    ));
    let stored = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record(&request.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.runtime.status, TaskLifecycleStatus::Running);
    assert!(!stored.runtime.task_paused);
    assert!(stored.runtime.paused_item_ids.is_empty());
    assert!(stored.runtime.queued_item_ids.is_empty());
    assert_eq!(
        stored
            .runtime
            .active_item_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([ITEM_1.to_owned(), ITEM_2.to_owned()])
    );
    let non_paused_count = stored.runtime.queued_item_ids.len()
        + stored.runtime.active_item_ids.len()
        + stored.runtime.completed_item_ids.len()
        + stored.runtime.failed_item_ids.len()
        + stored.runtime.skipped_item_ids.len()
        + stored.runtime.cancelled_item_ids.len();
    assert_eq!(non_paused_count, stored.snapshot.candidates.len());

    let (released, changed) = &*release;
    *released.lock().unwrap() = true;
    changed.notify_all();
    service.shutdown().unwrap();
}

#[test]
fn append_context_and_preflight_use_frozen_task_authority_for_three_candidates() {
    let database = TempStore::new("append-context-preflight");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);
    create_paused_subset_task(&service, "append-context");

    let context = service
        .read_nisar_subset_append_context_v2(&NisarAppendSubsetContextV2Request {
            task_id: "append-context".to_owned(),
        })
        .unwrap();
    assert_eq!(context.task_id, "append-context");
    assert_eq!(context.source_batch_id, BATCH_ID);
    assert_eq!(context.task_revision, 0);
    assert_eq!(context.task_status, TaskLifecycleStatus::Paused);
    assert_eq!(context.selected_frequency, "A");
    assert_eq!(context.selected_polarizations, ["HH"]);
    assert_eq!(
        context.output_root,
        std::env::temp_dir().join("append-context-output")
    );
    assert_eq!(context.concurrency, 2);
    assert_eq!(
        context.frozen_aoi_geojson,
        canonicalize_nisar_subset_geojson_v2(
            &rectangle(-1.0, -1.0, 25.0, 25.0),
            NisarSubsetGeometryRoleV2::Requested,
        )
        .unwrap()
        .canonical_geojson
    );

    let preflight = service
        .preflight_nisar_subset_append_v2(&NisarPreflightSubsetAppendV2Request {
            task_id: context.task_id.clone(),
            expected_task_revision: context.task_revision,
            source_item_ids: vec![ITEM_1.to_owned(), ITEM_2.to_owned(), ITEM_3.to_owned()],
        })
        .unwrap();
    assert_eq!(preflight.task_id, context.task_id);
    assert_eq!(preflight.source_batch_id, BATCH_ID);
    assert_eq!(preflight.task_revision, context.task_revision);
    assert_eq!(
        preflight
            .eligible_items
            .iter()
            .map(|item| item.source_item_id.as_str())
            .collect::<Vec<_>>(),
        vec![ITEM_1, ITEM_2]
    );
    assert_eq!(preflight.excluded_items.len(), 1);
    assert_eq!(preflight.excluded_items[0].source_item_id, ITEM_3);
    assert_eq!(
        preflight.excluded_items[0].reason,
        NisarSubsetBatchExclusionReason::NoOverlap
    );
    assert_eq!(preflight.common_selections.len(), 2);

    service
        .append_nisar_subset_batch_task_v2(&NisarAppendSubsetBatchTaskV2Request {
            task_id: context.task_id,
            expected_task_revision: 0,
            source_item_ids: vec![ITEM_2.to_owned()],
            event_id: "append-context:item-2".to_owned(),
            occurred_at: "2026-09-05T00:00:01Z".to_owned(),
        })
        .unwrap();
    let stored = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("append-context")
        .unwrap()
        .unwrap();
    assert_eq!(stored.snapshot.candidates.len(), 2);
    assert!(stored
        .snapshot
        .candidates
        .iter()
        .any(|item| item.item_id == ITEM_2));
}

#[test]
fn append_preflight_reports_frequency_and_polarization_exclusions() {
    let database = TempStore::new("append-science-exclusions");
    let mut candidates = source_items();
    candidates[1].metadata.remove("frequency_a_polarization");
    candidates[2]
        .metadata
        .insert("frequency_a_polarization".to_owned(), json!("HH"));
    candidates[2].footprint_geojson = Some(rectangle(5.0, 5.0, 15.0, 15.0));
    freeze_source_items(&database.0, candidates);
    let service = fixture_service(&database.0);
    let prepared = service
        .prepare_nisar_subset_batch_v2(&NisarPrepareSubsetBatchV2Request {
            source_batch_id: BATCH_ID.to_owned(),
            source_item_ids: vec![ITEM_1.to_owned()],
            expected_workbench_revision: 0,
            subset_draft_geometry: rectangle(-1.0, -1.0, 25.0, 25.0),
            selected_frequency: "A".to_owned(),
            selected_polarizations: vec!["HV".to_owned()],
        })
        .unwrap();
    service
        .confirm_nisar_subset_batch_task_v2(&NisarConfirmSubsetBatchTaskV2Request {
            source_batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: prepared.workbench_revision,
            items: prepared
                .intents
                .into_iter()
                .map(|value| NisarConfirmSubsetBatchItemV2 {
                    source_item_id: value.source_item_id,
                    prepared_intent: value.intent,
                })
                .collect(),
            task_id: "append-science".to_owned(),
            output_root: std::env::temp_dir().join("append-science-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-05T00:00:00Z".to_owned(),
        })
        .unwrap();

    let preflight = service
        .preflight_nisar_subset_append_v2(&NisarPreflightSubsetAppendV2Request {
            task_id: "append-science".to_owned(),
            expected_task_revision: 0,
            source_item_ids: vec![ITEM_2.to_owned(), ITEM_3.to_owned()],
        })
        .unwrap();
    assert!(preflight.eligible_items.is_empty());
    assert!(preflight.common_selections.is_empty());
    assert_eq!(preflight.excluded_items.len(), 2);
    assert_eq!(
        preflight
            .excluded_items
            .iter()
            .find(|item| item.source_item_id == ITEM_2)
            .unwrap()
            .reason,
        NisarSubsetBatchExclusionReason::FrequencyIncompatible
    );
    assert_eq!(
        preflight
            .excluded_items
            .iter()
            .find(|item| item.source_item_id == ITEM_3)
            .unwrap()
            .reason,
        NisarSubsetBatchExclusionReason::PolarizationIncompatible
    );
}

#[test]
fn append_preflight_rejects_a_mutated_task_revision_until_context_is_refreshed() {
    let database = TempStore::new("append-context-revision");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);
    create_paused_subset_task(&service, "append-revision");
    let initial = service
        .read_nisar_subset_append_context_v2(&NisarAppendSubsetContextV2Request {
            task_id: "append-revision".to_owned(),
        })
        .unwrap();

    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: initial.task_id.clone(),
            expected_revision: initial.task_revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: Vec::new(),
            },
            event_id: "append-revision:advance".to_owned(),
            occurred_at: "2026-09-05T00:00:01Z".to_owned(),
        })
        .unwrap();

    let stale = service
        .preflight_nisar_subset_append_v2(&NisarPreflightSubsetAppendV2Request {
            task_id: initial.task_id.clone(),
            expected_task_revision: initial.task_revision,
            source_item_ids: vec![ITEM_2.to_owned()],
        })
        .unwrap_err();
    assert_eq!(stale.code(), "NISAR_SUBSET_V2_SEARCH_BATCH_STALE");

    let refreshed = service
        .read_nisar_subset_append_context_v2(&NisarAppendSubsetContextV2Request {
            task_id: initial.task_id.clone(),
        })
        .unwrap();
    assert_eq!(refreshed.task_revision, initial.task_revision + 1);
    let preflight = service
        .preflight_nisar_subset_append_v2(&NisarPreflightSubsetAppendV2Request {
            task_id: refreshed.task_id,
            expected_task_revision: refreshed.task_revision,
            source_item_ids: vec![ITEM_2.to_owned()],
        })
        .unwrap();
    assert_eq!(preflight.eligible_items.len(), 1);
    assert!(preflight.excluded_items.is_empty());
}

#[test]
fn terminal_subset_successor_is_new_paused_task_and_leaves_original_unchanged() {
    let database = TempStore::new("append-successor");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);
    create_paused_subset_task(&service, "append-terminal");
    let before = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("append-terminal")
        .unwrap()
        .unwrap();
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: "append-terminal".to_owned(),
            expected_revision: before.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec![ITEM_1.to_owned()],
            },
            event_id: "append-terminal:completed".to_owned(),
            occurred_at: "2026-09-05T00:00:01Z".to_owned(),
        })
        .unwrap();
    let terminal = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("append-terminal")
        .unwrap()
        .unwrap();
    assert_eq!(terminal.runtime.status, TaskLifecycleStatus::Completed);

    let request = NisarCreateSubsetSuccessorTaskV2Request {
        source_task_id: "append-terminal".to_owned(),
        expected_task_revision: terminal.runtime.revision,
        source_item_ids: vec![ITEM_2.to_owned()],
        new_task_id: "append-successor".to_owned(),
        output_root: std::env::temp_dir().join("append-successor-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-05T00:00:02Z".to_owned(),
        recreate_original_items: false,
    };
    service
        .create_nisar_subset_successor_task_v2(&request)
        .unwrap();
    let successor = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record(&request.new_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(successor.snapshot.task_id, request.new_task_id);
    assert_eq!(successor.snapshot.batch_id, BATCH_ID);
    assert_eq!(successor.snapshot.candidates.len(), 1);
    assert_eq!(successor.snapshot.candidates[0].item_id, ITEM_2);
    assert_eq!(successor.runtime.status, TaskLifecycleStatus::Paused);
    assert!(successor.runtime.task_paused);
    assert_eq!(successor.runtime.paused_item_ids, [ITEM_2]);
    assert_eq!(successor.snapshot.output_root, request.output_root);
    assert_eq!(successor.runtime.concurrency, request.concurrency);

    let original_after_create = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("append-terminal")
        .unwrap()
        .unwrap();
    assert_eq!(original_after_create.runtime, terminal.runtime);
    assert_eq!(original_after_create.snapshot, terminal.snapshot);

    // The stable target ID makes a repeated create safe before the UI starts it.
    service
        .create_nisar_subset_successor_task_v2(&request)
        .unwrap();
    let successor_after_retry = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record(&request.new_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        successor_after_retry.runtime.revision,
        successor.runtime.revision
    );
    assert_eq!(successor_after_retry.snapshot, successor.snapshot);

    service
        .start_newly_confirmed_task(&request.new_task_id)
        .unwrap();
    let original_after_start = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("append-terminal")
        .unwrap()
        .unwrap();
    assert_eq!(original_after_start.runtime, terminal.runtime);
    assert_eq!(original_after_start.snapshot, terminal.snapshot);
}

#[test]
fn terminal_two_scene_task_can_recreate_all_original_scenes_without_inheriting_old_checkpoint() {
    let database = TempStore::new("recreate-original-two-scene");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);
    let prepared = service
        .prepare_nisar_subset_batch_v2(&prepare_request(
            vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
            rectangle(-1.0, -1.0, 25.0, 25.0),
        ))
        .unwrap();
    let original_root = std::env::temp_dir().join("recreate-original-two-scene");
    service
        .confirm_nisar_subset_batch_task_v2(&NisarConfirmSubsetBatchTaskV2Request {
            source_batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: prepared.workbench_revision,
            items: prepared
                .intents
                .into_iter()
                .map(|intent| NisarConfirmSubsetBatchItemV2 {
                    source_item_id: intent.source_item_id,
                    prepared_intent: intent.intent,
                })
                .collect(),
            task_id: "recreate-source".to_owned(),
            output_root: original_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-05T00:00:00Z".to_owned(),
        })
        .unwrap();
    let active = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("recreate-source")
        .unwrap()
        .unwrap();
    TaskStore::open(&database.0)
        .unwrap()
        .update_task(&StoreUpdateTaskRequest {
            task_id: "recreate-source".to_owned(),
            expected_revision: active.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: Vec::new(),
            },
            event_id: "recreate-original-cancel".to_owned(),
            occurred_at: "2026-09-05T00:00:01Z".to_owned(),
        })
        .unwrap();
    let cancelled = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("recreate-source")
        .unwrap()
        .unwrap();
    TaskStore::open(&database.0)
        .unwrap()
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "recreate-source".to_owned(),
            expected_revision: cancelled.runtime.revision,
            archived_at: "2026-09-05T00:00:01Z".to_owned(),
        })
        .unwrap();
    let original = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("recreate-source")
        .unwrap()
        .unwrap();
    assert_eq!(original.runtime.status, TaskLifecycleStatus::Cancelled);
    let read = service
        .read_nisar_subset_append_context_v2(&NisarAppendSubsetContextV2Request {
            task_id: "recreate-source".to_owned(),
        })
        .unwrap();
    assert_eq!(read.original_item_ids.len(), 2);
    let mut request = NisarCreateSubsetSuccessorTaskV2Request {
        source_task_id: "recreate-source".to_owned(),
        expected_task_revision: original.runtime.revision,
        source_item_ids: read.original_item_ids,
        new_task_id: "recreated-two-scene".to_owned(),
        output_root: original_root.join("recreated-two-scene"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-05T00:00:02Z".to_owned(),
        recreate_original_items: true,
    };
    request.recreate_original_items = false;
    assert!(service
        .create_nisar_subset_successor_task_v2(&request)
        .is_err());
    request.recreate_original_items = true;
    request.source_item_ids.pop();
    assert!(service
        .create_nisar_subset_successor_task_v2(&request)
        .is_err());
    request.source_item_ids.push(ITEM_2.to_owned());
    request.output_root = original_root.clone();
    assert!(service
        .create_nisar_subset_successor_task_v2(&request)
        .is_err());
    request.output_root = original_root.join(&request.new_task_id);
    service
        .create_nisar_subset_successor_task_v2(&request)
        .unwrap();
    let successor = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record(&request.new_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(successor.runtime.status, TaskLifecycleStatus::Paused);
    assert!(successor.runtime.task_paused);
    assert_eq!(successor.runtime.paused_item_ids.len(), 2);
    assert_eq!(successor.snapshot.candidates.len(), 2);
    assert_eq!(successor.snapshot.output_root, request.output_root);
    assert!(successor
        .snapshot
        .candidates
        .iter()
        .all(|item| { !item.metadata.contains_key("nisar_archived_recovery_v2") }));
    assert_eq!(
        TaskStore::open(&database.0)
            .unwrap()
            .load_task_record("recreate-source")
            .unwrap()
            .unwrap(),
        original
    );
    service
        .create_nisar_subset_successor_task_v2(&request)
        .unwrap();
    assert_eq!(
        TaskStore::open(&database.0)
            .unwrap()
            .load_task_record(&request.new_task_id)
            .unwrap()
            .unwrap(),
        successor
    );
}

#[test]
fn batch_confirm_rejects_a_mismatched_later_intent_without_creating_any_task() {
    let database = TempStore::new("confirm-atomic-failure");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);
    let prepared = service
        .prepare_nisar_subset_batch_v2(&prepare_request(
            vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
            rectangle(-1.0, -1.0, 25.0, 25.0),
        ))
        .unwrap();
    let mut items = prepared
        .intents
        .into_iter()
        .map(|value| NisarConfirmSubsetBatchItemV2 {
            source_item_id: value.source_item_id,
            prepared_intent: value.intent,
        })
        .collect::<Vec<_>>();
    items[1].prepared_intent = items[0].prepared_intent.clone();

    let error = service
        .confirm_nisar_subset_batch_task_v2(&NisarConfirmSubsetBatchTaskV2Request {
            source_batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: prepared.workbench_revision,
            items,
            task_id: "subset-batch-atomic-failure".to_owned(),
            output_root: std::env::temp_dir().join("subset-batch-atomic-failure-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-04T00:00:00Z".to_owned(),
        })
        .unwrap_err();

    assert_eq!(
        error.code(),
        "NISAR_SUBSET_V2_PREPARED_INTENT_IDENTITY_MISMATCH"
    );
    assert!(TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("subset-batch-atomic-failure")
        .unwrap()
        .is_none());
}

#[test]
fn subset_append_rebuilds_one_prepared_intent_per_new_source_scene() {
    let database = TempStore::new("append-subset");
    freeze_source(&database.0);
    let service = fixture_service(&database.0);
    let geometry = rectangle(-1.0, -1.0, 25.0, 25.0);
    let prepared = service
        .prepare_nisar_subset_batch_v2(&prepare_request(vec![ITEM_1.to_owned()], geometry))
        .unwrap();
    service
        .confirm_nisar_subset_batch_task_v2(&NisarConfirmSubsetBatchTaskV2Request {
            source_batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: prepared.workbench_revision,
            items: prepared
                .intents
                .into_iter()
                .map(|value| NisarConfirmSubsetBatchItemV2 {
                    source_item_id: value.source_item_id,
                    prepared_intent: value.intent,
                })
                .collect(),
            task_id: "subset-append".to_owned(),
            output_root: std::env::temp_dir().join("subset-append-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-05T00:00:00Z".to_owned(),
        })
        .unwrap();
    let before = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("subset-append")
        .unwrap()
        .unwrap();
    service
        .append_nisar_subset_batch_task_v2(&NisarAppendSubsetBatchTaskV2Request {
            task_id: "subset-append".to_owned(),
            expected_task_revision: before.runtime.revision,
            source_item_ids: vec![ITEM_2.to_owned()],
            event_id: "subset-append:item-2".to_owned(),
            occurred_at: "2026-09-05T00:00:01Z".to_owned(),
        })
        .unwrap();

    let stored = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("subset-append")
        .unwrap()
        .unwrap();
    assert_eq!(stored.snapshot.candidates.len(), 2);
    let intents = stored
        .snapshot
        .candidates
        .iter()
        .map(|item| {
            parse_nisar_subset_prepared_intent_v2_envelope(&item.metadata)
                .unwrap()
                .unwrap()
                .intent()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(intents[0].requested_geometry, intents[1].requested_geometry);
    assert_eq!(intents[0].selected_frequency, intents[1].selected_frequency);
    assert_eq!(
        intents[0].selected_polarizations,
        intents[1].selected_polarizations
    );
    assert_ne!(
        intents[0].canonical_geometry_digest.effective_sha256,
        intents[1].canonical_geometry_digest.effective_sha256
    );
    assert!(stored
        .runtime
        .selected_item_ids
        .iter()
        .any(|item_id| item_id == ITEM_2));
    let item_2_accounted = stored
        .runtime
        .queued_item_ids
        .iter()
        .chain(stored.runtime.active_item_ids.iter())
        .chain(stored.runtime.completed_item_ids.iter())
        .chain(stored.runtime.failed_item_ids.iter())
        .chain(stored.runtime.skipped_item_ids.iter())
        .chain(stored.runtime.cancelled_item_ids.iter())
        .any(|item_id| item_id == ITEM_2);
    assert!(item_2_accounted);
}

#[test]
fn paused_subset_task_resume_clears_task_and_item_pause_before_scheduler_reclaims() {
    let database = TempStore::new("resume-subset");
    freeze_source(&database.0);
    let (claims, claimed) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let service = fixture_service_with_worker(
        &database.0,
        Arc::new(ClaimingWorker {
            claims,
            release: Arc::clone(&release),
        }),
    );
    let prepared = service
        .prepare_nisar_subset_batch_v2(&prepare_request(
            vec![ITEM_1.to_owned(), ITEM_2.to_owned()],
            rectangle(-1.0, -1.0, 25.0, 25.0),
        ))
        .unwrap();
    let request = NisarConfirmSubsetBatchTaskV2Request {
        source_batch_id: BATCH_ID.to_owned(),
        expected_workbench_revision: prepared.workbench_revision,
        items: prepared
            .intents
            .into_iter()
            .map(|value| NisarConfirmSubsetBatchItemV2 {
                source_item_id: value.source_item_id,
                prepared_intent: value.intent,
            })
            .collect(),
        task_id: "subset-resume".to_owned(),
        output_root: std::env::temp_dir().join("subset-resume-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-05T00:00:00Z".to_owned(),
    };
    service
        .confirm_nisar_subset_batch_task_v2(&request)
        .unwrap();
    service
        .start_newly_confirmed_task(&request.task_id)
        .unwrap();
    for _ in 0..2 {
        claimed.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    let running = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record(&request.task_id)
        .unwrap()
        .unwrap();
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: request.task_id.clone(),
            expected_revision: running.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: Vec::new(),
            },
            event_id: "subset-resume:pause-task".to_owned(),
            occurred_at: "2026-09-05T00:00:01Z".to_owned(),
        })
        .unwrap();
    let task_paused = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record(&request.task_id)
        .unwrap()
        .unwrap();
    assert!(task_paused.runtime.task_paused);
    assert_eq!(task_paused.runtime.active_item_ids.len(), 2);
    assert!(task_paused.runtime.paused_item_ids.is_empty());

    // Match the real UI sequence: PauseTask can be followed immediately by
    // Continue before the active workers publish Interrupted results.
    service.resume_nisar_subset_task(&request.task_id).unwrap();
    let resumed = TaskStore::open(&database.0)
        .unwrap()
        .load_task_record(&request.task_id)
        .unwrap()
        .unwrap();
    assert!(!resumed.runtime.task_paused);
    assert!(resumed.runtime.paused_item_ids.is_empty());
    assert_eq!(
        resumed.runtime.queued_item_ids.len() + resumed.runtime.active_item_ids.len(),
        2
    );

    let (released, changed) = &*release;
    *released.lock().unwrap() = true;
    changed.notify_all();
    service.shutdown().unwrap();
}
