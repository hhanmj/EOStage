use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
    AssetResolverParameters, AssetRole, DownloadItemResult, DownloadLayout, DownloadOutcome,
    FrozenAssetAccess, FrozenSearchBatch, NisarSubsetChecksumAlgorithm, NisarSubsetChecksumV1,
    NisarSubsetDependencyEvidenceAuthorityV2, NisarSubsetDependencyRelationV2,
    NisarSubsetFrequency, NisarSubsetFrequencyCapabilityV2, NisarSubsetPlanMetadataEvidenceV2,
    NisarSubsetPlanV2, NisarSubsetPreparedIntentBuildInputV2, NisarSubsetPreparedIntentV2,
    NisarSubsetQualityStatusV2, NisarSubsetSourceAssetIdentityV2, NisarSubsetSourceCapabilityV2,
    NisarSubsetSourceDependencyEdgeV2, NisarSubsetSourceDependencyGraphV2,
    NisarSubsetSourceDependencyNodeKindV2, NisarSubsetSourceDependencyNodeV2,
    NisarSubsetSourceProductIdentityV2, SearchWorkbenchState, SnapshotItem, StableAssetReference,
    StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskLifecycleStatus, TaskStateCommandKind, NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
};
use insar_store::{
    embed_nisar_subset_plan_v2_envelope, embed_nisar_subset_prepared_intent_v2_envelope,
    parse_nisar_subset_plan_v2_envelope, parse_nisar_subset_prepared_intent_v2_envelope,
    StoreAppendExplicitTaskItemsRequest, StoreAttachNisarSubsetExecutionPlanV2Request,
    StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request, StoreError,
    StoreFreezeNisarSubsetSourceSizeV2Request, StoreTaskTransferTelemetryUpdate,
    StoredNisarSubsetExecutionAnchorV2, TaskStore, NISAR_SUBSET_PLAN_V2_ENVELOPE_METADATA_KEY,
    NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY,
    NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND, NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const ITEM_ID: &str = "PROVIDER::asf::ITEM::G9900000001-ASF";
const ITEM_ID_2: &str = "PROVIDER::asf::ITEM::G9900000002-ASF";
const ITEM_ID_3: &str = "PROVIDER::asf::ITEM::G9900000003-ASF";

#[path = "support/nisar_budget_upgrade.rs"]
mod budget_upgrade;

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-nisar-v2-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn prepared_intent_store_api_is_public() {
    let _ = std::mem::size_of::<StoreAttachNisarSubsetExecutionPlanV2Request>();
    let _embed = embed_nisar_subset_prepared_intent_v2_envelope;
    let _parse = parse_nisar_subset_prepared_intent_v2_envelope;
}

fn rectangle(west: f64, south: f64, east: f64, north: f64) -> Map<String, Value> {
    map(json!({
        "type": "Polygon",
        "coordinates": [[[west, south], [east, south], [east, north], [west, north], [west, south]]]
    }))
}

fn graph() -> NisarSubsetSourceDependencyGraphV2 {
    use NisarSubsetDependencyRelationV2 as Relation;
    use NisarSubsetSourceDependencyNodeKindV2 as Kind;

    let coordinate = "/science/LSAR/RSLC/metadata/geolocationGrid/frequencyA/coordinateX";
    let dimension = "/science/LSAR/RSLC/swaths/frequencyA/zeroDopplerTime";
    let mask = "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask";
    let identification = "/science/LSAR/identification/granuleId";
    let provenance = "/science/LSAR/identification/productVersion";
    let attribute = "/";
    let other = "/science/LSAR/RSLC/metadata/orbit";
    let mut nodes = vec![
        (Kind::CoordinateDataset, coordinate),
        (Kind::DimensionScale, dimension),
        (Kind::ValidityMask, mask),
        (Kind::IdentificationMetadata, identification),
        (Kind::ProvenanceMetadata, provenance),
        (Kind::Attribute, attribute),
        (Kind::OtherScientificDependency, other),
    ]
    .into_iter()
    .map(|(kind, source_path)| NisarSubsetSourceDependencyNodeV2 {
        source_path: source_path.to_owned(),
        kind,
        attribute_name: (kind == Kind::Attribute).then(|| "Conventions".to_owned()),
    })
    .collect::<Vec<_>>();
    let mut edges = Vec::new();
    for polarization in ["HH", "HV"] {
        let science = format!("/science/LSAR/RSLC/swaths/frequencyA/{polarization}");
        nodes.push(NisarSubsetSourceDependencyNodeV2 {
            source_path: science.clone(),
            kind: Kind::ScienceDataset,
            attribute_name: None,
        });
        for (dependency_path, relation) in [
            (coordinate, Relation::Coordinate),
            (dimension, Relation::DimensionScale),
            (mask, Relation::ValidityMask),
            (identification, Relation::RequiredMetadata),
            (provenance, Relation::Provenance),
            (attribute, Relation::RequiredAttribute),
            (other, Relation::OtherScientificDependency),
        ] {
            edges.push(NisarSubsetSourceDependencyEdgeV2 {
                dependent_path: science.clone(),
                dependency_path: dependency_path.to_owned(),
                relation,
            });
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

fn graph_b() -> NisarSubsetSourceDependencyGraphV2 {
    let mut graph = graph();
    graph.evidence_sha256 = "e".repeat(64);
    for node in &mut graph.nodes {
        node.source_path = node.source_path.replace("frequencyA", "frequencyB");
    }
    for edge in &mut graph.edges {
        edge.dependent_path = edge.dependent_path.replace("frequencyA", "frequencyB");
        edge.dependency_path = edge.dependency_path.replace("frequencyA", "frequencyB");
    }
    graph
}

fn prepared_intent() -> NisarSubsetPreparedIntentV2 {
    prepared_intent_with(
        NisarSubsetSourceProductIdentityV2 {
            provider_id: "asf.nasa.earthdata".to_owned(),
            collection_id: "C2853145197-ASF".to_owned(),
            collection_profile_id: "asf.nisar.l.rslc.provisional.v1".to_owned(),
            provider_product_id: "G9900000001-ASF".to_owned(),
            provider_product_key: ITEM_ID.to_owned(),
            product_specification_version: "1.5.0".to_owned(),
            crid: "P05023".to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        NisarSubsetSourceAssetIdentityV2 {
            asset_key: "science-rslc-hdf5".to_owned(),
            stable_asset_key: format!("{ITEM_ID}::ASSET::science-rslc-hdf5"),
            file_name: "NISAR_SOURCE.h5".to_owned(),
            checksum: Some(NisarSubsetChecksumV1 {
                algorithm: NisarSubsetChecksumAlgorithm::Sha256,
                value: "a".repeat(64),
            }),
        },
        map(json!({
            "type": "MultiPolygon",
            "coordinates": [
                [
                    [[-1.0, -1.0], [6.0, -1.0], [6.0, 6.0], [-1.0, 6.0], [-1.0, -1.0]],
                    [[1.0, 1.0], [1.0, 2.0], [2.0, 2.0], [2.0, 1.0], [1.0, 1.0]]
                ],
                [[[8.0, 8.0], [12.0, 8.0], [12.0, 12.0], [8.0, 12.0], [8.0, 8.0]]]
            ]
        })),
        vec!["HV".to_owned(), "HH".to_owned()],
    )
}

fn prepared_intent_two() -> NisarSubsetPreparedIntentV2 {
    prepared_intent_with(
        NisarSubsetSourceProductIdentityV2 {
            provider_id: "asf.nasa.earthdata".to_owned(),
            collection_id: "C2853145197-ASF".to_owned(),
            collection_profile_id: "asf.nisar.l.rslc.provisional.v1".to_owned(),
            provider_product_id: "G9900000002-ASF".to_owned(),
            provider_product_key: ITEM_ID_2.to_owned(),
            product_specification_version: "1.5.0".to_owned(),
            crid: "P05023".to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        NisarSubsetSourceAssetIdentityV2 {
            asset_key: "science-rslc-hdf5".to_owned(),
            stable_asset_key: format!("{ITEM_ID_2}::ASSET::science-rslc-hdf5"),
            file_name: "NISAR_SOURCE_TWO.h5".to_owned(),
            checksum: Some(NisarSubsetChecksumV1 {
                algorithm: NisarSubsetChecksumAlgorithm::Sha256,
                value: "d".repeat(64),
            }),
        },
        rectangle(1.0, 1.0, 5.0, 5.0),
        vec!["HH".to_owned()],
    )
}

fn prepared_intent_three() -> NisarSubsetPreparedIntentV2 {
    prepared_intent_with(
        NisarSubsetSourceProductIdentityV2 {
            provider_id: "asf.nasa.earthdata".to_owned(),
            collection_id: "C2853145197-ASF".to_owned(),
            collection_profile_id: "asf.nisar.l.rslc.provisional.v1".to_owned(),
            provider_product_id: "G9900000003-ASF".to_owned(),
            provider_product_key: ITEM_ID_3.to_owned(),
            product_specification_version: "1.5.0".to_owned(),
            crid: "P05023".to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        NisarSubsetSourceAssetIdentityV2 {
            asset_key: "science-rslc-hdf5".to_owned(),
            stable_asset_key: format!("{ITEM_ID_3}::ASSET::science-rslc-hdf5"),
            file_name: "NISAR_SOURCE_THREE.h5".to_owned(),
            checksum: Some(NisarSubsetChecksumV1 {
                algorithm: NisarSubsetChecksumAlgorithm::Sha256,
                value: "f".repeat(64),
            }),
        },
        rectangle(2.0, 2.0, 6.0, 6.0),
        vec!["HH".to_owned()],
    )
}

fn prepared_intent_with(
    source_product_identity: NisarSubsetSourceProductIdentityV2,
    source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    requested_geometry: Map<String, Value>,
    selected_polarizations: Vec<String>,
) -> NisarSubsetPreparedIntentV2 {
    prepared_intent_with_authority(
        source_product_identity,
        source_asset_identity,
        rectangle(0.0, 0.0, 10.0, 10.0),
        requested_geometry,
        NisarSubsetFrequency::A,
        selected_polarizations,
    )
}

fn prepared_intent_with_authority(
    source_product_identity: NisarSubsetSourceProductIdentityV2,
    source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    source_footprint: Map<String, Value>,
    requested_geometry: Map<String, Value>,
    selected_frequency: NisarSubsetFrequency,
    selected_polarizations: Vec<String>,
) -> NisarSubsetPreparedIntentV2 {
    NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity,
        source_asset_identity,
        source_capability: NisarSubsetSourceCapabilityV2 {
            evidence_sha256: "b".repeat(64),
            frequencies: vec![
                NisarSubsetFrequencyCapabilityV2 {
                    frequency: NisarSubsetFrequency::A,
                    available_polarizations: vec!["HV".to_owned(), "HH".to_owned()],
                },
                NisarSubsetFrequencyCapabilityV2 {
                    frequency: NisarSubsetFrequency::B,
                    available_polarizations: vec!["HV".to_owned(), "HH".to_owned()],
                },
            ],
        },
        source_footprint,
        requested_geometry,
        selected_frequency,
        selected_polarizations,
    })
    .unwrap()
}

fn plan() -> NisarSubsetPlanV2 {
    plan_with_graph(prepared_intent(), graph())
}

fn plan_with_graph(
    intent: NisarSubsetPreparedIntentV2,
    graph: NisarSubsetSourceDependencyGraphV2,
) -> NisarSubsetPlanV2 {
    NisarSubsetPlanV2::build_from_prepared_intent(
        intent,
        graph,
        NisarSubsetPlanMetadataEvidenceV2 {
            source_estimate: None,
            transfer_estimate: None,
            derived_estimate: None,
            warnings: vec!["fixture-only reviewed source evidence".to_owned()],
        },
    )
    .unwrap()
}

fn task_item(intent: &NisarSubsetPreparedIntentV2) -> SnapshotItem {
    task_item_for(intent, source_item())
}

fn task_item_with_deferred_source_size(intent: &NisarSubsetPreparedIntentV2) -> SnapshotItem {
    let mut item = task_item(intent);
    item.asset_access = Some(FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: intent.source_product_identity.provider_id.clone(),
            collection_id: intent.source_product_identity.collection_id.clone(),
            provider_product_id: intent.source_product_identity.provider_product_id.clone(),
            provider_product_key: intent.source_product_identity.provider_product_key.clone(),
            canonical_product_key: None,
            asset_key: intent.source_asset_identity.asset_key.clone(),
            stable_asset_key: intent.source_asset_identity.stable_asset_key.clone(),
        },
        descriptor: AssetAccessDescriptor {
            resolver_id: "fixture.nisar.resolver.v1".to_owned(),
            scheme: AssetAccessScheme::OauthBearerHttps,
            stable_href: Some("https://example.invalid/nisar-source.h5".to_owned()),
            requires_credentials: true,
            credential_profile: Some("fixture-bhoonidhi".to_owned()),
            resolver_parameters: AssetResolverParameters {},
        },
        role: AssetRole::Primary,
        file_name: Some(intent.source_asset_identity.file_name.clone()),
        expected_size_bytes: None,
        checksum_algorithm: intent
            .source_asset_identity
            .checksum
            .as_ref()
            .map(|_| "sha256".to_owned()),
        checksum_value: intent
            .source_asset_identity
            .checksum
            .as_ref()
            .map(|checksum| checksum.value.clone()),
    });
    item.execution_policy = Some(AssetExecutionPolicy {
        authority: AssetExecutionAuthority::ResolverAuthoritative,
        policy_id: "fixture.nisar.resolver.v1".to_owned(),
        collection_profile_id: Some(intent.source_product_identity.collection_profile_id.clone()),
    });
    item
}

fn task_item_for(intent: &NisarSubsetPreparedIntentV2, mut item: SnapshotItem) -> SnapshotItem {
    item.metadata.insert(
        "source_item_id".to_owned(),
        json!(intent.source_product_identity.provider_product_key),
    );
    item.metadata.insert(
        "dataset".to_owned(),
        json!("sar.nisar.l.rslc.provisional.v1"),
    );
    item.metadata.insert(
        "delivery_kind".to_owned(),
        json!("remote_derived_subset_v2"),
    );
    embed_nisar_subset_prepared_intent_v2_envelope(&mut item.metadata, intent).unwrap();
    item
}

fn source_item() -> SnapshotItem {
    source_item_with_id(ITEM_ID)
}

fn source_item_two() -> SnapshotItem {
    source_item_with_id(ITEM_ID_2)
}

fn source_item_three() -> SnapshotItem {
    source_item_with_id(ITEM_ID_3)
}

fn source_item_with_id(item_id: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: "NISAR source".to_owned(),
        acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
        footprint_geojson: Some(rectangle(0.0, 0.0, 10.0, 10.0)),
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn save_source(store: &mut TaskStore) -> FrozenSearchBatch {
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "nisar-source-batch".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        created_at: "2026-08-29T00:00:00Z".to_owned(),
        aoi_name: "Working AOI A".to_owned(),
        aoi_geojson: Some(rectangle(3.0, 3.0, 4.0, 4.0)),
        query: Map::new(),
        search_stats: None,
        candidates: vec![source_item()],
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: vec![ITEM_ID.to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    batch
}

fn task_request(path: &Path, task_id: &str) -> StoreCreateTaskRequest {
    StoreCreateTaskRequest {
        task_id: task_id.to_owned(),
        batch_id: "nisar-source-batch".to_owned(),
        selected_item_ids: vec![ITEM_ID.to_owned()],
        checked_item_ids: vec![ITEM_ID.to_owned()],
        page_index: 0,
        page_size: 24,
        output_root: path.join("output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-08-29T00:00:00Z".to_owned(),
    }
}

fn create_intent_task(
    store: &mut TaskStore,
    database: &Path,
    task_id: &str,
    intent: &NisarSubsetPreparedIntentV2,
) {
    save_source(store);
    store
        .create_paused_task_with_explicit_items(
            &task_request(database, task_id),
            0,
            &[ITEM_ID.to_owned()],
            &[task_item(intent)],
        )
        .unwrap();
}

fn activate_intent_task(store: &mut TaskStore, task_id: &str) {
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeTask,
                item_ids: vec![],
            },
            event_id: format!("{task_id}:resume"),
            occurred_at: "2026-08-29T00:00:30Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeItems,
                item_ids: vec![ITEM_ID.to_owned()],
            },
            event_id: format!("{task_id}:resume-item"),
            occurred_at: "2026-08-29T00:00:40Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 2,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec![ITEM_ID.to_owned()],
            },
            event_id: format!("{task_id}:activate"),
            occurred_at: "2026-08-29T00:00:45Z".to_owned(),
        })
        .unwrap();
}

fn create_active_deferred_size_task(
    store: &mut TaskStore,
    database: &Path,
    task_id: &str,
    intent: &NisarSubsetPreparedIntentV2,
) {
    save_source(store);
    store
        .create_paused_task_with_explicit_items(
            &task_request(database, task_id),
            0,
            &[ITEM_ID.to_owned()],
            &[task_item_with_deferred_source_size(intent)],
        )
        .unwrap();
    activate_intent_task(store, task_id);
}

fn source_size_request(
    task_id: &str,
    intent: &NisarSubsetPreparedIntentV2,
    expected_revision: u64,
    expected_size_bytes: u64,
) -> StoreFreezeNisarSubsetSourceSizeV2Request {
    StoreFreezeNisarSubsetSourceSizeV2Request {
        task_id: task_id.to_owned(),
        item_id: ITEM_ID.to_owned(),
        provider_id: intent.source_product_identity.provider_id.clone(),
        collection_id: intent.source_product_identity.collection_id.clone(),
        provider_product_id: intent.source_product_identity.provider_product_id.clone(),
        provider_product_key: intent.source_product_identity.provider_product_key.clone(),
        asset_key: intent.source_asset_identity.asset_key.clone(),
        stable_asset_key: intent.source_asset_identity.stable_asset_key.clone(),
        prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
        expected_revision,
        occurred_at: "2026-10-16T00:00:00Z".to_owned(),
        expected_size_bytes,
    }
}

fn create_active_planned_task(store: &mut TaskStore, database: &Path, task_id: &str) {
    let intent = prepared_intent();
    create_intent_task(store, database, task_id, &intent);
    activate_intent_task(store, task_id);
    store
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: task_id.to_owned(),
            item_id: ITEM_ID.to_owned(),
            prepared_intent_sha256: intent.canonical_intent_sha256,
            expected_revision: 3,
            occurred_at: "2026-08-29T00:01:00Z".to_owned(),
            plan: plan(),
        })
        .unwrap();
}

fn anchor_request(
    task_id: &str,
    kind: &str,
    lineage: char,
    expected_record_sha256: Option<String>,
    record: char,
    canonical_json: String,
) -> StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
    StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
        task_id: task_id.to_owned(),
        item_id: ITEM_ID.to_owned(),
        anchor_kind: kind.to_owned(),
        lineage_key_sha256: lineage.to_string().repeat(64),
        expected_record_sha256,
        record_sha256: record.to_string().repeat(64),
        canonical_json,
    }
}

const TEST_RECOVERY_TELEMETRY_CONTRACT: &str = "nisar_subset_committed_recovery_telemetry_v1";
const TEST_RECOVERY_TELEMETRY_DIGEST_CONTRACT: &str =
    "nisar_subset_science_checkpoint_anchor_record_v2_recovery_telemetry_v1";

#[derive(Clone, Serialize)]
struct TestRecoveryTelemetry {
    contract: String,
    checkpoint_sha256: String,
    reusable_verified_bytes: u64,
    reused_verified_bytes: u64,
}

#[derive(Serialize)]
struct TestScienceAnchorRecord {
    lineage_key_sha256: String,
    execution_binding_sha256: String,
    generation: u64,
    committed_checkpoint_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    committed_recovery_telemetry: Option<TestRecoveryTelemetry>,
    pending_transition: Option<Value>,
    record_sha256: String,
}

fn test_sha256(value: &impl Serialize) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}

fn extended_recovery_anchor(
    reusable_verified_bytes: u64,
    reused_verified_bytes: u64,
) -> (String, String, String) {
    let lineage = "a".repeat(64);
    let execution_binding = "b".repeat(64);
    let checkpoint = "c".repeat(64);
    let committed = Some(checkpoint.clone());
    let telemetry = TestRecoveryTelemetry {
        contract: TEST_RECOVERY_TELEMETRY_CONTRACT.to_owned(),
        checkpoint_sha256: checkpoint,
        reusable_verified_bytes,
        reused_verified_bytes,
    };
    let pending: Option<Value> = None;
    let record_sha256 = test_sha256(&(
        TEST_RECOVERY_TELEMETRY_DIGEST_CONTRACT,
        &lineage,
        &execution_binding,
        7_u64,
        &committed,
        &telemetry,
        &pending,
    ));
    let canonical_json = serde_json::to_string(&TestScienceAnchorRecord {
        lineage_key_sha256: lineage.clone(),
        execution_binding_sha256: execution_binding,
        generation: 7,
        committed_checkpoint_sha256: committed,
        committed_recovery_telemetry: Some(telemetry),
        pending_transition: pending,
        record_sha256: record_sha256.clone(),
    })
    .unwrap();
    (lineage, record_sha256, canonical_json)
}

fn save_extended_recovery_anchor(
    store: &mut TaskStore,
    task_id: &str,
    item_id: &str,
    reusable_verified_bytes: u64,
    reused_verified_bytes: u64,
) {
    let (lineage_key_sha256, record_sha256, canonical_json) =
        extended_recovery_anchor(reusable_verified_bytes, reused_verified_bytes);
    assert!(store
        .compare_and_swap_nisar_subset_execution_anchor_v2(
            &StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
                task_id: task_id.to_owned(),
                item_id: item_id.to_owned(),
                anchor_kind: NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND.to_owned(),
                lineage_key_sha256,
                expected_record_sha256: None,
                record_sha256,
                canonical_json,
            },
        )
        .unwrap());
}

#[test]
fn recovery_telemetry_projects_valid_anchor_without_conflating_network_bytes() {
    let database = TempDatabase::new("recovery-telemetry-valid");
    let task_id = "nisar-v2-recovery-telemetry-valid";
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), task_id);
    store
        .save_task_transfer_telemetry_with_network(
            &StoreTaskTransferTelemetryUpdate {
                task_id: task_id.to_owned(),
                expected_revision: 0,
                transfer_active_millis: 1_000,
                updated_at: "2026-08-29T00:02:00Z".to_owned(),
            },
            Some(16_900_000),
        )
        .unwrap();
    save_extended_recovery_anchor(&mut store, task_id, ITEM_ID, 49_686_192, 47_876_062);

    let summary = store
        .core_restore(false)
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| task.task_id == task_id)
        .unwrap();
    assert_eq!(summary.task_type.as_deref(), Some("nisar_subset"));
    assert_eq!(summary.network_bytes_received, Some(16_900_000));
    assert_eq!(summary.reusable_verified_bytes, Some(49_686_192));
    assert_eq!(summary.reused_verified_bytes, Some(47_876_062));
}

fn archived_recovery_source(store: &mut TaskStore, database: &Path) -> insar_store::StoredTask {
    use insar_core::{
        DownloadFailure, DownloadItemResult, DownloadOutcome, StoreArchiveTaskRequest,
    };
    let task_id = "archived-recovery-source";
    create_active_planned_task(store, database, task_id);
    let anchor_json = json!({
        "execution_binding_sha256": "d".repeat(64), "generation": 7,
        "lineage_key_sha256": "a".repeat(64), "record_sha256": "b".repeat(64),
        "committed_checkpoint_sha256": "c".repeat(64), "pending_transition": null
    })
    .to_string();
    store
        .compare_and_swap_nisar_subset_execution_anchor_v2(&anchor_request(
            task_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            'a',
            None,
            'b',
            anchor_json,
        ))
        .unwrap();
    store
        .save_scheduler_download_result(&DownloadItemResult {
            task_id: task_id.to_owned(),
            item_id: ITEM_ID.to_owned(),
            outcome: DownloadOutcome::Failed,
            destination: database.join("output"),
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(DownloadFailure {
                code: "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED".to_owned(),
                reason_zh: "fixture auth failure".to_owned(),
                retryable: false,
            }),
            finished_at: "2026-08-29T00:02:00Z".to_owned(),
        })
        .unwrap();
    let current = store.load_task_record(task_id).unwrap().unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: current.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec![ITEM_ID.to_owned()],
            },
            event_id: "recovery-source-failed".to_owned(),
            occurred_at: "2026-08-29T00:02:00Z".to_owned(),
        })
        .unwrap();
    let current = store.load_task_record(task_id).unwrap().unwrap();
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: current.runtime.revision,
            archived_at: "2026-08-29T00:03:00Z".to_owned(),
        })
        .unwrap();
    store.load_task_record(task_id).unwrap().unwrap()
}

#[test]
fn archived_recovery_creates_paused_successor_without_mutating_source() {
    let database = TempDatabase::new("archived-recovery");
    let mut store = TaskStore::open(database.path()).unwrap();
    let source = archived_recovery_source(&mut store, database.path());
    let original_rows = archived_recovery_rows(database.path(), &source.snapshot.task_id);
    let request = insar_store::StoreCreateNisarArchivedRecoveryV2Request {
        source_task_id: source.snapshot.task_id.clone(),
        source_item_id: ITEM_ID.to_owned(),
        expected_source_revision: source.runtime.revision,
        new_task_id: "recovery-new".to_owned(),
        created_at: "2026-08-29T00:04:00Z".to_owned(),
    };
    let recovered = store
        .create_nisar_archived_recovery_v2(&request)
        .expect("confirmed archived recovery creates a new task");
    assert_eq!(recovered.runtime.status, TaskLifecycleStatus::Paused);
    assert!(recovered.runtime.task_paused);
    assert_eq!(recovered.runtime.paused_item_ids, [ITEM_ID]);
    assert_eq!(
        recovered.snapshot.output_root,
        source.snapshot.output_root.join("recovery-new")
    );
    assert_eq!(
        store
            .load_task_record(&source.snapshot.task_id)
            .unwrap()
            .unwrap(),
        source
    );
    assert_eq!(
        store
            .load_task(&source.snapshot.task_id)
            .unwrap()
            .unwrap()
            .runtime
            .status,
        TaskLifecycleStatus::Failed
    );
    assert_eq!(
        archived_recovery_rows(database.path(), &source.snapshot.task_id),
        original_rows
    );
    assert_eq!(
        store.create_nisar_archived_recovery_v2(&request).unwrap(),
        recovered
    );
    let proof = store
        .load_nisar_archived_recovery_v2("recovery-new", ITEM_ID)
        .unwrap()
        .unwrap();
    assert_eq!(proof.source_snapshot, source.snapshot);
    assert_eq!(proof.source_anchor.record_sha256, "b".repeat(64));
    drop(store);
    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened
            .load_nisar_archived_recovery_v2("recovery-new", ITEM_ID)
            .unwrap()
            .unwrap(),
        proof
    );
    assert_eq!(
        archived_recovery_rows(database.path(), &source.snapshot.task_id),
        original_rows
    );
}

fn archived_recovery_rows(database: &Path, task_id: &str) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let connection = Connection::open(database).unwrap();
    [
        "task_snapshots",
        "task_runtime",
        "task_view_state",
        "task_events",
        "task_history",
        "download_results",
        "nisar_subset_execution_anchors_v2",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = connection
            .prepare(&format!(
                "SELECT * FROM {table} WHERE task_id=?1 ORDER BY rowid"
            ))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([task_id], |row| {
                (0..columns)
                    .map(|index| row.get(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}

fn archived_recovery_request(
    source: &insar_store::StoredTask,
    new_task_id: &str,
) -> insar_store::StoreCreateNisarArchivedRecoveryV2Request {
    insar_store::StoreCreateNisarArchivedRecoveryV2Request {
        source_task_id: source.snapshot.task_id.clone(),
        source_item_id: ITEM_ID.to_owned(),
        expected_source_revision: source.runtime.revision,
        new_task_id: new_task_id.to_owned(),
        created_at: "2026-08-29T00:04:00Z".to_owned(),
    }
}

#[test]
fn archived_recovery_rejects_stale_revision_without_partial_destination() {
    let database = TempDatabase::new("recovery-stale-revision");
    let mut store = TaskStore::open(database.path()).unwrap();
    let source = archived_recovery_source(&mut store, database.path());
    let before = archived_recovery_rows(database.path(), &source.snapshot.task_id);
    let mut request = archived_recovery_request(&source, "recovery-stale");
    request.expected_source_revision += 1;
    assert!(matches!(
        store.create_nisar_archived_recovery_v2(&request),
        Err(StoreError::RevisionConflict)
    ));
    assert!(store
        .load_task_record(&request.new_task_id)
        .unwrap()
        .is_none());
    assert_eq!(
        archived_recovery_rows(database.path(), &source.snapshot.task_id),
        before
    );
}

#[test]
fn archived_recovery_rejects_non_recoverable_failure_without_touching_source() {
    let database = TempDatabase::new("recovery-not-eligible");
    let mut store = TaskStore::open(database.path()).unwrap();
    let source = archived_recovery_source(&mut store, database.path());
    let connection = Connection::open(database.path()).unwrap();
    connection.execute(
        "UPDATE download_results SET result_json=json_set(result_json,'$.failure.code','NISAR_SUBSET_V2_EXECUTION_STOPPED') WHERE task_id=?1 AND item_id=?2",
        params![source.snapshot.task_id, ITEM_ID],
    ).unwrap();
    drop(connection);
    let before = archived_recovery_rows(database.path(), &source.snapshot.task_id);
    let request = archived_recovery_request(&source, "recovery-denied");
    let error = store
        .create_nisar_archived_recovery_v2(&request)
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::NisarArchivedRecoverySourceIneligible
    ));
    assert_eq!(error.code(), "NISAR_ARCHIVED_RECOVERY_SOURCE_INELIGIBLE");
    assert!(store
        .load_task_record(&request.new_task_id)
        .unwrap()
        .is_none());
    assert_eq!(
        archived_recovery_rows(database.path(), &source.snapshot.task_id),
        before
    );
}

#[test]
fn archived_recovery_rejects_second_successor_but_same_request_is_idempotent() {
    let database = TempDatabase::new("recovery-duplicate");
    let mut store = TaskStore::open(database.path()).unwrap();
    let source = archived_recovery_source(&mut store, database.path());
    let before = archived_recovery_rows(database.path(), &source.snapshot.task_id);
    let first = archived_recovery_request(&source, "recovery-first");
    let recovered = store.create_nisar_archived_recovery_v2(&first).unwrap();
    assert_eq!(
        store.create_nisar_archived_recovery_v2(&first).unwrap(),
        recovered
    );
    let second = archived_recovery_request(&source, "recovery-second");
    assert!(matches!(
        store.create_nisar_archived_recovery_v2(&second),
        Err(StoreError::AlreadyExists)
    ));
    assert!(store
        .load_task_record(&second.new_task_id)
        .unwrap()
        .is_none());
    assert_eq!(
        archived_recovery_rows(database.path(), &source.snapshot.task_id),
        before
    );
}

#[test]
fn archived_recovery_rejects_pending_checkpoint_or_any_writer_anchor() {
    for writer in [false, true] {
        let database = TempDatabase::new("recovery-pending-writer");
        let mut store = TaskStore::open(database.path()).unwrap();
        let source = archived_recovery_source(&mut store, database.path());
        let connection = Connection::open(database.path()).unwrap();
        // Fixture-only persisted boundary states; never mutate an actual Store.
        if writer {
            connection.execute("INSERT INTO nisar_subset_execution_anchors_v2(task_id,item_id,anchor_kind,lineage_key_sha256,record_sha256,canonical_json) VALUES(?1,?2,?3,?4,?5,'{}')",
                params![source.snapshot.task_id, ITEM_ID, NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND, "a".repeat(64), "f".repeat(64)]).unwrap();
        } else {
            connection.execute("UPDATE nisar_subset_execution_anchors_v2 SET canonical_json=json_set(canonical_json,'$.pending_transition',json('{\"fixture\":\"pending\"}')) WHERE task_id=?1",
                [&source.snapshot.task_id]).unwrap();
        }
        let before = archived_recovery_rows(database.path(), &source.snapshot.task_id);
        let request = archived_recovery_request(&source, "recovery-unsafe");
        assert!(store.create_nisar_archived_recovery_v2(&request).is_err());
        assert!(store
            .load_task_record(&request.new_task_id)
            .unwrap()
            .is_none());
        assert_eq!(
            archived_recovery_rows(database.path(), &source.snapshot.task_id),
            before
        );
    }
}

#[test]
fn archived_recovery_rejects_source_or_destination_tombstone_before_creation() {
    for tombstone_source in [false, true] {
        let database = TempDatabase::new("recovery-tombstone-create");
        let mut store = TaskStore::open(database.path()).unwrap();
        let source = archived_recovery_source(&mut store, database.path());
        let request = archived_recovery_request(&source, "recovery-tombstone");
        let connection = Connection::open(database.path()).unwrap();
        let tombstoned = if tombstone_source {
            &request.source_task_id
        } else {
            &request.new_task_id
        };
        connection.execute("INSERT INTO task_tombstones(task_id,deleted_at,identity_json) VALUES(?1,'2026-08-29T00:03:30Z','{}')", [tombstoned]).unwrap();
        let before = archived_recovery_rows(database.path(), &source.snapshot.task_id);
        assert!(matches!(
            store.create_nisar_archived_recovery_v2(&request),
            Err(StoreError::NotFound)
        ));
        assert!(store
            .load_task_record(&request.new_task_id)
            .unwrap()
            .is_none());
        assert_eq!(
            archived_recovery_rows(database.path(), &source.snapshot.task_id),
            before
        );
    }
}

#[test]
fn archived_recovery_load_rejects_tombstoned_successor() {
    let database = TempDatabase::new("recovery-tombstone-load");
    let mut store = TaskStore::open(database.path()).unwrap();
    let source = archived_recovery_source(&mut store, database.path());
    let request = archived_recovery_request(&source, "recovery-deleted");
    store.create_nisar_archived_recovery_v2(&request).unwrap();
    let connection = Connection::open(database.path()).unwrap();
    connection.execute("INSERT INTO task_tombstones(task_id,deleted_at,identity_json) VALUES(?1,'2026-08-29T00:05:00Z','{}')", [&request.new_task_id]).unwrap();
    assert!(matches!(
        store.load_nisar_archived_recovery_v2(&request.new_task_id, ITEM_ID),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn archived_recovery_marker_cannot_be_injected_by_ordinary_explicit_create() {
    let database = TempDatabase::new("recovery-marker-injection");
    let mut store = TaskStore::open(database.path()).unwrap();
    save_source(&mut store);
    let mut item = task_item(&prepared_intent());
    item.metadata.insert("nisar_archived_recovery_v2".to_owned(), json!({"schema_version":1,"source_task_id":"fake-source","source_item_id":ITEM_ID,"proof_sha256":"a".repeat(64)}));
    let request = task_request(database.path(), "injected-recovery");
    assert!(matches!(
        store.create_paused_task_with_explicit_items(&request, 0, &[ITEM_ID.to_owned()], &[item]),
        Err(StoreError::NisarSubsetExecutionPlanInvalid)
    ));
    assert!(store.load_task_record(&request.task_id).unwrap().is_none());
}

#[test]
fn archived_recovery_provenance_cannot_be_updated_or_deleted() {
    let database = TempDatabase::new("recovery-proof-immutable");
    let mut store = TaskStore::open(database.path()).unwrap();
    let source = archived_recovery_source(&mut store, database.path());
    let request = archived_recovery_request(&source, "recovery-immutable");
    store.create_nisar_archived_recovery_v2(&request).unwrap();
    let proof = store
        .load_nisar_archived_recovery_v2(&request.new_task_id, ITEM_ID)
        .unwrap();
    let connection = Connection::open(database.path()).unwrap();
    assert!(connection
        .execute(
            "UPDATE nisar_archived_recovery_v2 SET proof_sha256=?1 WHERE task_id=?2",
            params!["f".repeat(64), request.new_task_id]
        )
        .is_err());
    assert!(connection
        .execute(
            "DELETE FROM nisar_archived_recovery_v2 WHERE task_id=?1",
            [&request.new_task_id]
        )
        .is_err());
    assert_eq!(
        store
            .load_nisar_archived_recovery_v2(&request.new_task_id, ITEM_ID)
            .unwrap(),
        proof
    );
}

#[test]
fn prepared_intent_geometry_envelopes_roundtrip_canonically() {
    let shapes = [
        rectangle(1.0, 1.0, 3.0, 3.0),
        map(json!({
            "type": "Polygon",
            "coordinates": [
                [[1.0, 1.0], [5.0, 1.0], [5.0, 5.0], [1.0, 5.0], [1.0, 1.0]],
                [[2.0, 2.0], [2.0, 3.0], [3.0, 3.0], [3.0, 2.0], [2.0, 2.0]]
            ]
        })),
        map(json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0], [1.0, 1.0]]],
                [[[6.0, 6.0], [9.0, 6.0], [9.0, 9.0], [6.0, 9.0], [6.0, 6.0]]]
            ]
        })),
    ];
    for shape in shapes {
        let base = prepared_intent();
        let intent = prepared_intent_with(
            base.source_product_identity,
            base.source_asset_identity,
            shape,
            vec!["HH".to_owned(), "HV".to_owned()],
        );
        let mut metadata = Map::new();
        embed_nisar_subset_prepared_intent_v2_envelope(&mut metadata, &intent).unwrap();
        let encoded = serde_json::to_string(&metadata).unwrap();
        let decoded: Map<String, Value> = serde_json::from_str(&encoded).unwrap();
        let envelope = parse_nisar_subset_prepared_intent_v2_envelope(&decoded)
            .unwrap()
            .unwrap();
        assert_eq!(
            envelope.canonical_intent_json,
            intent.canonical_json().unwrap()
        );
        assert_eq!(
            envelope.canonical_intent_sha256,
            intent.canonical_intent_sha256
        );
        assert_eq!(envelope.intent().unwrap(), intent);
    }
}

#[test]
fn crash_before_plan_reopens_with_exact_intent_and_no_execution_plan() {
    let database = TempDatabase::new("intent-only");
    let intent = prepared_intent();
    let canonical = intent.canonical_json().unwrap();
    let mut store = TaskStore::open(database.path()).unwrap();
    let source_batch = save_source(&mut store);
    let source_workbench = store.load_workbench().unwrap().unwrap();
    store
        .create_paused_task_with_explicit_items(
            &task_request(database.path(), "nisar-v2-intent-only"),
            0,
            &[ITEM_ID.to_owned()],
            &[task_item(&intent)],
        )
        .unwrap();
    drop(store);

    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.load_search_batch("nisar-source-batch").unwrap(),
        Some(source_batch)
    );
    assert_eq!(reopened.load_workbench().unwrap(), Some(source_workbench));
    let task = reopened
        .load_task_record("nisar-v2-intent-only")
        .unwrap()
        .unwrap();
    assert_eq!(task.runtime.status, TaskLifecycleStatus::Paused);
    assert!(task.runtime.task_paused);
    assert!(task.runtime.queued_item_ids.is_empty());
    let metadata = &task.snapshot.candidates[0].metadata;
    let envelope = parse_nisar_subset_prepared_intent_v2_envelope(metadata)
        .unwrap()
        .unwrap();
    assert_eq!(envelope.canonical_intent_json, canonical);
    assert_eq!(envelope.intent().unwrap(), intent);
    assert!(parse_nisar_subset_plan_v2_envelope(metadata)
        .unwrap()
        .is_none());
}

#[test]
fn tampered_prepared_intent_projection_is_typed_and_store_is_not_mutated() {
    let database = TempDatabase::new("tamper-intent");
    let mut store = TaskStore::open(database.path()).unwrap();
    save_source(&mut store);
    let mut item = task_item(&prepared_intent());
    item.metadata[NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY]
        ["canonical_intent_sha256"] = json!("f".repeat(64));
    assert!(matches!(
        store.create_paused_task_with_explicit_items(
            &task_request(database.path(), "nisar-v2-tampered"),
            0,
            &[ITEM_ID.to_owned()],
            &[item],
        ),
        Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch)
    ));
    assert!(store
        .load_task_record("nisar-v2-tampered")
        .unwrap()
        .is_none());
}

#[test]
fn task_creation_freezes_only_prepared_intent() {
    let database = TempDatabase::new("reject-prefrozen-plan");
    let mut store = TaskStore::open(database.path()).unwrap();
    save_source(&mut store);
    let intent = prepared_intent();
    let intent_only_item = task_item(&intent);
    let mut item = intent_only_item.clone();
    embed_nisar_subset_plan_v2_envelope(&mut item.metadata, &plan()).unwrap();
    assert!(matches!(
        store.create_paused_task_with_explicit_items(
            &task_request(database.path(), "nisar-v2-prefrozen-plan"),
            0,
            &[ITEM_ID.to_owned()],
            &[item.clone()],
        ),
        Err(StoreError::NisarSubsetExecutionPlanAlreadyFrozenConflict)
    ));
    assert!(matches!(
        store.create_task_with_explicit_items(
            &task_request(database.path(), "nisar-v2-intent-running"),
            0,
            &[ITEM_ID.to_owned()],
            &[intent_only_item],
        ),
        Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch)
    ));
    assert!(matches!(
        store.create_task_with_explicit_items(
            &task_request(database.path(), "nisar-v2-prefrozen-plan-running"),
            0,
            &[ITEM_ID.to_owned()],
            &[item],
        ),
        Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch)
    ));
}

#[test]
fn source_batch_task_creation_cannot_bypass_plan_attach_boundary() {
    for include_plan in [false, true] {
        let label = if include_plan {
            "source-batch-prefrozen"
        } else {
            "source-batch-intent"
        };
        let database = TempDatabase::new(label);
        let intent = prepared_intent();
        let mut source = source_item();
        embed_nisar_subset_prepared_intent_v2_envelope(&mut source.metadata, &intent).unwrap();
        if include_plan {
            embed_nisar_subset_plan_v2_envelope(&mut source.metadata, &plan()).unwrap();
        }
        let batch = FrozenSearchBatch {
            schema_version: 1,
            batch_id: "nisar-source-batch".to_owned(),
            provider_id: "asf.nasa.earthdata".to_owned(),
            created_at: "2026-08-29T00:00:00Z".to_owned(),
            aoi_name: "Working AOI A".to_owned(),
            aoi_geojson: Some(rectangle(3.0, 3.0, 4.0, 4.0)),
            query: Map::new(),
            search_stats: None,
            candidates: vec![source],
        };
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch: batch.clone(),
                workbench: SearchWorkbenchState {
                    batch_id: batch.batch_id.clone(),
                    checked_item_ids: vec![ITEM_ID.to_owned()],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
                activate_workbench: true,
            })
            .unwrap();
        assert!(matches!(
            store.create_task(&task_request(database.path(), "nisar-source-batch-bypass")),
            Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch)
        ));
        assert!(store
            .load_task_record("nisar-source-batch-bypass")
            .unwrap()
            .is_none());
    }
}

#[test]
fn prepared_intent_task_creation_requires_fresh_task_id_before_and_after_attach() {
    let database = TempDatabase::new("prepared-intent-fresh-task-id");
    let intent = prepared_intent();
    let mut store = TaskStore::open(database.path()).unwrap();
    save_source(&mut store);
    let request = task_request(database.path(), "nisar-v2-fresh-task-id");
    let item = task_item(&intent);

    store
        .create_paused_task_with_explicit_items(
            &request,
            0,
            &[ITEM_ID.to_owned()],
            std::slice::from_ref(&item),
        )
        .unwrap();
    assert!(matches!(
        store.create_paused_task_with_explicit_items(
            &request,
            0,
            &[ITEM_ID.to_owned()],
            std::slice::from_ref(&item),
        ),
        Err(StoreError::AlreadyExists)
    ));

    activate_intent_task(&mut store, &request.task_id);
    store
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: request.task_id.clone(),
            item_id: ITEM_ID.to_owned(),
            prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
            expected_revision: 3,
            occurred_at: "2026-08-29T00:01:00Z".to_owned(),
            plan: plan(),
        })
        .unwrap();
    assert!(matches!(
        store.create_paused_task_with_explicit_items(&request, 0, &[ITEM_ID.to_owned()], &[item],),
        Err(StoreError::AlreadyExists)
    ));
    let persisted = store.load_task_record(&request.task_id).unwrap().unwrap();
    assert_eq!(persisted.runtime.revision, 4);
    assert!(
        parse_nisar_subset_plan_v2_envelope(&persisted.snapshot.candidates[0].metadata)
            .unwrap()
            .is_some()
    );
}

#[test]
fn resolver_source_size_freeze_is_exact_once_conflict_safe_and_reopen_durable() {
    let database = TempDatabase::new("source-size-freeze");
    let task_id = "nisar-v2-source-size-freeze";
    let intent = prepared_intent();
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_deferred_size_task(&mut store, database.path(), task_id, &intent);
    assert_eq!(projected_task_totals(&store, task_id), (None, None));
    let request = source_size_request(task_id, &intent, 3, 38_067_799_318);

    let frozen = store.freeze_nisar_subset_source_size_v2(&request).unwrap();
    assert_eq!(frozen.runtime.revision, 4);
    assert_eq!(frozen.runtime.status, TaskLifecycleStatus::Running);
    assert_eq!(frozen.runtime.active_item_ids, [ITEM_ID]);
    assert_eq!(
        frozen.snapshot.candidates[0]
            .asset_access
            .as_ref()
            .and_then(|access| access.expected_size_bytes),
        Some(request.expected_size_bytes)
    );
    assert!(
        parse_nisar_subset_plan_v2_envelope(&frozen.snapshot.candidates[0].metadata)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        projected_task_totals(&store, task_id),
        (
            Some(request.expected_size_bytes),
            Some(request.expected_size_bytes)
        )
    );
    let event_count = store.list_task_events(task_id).unwrap().len();

    let mut repeated = request.clone();
    repeated.expected_revision = 4;
    repeated.occurred_at = "2026-10-16T00:00:01Z".to_owned();
    let idempotent = store.freeze_nisar_subset_source_size_v2(&repeated).unwrap();
    assert_eq!(idempotent.runtime.revision, 4);
    assert_eq!(store.list_task_events(task_id).unwrap().len(), event_count);

    let mut conflicting = repeated.clone();
    conflicting.expected_size_bytes += 1;
    assert!(matches!(
        store.freeze_nisar_subset_source_size_v2(&conflicting),
        Err(StoreError::NisarSubsetSourceSizeAlreadyFrozenConflict)
    ));
    assert_eq!(
        store
            .load_task_record(task_id)
            .unwrap()
            .unwrap()
            .snapshot
            .candidates[0]
            .asset_access
            .as_ref()
            .and_then(|access| access.expected_size_bytes),
        Some(request.expected_size_bytes)
    );
    drop(store);

    let reopened = TaskStore::open(database.path()).unwrap();
    let task = reopened.load_task_record(task_id).unwrap().unwrap();
    assert_eq!(task.runtime.revision, 4);
    assert_eq!(
        task.snapshot.candidates[0]
            .asset_access
            .as_ref()
            .and_then(|access| access.expected_size_bytes),
        Some(request.expected_size_bytes)
    );
    assert_eq!(
        projected_task_totals(&reopened, task_id),
        (
            Some(request.expected_size_bytes),
            Some(request.expected_size_bytes)
        )
    );
}

#[test]
fn resolver_source_size_freeze_rejects_wrong_authority_revision_and_state_without_mutation() {
    let database = TempDatabase::new("source-size-binding");
    let task_id = "nisar-v2-source-size-binding";
    let intent = prepared_intent();
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_deferred_size_task(&mut store, database.path(), task_id, &intent);
    let valid = source_size_request(task_id, &intent, 3, 4096);
    let before = store.load_task_record(task_id).unwrap().unwrap();

    let mut wrong_item = valid.clone();
    wrong_item.item_id = "wrong-item".to_owned();
    assert!(matches!(
        store.freeze_nisar_subset_source_size_v2(&wrong_item),
        Err(StoreError::NisarSubsetSourceSizeStateInvalid)
    ));
    let mut wrong_provider = valid.clone();
    wrong_provider.provider_id = "wrong.provider".to_owned();
    assert!(matches!(
        store.freeze_nisar_subset_source_size_v2(&wrong_provider),
        Err(StoreError::NisarSubsetSourceSizeIdentityMismatch)
    ));
    let mut wrong_product = valid.clone();
    wrong_product.provider_product_id = "wrong-product".to_owned();
    assert!(matches!(
        store.freeze_nisar_subset_source_size_v2(&wrong_product),
        Err(StoreError::NisarSubsetSourceSizeIdentityMismatch)
    ));
    let mut wrong_asset = valid.clone();
    wrong_asset.stable_asset_key = "wrong-stable-asset".to_owned();
    assert!(matches!(
        store.freeze_nisar_subset_source_size_v2(&wrong_asset),
        Err(StoreError::NisarSubsetSourceSizeIdentityMismatch)
    ));
    let mut wrong_intent = valid.clone();
    wrong_intent.prepared_intent_sha256 = "0".repeat(64);
    assert!(matches!(
        store.freeze_nisar_subset_source_size_v2(&wrong_intent),
        Err(StoreError::NisarSubsetSourceSizeIdentityMismatch)
    ));
    let mut stale = valid.clone();
    stale.expected_revision = 2;
    assert!(matches!(
        store.freeze_nisar_subset_source_size_v2(&stale),
        Err(StoreError::RevisionConflict)
    ));
    let mut zero = valid.clone();
    zero.expected_size_bytes = 0;
    assert!(matches!(
        store.freeze_nisar_subset_source_size_v2(&zero),
        Err(StoreError::NisarSubsetSourceSizeInvalid)
    ));
    assert_eq!(store.load_task_record(task_id).unwrap().unwrap(), before);

    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 3,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: format!("{task_id}:pause-before-source-size"),
            occurred_at: "2026-10-16T00:00:02Z".to_owned(),
        })
        .unwrap();
    let mut stopped = valid;
    stopped.expected_revision = 4;
    assert!(matches!(
        store.freeze_nisar_subset_source_size_v2(&stopped),
        Err(StoreError::NisarSubsetSourceSizeStateInvalid)
    ));
    assert_eq!(
        store
            .load_task_record(task_id)
            .unwrap()
            .unwrap()
            .snapshot
            .candidates[0]
            .asset_access
            .as_ref()
            .and_then(|access| access.expected_size_bytes),
        None
    );
}

#[test]
fn attach_is_exact_once_revisioned_and_survives_reopen_while_preserving_active_execution() {
    let database = TempDatabase::new("attach");
    let intent = prepared_intent();
    let expected_plan = plan();
    let mut store = TaskStore::open(database.path()).unwrap();
    create_intent_task(&mut store, database.path(), "nisar-v2-attach", &intent);
    activate_intent_task(&mut store, "nisar-v2-attach");
    let persisted_status: String = Connection::open(database.path())
        .unwrap()
        .query_row(
            "SELECT status FROM task_runtime WHERE task_id = 'nisar-v2-attach'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted_status, "running");
    let scheduler_task = store
        .load_scheduler_task("nisar-v2-attach")
        .unwrap()
        .unwrap();
    assert_eq!(scheduler_task.runtime.status, TaskLifecycleStatus::Running);
    assert_eq!(scheduler_task.runtime.active_item_ids, [ITEM_ID]);
    assert!(scheduler_task.runtime.queued_item_ids.is_empty());
    let before_batch = store.load_search_batch("nisar-source-batch").unwrap();
    let before_workbench = store.load_workbench().unwrap();

    let attached = store
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: "nisar-v2-attach".to_owned(),
            item_id: ITEM_ID.to_owned(),
            prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
            expected_revision: 3,
            occurred_at: "2026-08-29T00:01:00Z".to_owned(),
            plan: expected_plan.clone(),
        })
        .unwrap();
    assert_eq!(attached.runtime.revision, 4);
    assert_eq!(attached.runtime.status, TaskLifecycleStatus::Running);
    assert!(!attached.runtime.task_paused);
    assert_eq!(attached.runtime.active_item_ids, [ITEM_ID]);
    assert!(attached.runtime.queued_item_ids.is_empty());
    let envelope = parse_nisar_subset_plan_v2_envelope(&attached.snapshot.candidates[0].metadata)
        .unwrap()
        .unwrap();
    assert_eq!(envelope.plan().unwrap(), expected_plan);
    assert_eq!(
        envelope.prepared_intent_sha256,
        intent.canonical_intent_sha256
    );

    let retried = store
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: "nisar-v2-attach".to_owned(),
            item_id: ITEM_ID.to_owned(),
            prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
            expected_revision: 4,
            occurred_at: "2026-08-29T00:02:00Z".to_owned(),
            plan: expected_plan.clone(),
        })
        .unwrap();
    assert_eq!(retried.runtime.revision, 4);
    assert_eq!(store.list_task_events("nisar-v2-attach").unwrap().len(), 5);
    assert_eq!(
        store.load_search_batch("nisar-source-batch").unwrap(),
        before_batch
    );
    assert_eq!(store.load_workbench().unwrap(), before_workbench);
    drop(store);

    let reopened = TaskStore::open(database.path()).unwrap();
    let task = reopened
        .load_task_record("nisar-v2-attach")
        .unwrap()
        .unwrap();
    assert_eq!(task.runtime.status, TaskLifecycleStatus::Running);
    assert_eq!(task.runtime.active_item_ids, [ITEM_ID]);
    let frozen = parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[0].metadata)
        .unwrap()
        .unwrap();
    assert_eq!(
        frozen.canonical_plan_json,
        expected_plan.canonical_json().unwrap()
    );
    assert_eq!(
        frozen.canonical_semantic_plan_digest,
        expected_plan.canonical_semantic_plan_digest
    );
}

#[test]
fn stale_retry_and_different_plan_never_replace_frozen_plan() {
    let database = TempDatabase::new("conflicts");
    let intent = prepared_intent();
    let first_plan = plan();
    let mut store = TaskStore::open(database.path()).unwrap();
    create_intent_task(&mut store, database.path(), "nisar-v2-conflicts", &intent);
    activate_intent_task(&mut store, "nisar-v2-conflicts");
    store
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: "nisar-v2-conflicts".to_owned(),
            item_id: ITEM_ID.to_owned(),
            prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
            expected_revision: 3,
            occurred_at: "2026-08-29T00:01:00Z".to_owned(),
            plan: first_plan.clone(),
        })
        .unwrap();
    assert!(matches!(
        store.attach_nisar_subset_execution_plan_v2(
            &StoreAttachNisarSubsetExecutionPlanV2Request {
                task_id: "nisar-v2-conflicts".to_owned(),
                item_id: ITEM_ID.to_owned(),
                prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
                expected_revision: 0,
                occurred_at: "2026-08-29T00:02:00Z".to_owned(),
                plan: first_plan.clone(),
            },
        ),
        Err(StoreError::RevisionConflict)
    ));
    let mut alternate_graph = graph();
    alternate_graph.evidence_sha256 = "d".repeat(64);
    let different = plan_with_graph(intent.clone(), alternate_graph);
    assert_ne!(
        different.canonical_semantic_plan_digest,
        first_plan.canonical_semantic_plan_digest
    );
    drop(store);
    let mut store = TaskStore::open(database.path()).unwrap();
    assert!(matches!(
        store.attach_nisar_subset_execution_plan_v2(
            &StoreAttachNisarSubsetExecutionPlanV2Request {
                task_id: "nisar-v2-conflicts".to_owned(),
                item_id: ITEM_ID.to_owned(),
                prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
                expected_revision: 4,
                occurred_at: "2026-08-29T00:03:00Z".to_owned(),
                plan: different,
            },
        ),
        Err(StoreError::NisarSubsetExecutionPlanAlreadyFrozenConflict)
    ));
    let task = store
        .load_task_record("nisar-v2-conflicts")
        .unwrap()
        .unwrap();
    let frozen = parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[0].metadata)
        .unwrap()
        .unwrap();
    assert_eq!(
        frozen.canonical_semantic_plan_digest,
        first_plan.canonical_semantic_plan_digest
    );
    assert_eq!(task.runtime.revision, 4);
}

#[test]
fn mismatched_plan_identity_dimensions_fail_typed_without_mutation() {
    let database = TempDatabase::new("binding");
    let intent = prepared_intent();
    let mut store = TaskStore::open(database.path()).unwrap();
    create_intent_task(&mut store, database.path(), "nisar-v2-binding", &intent);
    activate_intent_task(&mut store, "nisar-v2-binding");

    let mut product = intent.source_product_identity.clone();
    product.provider_product_id = "G9900000002-ASF".to_owned();
    let product_plan = plan_with_graph(
        prepared_intent_with(
            product,
            intent.source_asset_identity.clone(),
            intent.requested_geometry.canonical_geojson.clone(),
            intent.selected_polarizations.clone(),
        ),
        graph(),
    );
    let mut asset = intent.source_asset_identity.clone();
    asset.file_name = "OTHER_SOURCE.h5".to_owned();
    asset.stable_asset_key = format!("{ITEM_ID}::ASSET::other-science-rslc-hdf5");
    let asset_plan = plan_with_graph(
        prepared_intent_with(
            intent.source_product_identity.clone(),
            asset,
            intent.requested_geometry.canonical_geojson.clone(),
            intent.selected_polarizations.clone(),
        ),
        graph(),
    );
    let geometry_plan = plan_with_graph(
        prepared_intent_with(
            intent.source_product_identity.clone(),
            intent.source_asset_identity.clone(),
            rectangle(2.0, 2.0, 4.0, 4.0),
            intent.selected_polarizations.clone(),
        ),
        graph(),
    );
    let polarization_plan = plan_with_graph(
        prepared_intent_with(
            intent.source_product_identity.clone(),
            intent.source_asset_identity.clone(),
            intent.requested_geometry.canonical_geojson.clone(),
            vec!["HH".to_owned()],
        ),
        graph(),
    );
    let source_footprint_plan = plan_with_graph(
        prepared_intent_with_authority(
            intent.source_product_identity.clone(),
            intent.source_asset_identity.clone(),
            rectangle(0.0, 0.0, 9.0, 9.0),
            intent.requested_geometry.canonical_geojson.clone(),
            NisarSubsetFrequency::A,
            intent.selected_polarizations.clone(),
        ),
        graph(),
    );
    let frequency_plan = plan_with_graph(
        prepared_intent_with_authority(
            intent.source_product_identity.clone(),
            intent.source_asset_identity.clone(),
            intent.source_footprint_geometry.canonical_geojson.clone(),
            intent.requested_geometry.canonical_geojson.clone(),
            NisarSubsetFrequency::B,
            intent.selected_polarizations.clone(),
        ),
        graph_b(),
    );
    for candidate in [
        product_plan,
        asset_plan,
        geometry_plan,
        polarization_plan,
        source_footprint_plan,
        frequency_plan,
    ] {
        assert!(matches!(
            store.attach_nisar_subset_execution_plan_v2(
                &StoreAttachNisarSubsetExecutionPlanV2Request {
                    task_id: "nisar-v2-binding".to_owned(),
                    item_id: ITEM_ID.to_owned(),
                    prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
                    expected_revision: 3,
                    occurred_at: "2026-08-29T00:01:00Z".to_owned(),
                    plan: candidate,
                },
            ),
            Err(StoreError::NisarSubsetExecutionPlanIntentMismatch)
        ));
    }
    let task = store.load_task_record("nisar-v2-binding").unwrap().unwrap();
    assert_eq!(task.runtime.revision, 3);
    assert!(
        parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[0].metadata)
            .unwrap()
            .is_none()
    );
}

#[test]
fn ordinary_v1_explicit_task_metadata_remains_a_noop() {
    let database = TempDatabase::new("ordinary");
    let mut store = TaskStore::open(database.path()).unwrap();
    save_source(&mut store);
    let mut item = source_item();
    item.metadata
        .insert("source_item_id".to_owned(), json!(ITEM_ID));
    let task = store
        .create_paused_task_with_explicit_items(
            &task_request(database.path(), "ordinary-v1"),
            0,
            &[ITEM_ID.to_owned()],
            &[item],
        )
        .unwrap();
    assert_eq!(task.runtime.status, TaskLifecycleStatus::Paused);
    assert!(
        parse_nisar_subset_prepared_intent_v2_envelope(&task.snapshot.candidates[0].metadata,)
            .unwrap()
            .is_none()
    );
}

#[test]
fn v21_migration_allows_only_the_exact_plan_freeze_transition() {
    let database = TempDatabase::new("v21-trigger");
    let intent = prepared_intent();
    let expected_plan = plan();
    let mut store = TaskStore::open(database.path()).unwrap();
    create_intent_task(&mut store, database.path(), "nisar-v2-v21", &intent);
    activate_intent_task(&mut store, "nisar-v2-v21");
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE nisar_archived_recovery_v2;
             DROP TRIGGER task_snapshots_are_frozen;
             CREATE TRIGGER task_snapshots_are_frozen
             BEFORE UPDATE ON task_snapshots
             BEGIN
                 SELECT RAISE(ABORT, 'frozen task snapshot cannot be updated');
             END;
             DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             PRAGMA user_version = 20;",
        )
        .unwrap();
    drop(connection);

    let mut migrated = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        migrated.health().unwrap().schema_version,
        insar_store::STORE_SCHEMA_VERSION
    );
    migrated
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: "nisar-v2-v21".to_owned(),
            item_id: ITEM_ID.to_owned(),
            prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
            expected_revision: 3,
            occurred_at: "2026-08-29T00:01:00Z".to_owned(),
            plan: expected_plan,
        })
        .unwrap();
    drop(migrated);

    let connection = Connection::open(database.path()).unwrap();
    let snapshot_json: String = connection
        .query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id = 'nisar-v2-v21'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut replaced_plan: Value = serde_json::from_str(&snapshot_json).unwrap();
    replaced_plan["selected_products"][0]["metadata"][NISAR_SUBSET_PLAN_V2_ENVELOPE_METADATA_KEY]
        ["canonical_semantic_plan_digest"] = json!("f".repeat(64));
    assert!(connection
        .execute(
            "UPDATE task_snapshots SET snapshot_json = ?1 WHERE task_id = ?2",
            params![replaced_plan.to_string(), "nisar-v2-v21"],
        )
        .is_err());
    assert!(connection
        .execute(
            "UPDATE task_snapshots SET output_root = ?1 WHERE task_id = ?2",
            params!["C:/elsewhere", "nisar-v2-v21"],
        )
        .is_err());
}

#[test]
fn trigger_rejects_prepared_intent_replacement_during_first_plan_attach() {
    let database = TempDatabase::new("trigger-prepared");
    let intent = prepared_intent();
    let mut store = TaskStore::open(database.path()).unwrap();
    create_intent_task(
        &mut store,
        database.path(),
        "nisar-v2-trigger-prepared",
        &intent,
    );
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    let snapshot_json: String = connection
        .query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id = 'nisar-v2-trigger-prepared'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut tampered: Value = serde_json::from_str(&snapshot_json).unwrap();
    let plan_envelope =
        insar_store::NisarSubsetExecutionPlanEnvelopeV2::from_plan(&plan()).unwrap();
    tampered["selected_products"][0]["metadata"][NISAR_SUBSET_PLAN_V2_ENVELOPE_METADATA_KEY] =
        serde_json::to_value(plan_envelope).unwrap();
    tampered["selected_products"][0]["metadata"]
        [NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY]["canonical_intent_sha256"] =
        json!("f".repeat(64));
    assert!(connection
        .execute(
            "UPDATE task_snapshots SET snapshot_json = ?1 WHERE task_id = ?2",
            params![tampered.to_string(), "nisar-v2-trigger-prepared"],
        )
        .is_err());
}

#[test]
fn execution_anchor_cas_is_exact_stale_safe_lineage_bound_and_reopen_durable() {
    let database = TempDatabase::new("anchor-cas");
    let task_id = "nisar-v2-anchor-cas";
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), task_id);
    let before = store.load_task_record(task_id).unwrap().unwrap();

    assert_eq!(
        store
            .load_nisar_subset_execution_anchor_v2(
                task_id,
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            )
            .unwrap(),
        None
    );
    let genesis_json = r#"{"generation":0,"state":"pending"}"#.to_owned();
    assert!(store
        .compare_and_swap_nisar_subset_execution_anchor_v2(&anchor_request(
            task_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            'a',
            None,
            'b',
            genesis_json.clone(),
        ))
        .unwrap());
    let expected_genesis = StoredNisarSubsetExecutionAnchorV2 {
        task_id: task_id.to_owned(),
        item_id: ITEM_ID.to_owned(),
        anchor_kind: NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND.to_owned(),
        lineage_key_sha256: "a".repeat(64),
        record_sha256: "b".repeat(64),
        canonical_json: genesis_json,
    };
    assert_eq!(
        store
            .load_nisar_subset_execution_anchor_v2(
                task_id,
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            )
            .unwrap(),
        Some(expected_genesis.clone())
    );

    assert!(!store
        .compare_and_swap_nisar_subset_execution_anchor_v2(&anchor_request(
            task_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            'a',
            Some("c".repeat(64)),
            'd',
            r#"{"generation":1,"state":"committed"}"#.to_owned(),
        ))
        .unwrap());
    assert_eq!(
        store
            .load_nisar_subset_execution_anchor_v2(
                task_id,
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            )
            .unwrap(),
        Some(expected_genesis.clone())
    );
    assert!(matches!(
        store.compare_and_swap_nisar_subset_execution_anchor_v2(&anchor_request(
            task_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            'e',
            Some("b".repeat(64)),
            'd',
            r#"{"generation":1,"state":"committed"}"#.to_owned(),
        )),
        Err(StoreError::NisarSubsetExecutionAnchorLineageConflict)
    ));

    let committed_json = r#"{"generation":1,"state":"committed"}"#.to_owned();
    assert!(store
        .compare_and_swap_nisar_subset_execution_anchor_v2(&anchor_request(
            task_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            'a',
            Some("b".repeat(64)),
            'd',
            committed_json.clone(),
        ))
        .unwrap());
    assert_eq!(store.load_task_record(task_id).unwrap().unwrap(), before);
    drop(store);

    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened
            .load_nisar_subset_execution_anchor_v2(
                task_id,
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            )
            .unwrap(),
        Some(StoredNisarSubsetExecutionAnchorV2 {
            record_sha256: "d".repeat(64),
            canonical_json: committed_json,
            ..expected_genesis
        })
    );
    assert_eq!(reopened.load_task_record(task_id).unwrap().unwrap(), before);
}

#[test]
fn execution_anchor_roundtrips_large_science_pending_payload_and_all_writer_states() {
    let database = TempDatabase::new("anchor-roundtrip");
    let task_id = "nisar-v2-anchor-roundtrip";
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), task_id);

    let large_science_json = serde_json::to_string(&json!({
        "generation": 0,
        "pending_checkpoint_canonical_json": "x".repeat(1024 * 1024),
        "state": "pending"
    }))
    .unwrap();
    assert!(store
        .compare_and_swap_nisar_subset_execution_anchor_v2(&anchor_request(
            task_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            '1',
            None,
            '2',
            large_science_json.clone(),
        ))
        .unwrap());
    assert_eq!(
        store
            .load_nisar_subset_execution_anchor_v2(
                task_id,
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            )
            .unwrap()
            .unwrap()
            .canonical_json,
        large_science_json
    );

    let states = [
        "candidate_building",
        "candidate_validated",
        "artifact_promoted",
        "complete",
    ];
    let mut expected = None;
    for (index, state) in states.into_iter().enumerate() {
        let record = char::from_digit((index + 3) as u32, 10).unwrap();
        let canonical_json = format!(r#"{{"state":"{state}"}}"#);
        assert!(store
            .compare_and_swap_nisar_subset_execution_anchor_v2(&anchor_request(
                task_id,
                NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
                'f',
                expected.take(),
                record,
                canonical_json.clone(),
            ))
            .unwrap());
        let loaded = store
            .load_nisar_subset_execution_anchor_v2(
                task_id,
                ITEM_ID,
                NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
            )
            .unwrap()
            .unwrap();
        assert_eq!(loaded.canonical_json, canonical_json);
        assert_eq!(loaded.record_sha256, record.to_string().repeat(64));
        expected = Some(loaded.record_sha256);
    }
    drop(store);

    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened
            .load_nisar_subset_execution_anchor_v2(
                task_id,
                ITEM_ID,
                NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
            )
            .unwrap()
            .unwrap()
            .canonical_json,
        r#"{"state":"complete"}"#
    );
}

#[test]
fn v21_open_existing_migrates_execution_anchor_schema_and_preserves_task_authority() {
    let database = TempDatabase::new("v21-anchor-migration");
    let task_id = "nisar-v2-v21-anchor-migration";
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), task_id);
    let before = store.load_task_record(task_id).unwrap().unwrap();
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE nisar_archived_recovery_v2;
             DROP TABLE nisar_subset_execution_anchors_v2;
             DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             PRAGMA user_version = 21;",
        )
        .unwrap();
    drop(connection);

    let mut reopened = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        reopened.health().unwrap().schema_version,
        insar_store::STORE_SCHEMA_VERSION
    );
    assert_eq!(reopened.load_task_record(task_id).unwrap().unwrap(), before);
    assert!(reopened
        .compare_and_swap_nisar_subset_execution_anchor_v2(&anchor_request(
            task_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            'a',
            None,
            'b',
            r#"{"state":"pending"}"#.to_owned(),
        ))
        .unwrap());
    drop(reopened);

    let reopened = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        reopened
            .load_nisar_subset_execution_anchor_v2(
                task_id,
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            )
            .unwrap()
            .unwrap()
            .canonical_json,
        r#"{"state":"pending"}"#
    );
}

#[test]
fn multi_item_prepared_intents_require_one_unique_source_per_task_item() {
    let database = TempDatabase::new("multi-item-source-identity");
    let intent = prepared_intent();
    let mut store = TaskStore::open(database.path()).unwrap();
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "nisar-source-batch".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        created_at: "2026-09-04T00:00:00Z".to_owned(),
        aoi_name: "Batch AOI".to_owned(),
        aoi_geojson: Some(rectangle(0.0, 0.0, 10.0, 10.0)),
        query: Map::new(),
        search_stats: None,
        candidates: vec![source_item(), source_item_two()],
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id,
                checked_item_ids: vec![ITEM_ID.to_owned(), ITEM_ID_2.to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    let first = task_item(&intent);
    let mut duplicate_source = task_item(&intent);
    duplicate_source.item_id = ITEM_ID_2.to_owned();
    let mut request = task_request(database.path(), "nisar-v2-duplicate-source");
    request.selected_item_ids = vec![ITEM_ID.to_owned(), ITEM_ID_2.to_owned()];
    request.checked_item_ids = request.selected_item_ids.clone();

    assert!(matches!(
        store.create_paused_task_with_explicit_items(
            &request,
            0,
            &[ITEM_ID.to_owned(), ITEM_ID_2.to_owned()],
            &[first, duplicate_source],
        ),
        Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch)
    ));
    assert!(store
        .load_task_record("nisar-v2-duplicate-source")
        .unwrap()
        .is_none());
}

#[test]
fn multi_item_task_freezes_each_plan_by_exact_item_without_changing_other_items() {
    let database = TempDatabase::new("multi-item-plan");
    let intent_one = prepared_intent();
    let intent_two = prepared_intent_two();
    let plan_one = plan_with_graph(intent_one.clone(), graph());
    let plan_two = plan_with_graph(intent_two.clone(), graph());
    let source_one = source_item();
    let source_two = source_item_two();
    let mut store = TaskStore::open(database.path()).unwrap();
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "nisar-source-batch".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        created_at: "2026-09-04T00:00:00Z".to_owned(),
        aoi_name: "Batch AOI".to_owned(),
        aoi_geojson: Some(rectangle(0.0, 0.0, 10.0, 10.0)),
        query: Map::new(),
        search_stats: None,
        candidates: vec![source_one.clone(), source_two.clone()],
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id,
                checked_item_ids: vec![ITEM_ID.to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    let mut request = task_request(database.path(), "nisar-v2-multi-item");
    request.selected_item_ids = vec![ITEM_ID.to_owned(), ITEM_ID_2.to_owned()];
    request.checked_item_ids = request.selected_item_ids.clone();
    let task_items = vec![
        task_item_for(&intent_one, source_one),
        task_item_for(&intent_two, source_two),
    ];

    let paused = store
        .create_paused_task_with_explicit_items(
            &request,
            0,
            &[ITEM_ID.to_owned(), ITEM_ID_2.to_owned()],
            &task_items,
        )
        .unwrap();
    assert_eq!(paused.snapshot.candidates.len(), 2);
    assert_eq!(paused.runtime.paused_item_ids, [ITEM_ID, ITEM_ID_2]);
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: request.task_id.clone(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeTask,
                item_ids: vec![],
            },
            event_id: "nisar-v2-multi-item:resume".to_owned(),
            occurred_at: "2026-09-04T00:00:10Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: request.task_id.clone(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeItems,
                item_ids: vec![ITEM_ID.to_owned(), ITEM_ID_2.to_owned()],
            },
            event_id: "nisar-v2-multi-item:resume-items".to_owned(),
            occurred_at: "2026-09-04T00:00:20Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: request.task_id.clone(),
            expected_revision: 2,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec![ITEM_ID.to_owned(), ITEM_ID_2.to_owned()],
            },
            event_id: "nisar-v2-multi-item:activate-items".to_owned(),
            occurred_at: "2026-09-04T00:00:30Z".to_owned(),
        })
        .unwrap();
    let before_second = store
        .load_task_record(&request.task_id)
        .unwrap()
        .unwrap()
        .snapshot
        .candidates[1]
        .metadata
        .clone();

    let after_one = store
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: request.task_id.clone(),
            item_id: ITEM_ID.to_owned(),
            prepared_intent_sha256: intent_one.canonical_intent_sha256,
            expected_revision: 3,
            occurred_at: "2026-09-04T00:00:40Z".to_owned(),
            plan: plan_one.clone(),
        })
        .unwrap();
    assert_eq!(after_one.runtime.active_item_ids, [ITEM_ID, ITEM_ID_2]);
    assert_eq!(after_one.snapshot.candidates[1].metadata, before_second);
    assert!(
        parse_nisar_subset_plan_v2_envelope(&after_one.snapshot.candidates[0].metadata)
            .unwrap()
            .is_some()
    );
    assert!(
        parse_nisar_subset_plan_v2_envelope(&after_one.snapshot.candidates[1].metadata)
            .unwrap()
            .is_none()
    );

    let after_two = store
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: request.task_id.clone(),
            item_id: ITEM_ID_2.to_owned(),
            prepared_intent_sha256: intent_two.canonical_intent_sha256,
            expected_revision: 4,
            occurred_at: "2026-09-04T00:00:50Z".to_owned(),
            plan: plan_two.clone(),
        })
        .unwrap();
    assert_eq!(after_two.runtime.revision, 5);
    assert_eq!(after_two.runtime.active_item_ids, [ITEM_ID, ITEM_ID_2]);
    drop(store);

    let reopened = TaskStore::open(database.path()).unwrap();
    let task = reopened
        .load_task_record(&request.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(task.runtime.active_item_ids, [ITEM_ID, ITEM_ID_2]);
    assert_eq!(
        parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[0].metadata)
            .unwrap()
            .unwrap()
            .plan()
            .unwrap(),
        plan_one
    );
    assert_eq!(
        parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[1].metadata)
            .unwrap()
            .unwrap()
            .plan()
            .unwrap(),
        plan_two
    );
}

fn source_item_with_size(mut item: SnapshotItem, source_bytes: u64) -> SnapshotItem {
    item.payload
        .insert("file_size_remote".to_owned(), json!(source_bytes));
    item
}

fn projected_task_totals(store: &TaskStore, task_id: &str) -> (Option<u64>, Option<u64>) {
    let task = store
        .core_restore(false)
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| task.task_id == task_id)
        .expect("task summary");
    let source_total = serde_json::to_value(&task).unwrap()["source_total_bytes"].as_u64();
    (source_total, task.total_bytes)
}

#[test]
fn appended_nisar_items_keep_complete_frozen_source_total_across_lifecycle_and_reopen() {
    let database = TempDatabase::new("appended-source-total");
    let task_id = "nisar-v2-appended-source-total";
    let intent_one = prepared_intent();
    let intent_two = prepared_intent_two();
    let intent_three = prepared_intent_three();
    let source_one = source_item_with_size(source_item(), 1_000);
    let source_two = source_item_with_size(source_item_two(), 2_000);
    let source_three = source_item_with_size(source_item_three(), 3_000);
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "nisar-source-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-09-05T00:00:00Z".to_owned(),
                aoi_name: "Append source total AOI".to_owned(),
                aoi_geojson: Some(rectangle(0.0, 0.0, 10.0, 10.0)),
                query: Map::new(),
                search_stats: None,
                candidates: vec![source_one.clone(), source_two.clone(), source_three.clone()],
            },
            workbench: SearchWorkbenchState {
                batch_id: "nisar-source-batch".to_owned(),
                checked_item_ids: vec![ITEM_ID.to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    let request = task_request(database.path(), task_id);
    store
        .create_paused_task_with_explicit_items(
            &request,
            0,
            &[ITEM_ID.to_owned()],
            &[task_item_for(&intent_one, source_one)],
        )
        .unwrap();
    assert_eq!(
        projected_task_totals(&store, task_id),
        (Some(1_000), Some(1_000))
    );

    store
        .append_explicit_task_items(&StoreAppendExplicitTaskItemsRequest {
            task_id: task_id.to_owned(),
            expected_revision: 0,
            event_id: "nisar-v2-appended-source-total:append-two".to_owned(),
            occurred_at: "2026-09-05T00:00:10Z".to_owned(),
            source_item_ids: vec![ITEM_ID_2.to_owned()],
            task_items: vec![task_item_for(&intent_two, source_two)],
        })
        .unwrap();
    assert_eq!(projected_task_totals(&store, task_id), (Some(3_000), None));

    for (expected_revision, kind, item_ids, event_id) in [
        (1, TaskStateCommandKind::ResumeTask, vec![], "resume-task"),
        (
            2,
            TaskStateCommandKind::ResumeItems,
            vec![ITEM_ID.to_owned()],
            "resume-first",
        ),
    ] {
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision,
                command: StoreTaskCommand { kind, item_ids },
                event_id: format!("nisar-v2-appended-source-total:{event_id}"),
                occurred_at: "2026-09-05T00:00:20Z".to_owned(),
            })
            .unwrap();
    }
    store
        .save_download_result(
            &DownloadItemResult {
                task_id: task_id.to_owned(),
                item_id: ITEM_ID.to_owned(),
                outcome: DownloadOutcome::Completed,
                destination: database.path().with_extension("output-one.h5"),
                bytes_written: 100,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-09-05T00:00:30Z".to_owned(),
            },
            0,
        )
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 3,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec![ITEM_ID.to_owned()],
            },
            event_id: "nisar-v2-appended-source-total:complete-first".to_owned(),
            occurred_at: "2026-09-05T00:00:30Z".to_owned(),
        })
        .unwrap();
    assert_eq!(projected_task_totals(&store, task_id), (Some(3_000), None));

    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 4,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "nisar-v2-appended-source-total:pause".to_owned(),
            occurred_at: "2026-09-05T00:00:40Z".to_owned(),
        })
        .unwrap();
    store
        .append_explicit_task_items(&StoreAppendExplicitTaskItemsRequest {
            task_id: task_id.to_owned(),
            expected_revision: 5,
            event_id: "nisar-v2-appended-source-total:append-three".to_owned(),
            occurred_at: "2026-09-05T00:00:50Z".to_owned(),
            source_item_ids: vec![ITEM_ID_3.to_owned()],
            task_items: vec![task_item_for(&intent_three, source_three)],
        })
        .unwrap();
    assert_eq!(projected_task_totals(&store, task_id), (Some(6_000), None));
    drop(store);

    let mut reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        projected_task_totals(&reopened, task_id),
        (Some(6_000), None)
    );
    reopened
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 6,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeTask,
                item_ids: vec![],
            },
            event_id: "nisar-v2-appended-source-total:resume-after-reopen".to_owned(),
            occurred_at: "2026-09-05T00:01:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(
        projected_task_totals(&reopened, task_id),
        (Some(6_000), None)
    );
    let expected_status = reopened
        .core_restore(false)
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| task.task_id == task_id)
        .unwrap()
        .status;
    drop(reopened);

    let connection = Connection::open(database.path()).unwrap();
    let (added_revision, position, added_at): (i64, i64, String) = connection
        .query_row(
            "SELECT added_revision, position, added_at FROM task_membership_additions \
             WHERE task_id=?1 AND item_id=?2",
            params![task_id, ITEM_ID_3],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let mismatched_item_json: String = connection
        .query_row(
            "SELECT item_json FROM task_membership_additions WHERE task_id=?1 AND item_id=?2",
            params![task_id, ITEM_ID_2],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute(
            "DELETE FROM task_membership_additions WHERE task_id=?1 AND item_id=?2",
            params![task_id, ITEM_ID_3],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_membership_additions(
                 task_id,item_id,added_revision,position,added_at,item_json
             ) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                task_id,
                ITEM_ID_3,
                added_revision,
                position,
                added_at,
                mismatched_item_json
            ],
        )
        .unwrap();
    drop(connection);

    let corrupted = TaskStore::open(database.path()).unwrap();
    let restored = corrupted.core_restore(false).unwrap();
    let task = restored
        .tasks
        .iter()
        .find(|task| task.task_id == task_id)
        .expect("invalid display-only addition must not remove the task projection");
    assert_eq!(task.status, expected_status);
    assert_eq!(task.source_total_bytes, None);
    assert_eq!(task.total_bytes, None);
    assert!(restored
        .legacy_task_warnings
        .iter()
        .all(|warning| warning.task_id != task_id));
    assert!(corrupted.load_scheduler_task(task_id).is_err());
}
