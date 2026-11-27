use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, FrozenTaskSnapshot, FrozenTaskSnapshotV3,
    SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest, StoreRestoreRequest,
    StoreSearchBatchRequest,
};
use insar_store::{
    decode_any_snapshot, decode_v2_only_snapshot, encode_task_snapshot,
    validate_task_snapshot_row_envelope, CompactTaskSnapshotV2, StoreError, StoredTaskSnapshot,
    TaskSnapshotRowEnvelope, TaskSnapshotRowEnvelopeIssue, TaskStore, V3SnapshotValidationIssue,
};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Map, Value};

const CODEC_FIXTURES: [&str; 15] = [
    "valid-legacy-v2.json",
    "valid-compact-v2.json",
    "valid-v3-single.json",
    "valid-v3-shared-poeorb.json",
    "malformed-json.json",
    "v3-structural-invalid.json",
    "v3-graph-invalid.json",
    "persistence-version-1.json",
    "persistence-version-4.json",
    "persistence-version-noninteger.json",
    "v3-v2-reader-rejected.json",
    "v3-round-trip.json",
    "legacy-v2-serde-shape-invalid.json",
    "compact-v2-serde-shape-invalid.json",
    "compact-v2-business-invalid.json",
];

const ROW_ENVELOPE_FIXTURES: [&str; 6] = [
    "row-envelope-task-id-mismatch.json",
    "row-envelope-batch-id-mismatch.json",
    "row-envelope-provider-id-mismatch.json",
    "row-envelope-created-at-mismatch.json",
    "row-envelope-output-root-mismatch.json",
    "row-envelope-layout-mismatch.json",
];

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after the Unix epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-snapshot-codec-{}-{nonce}-{seq}.sqlite3",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[test]
fn valid_legacy_v2_projects_trusted_selected_source_total() {
    let database = TempDatabase::new();
    {
        let mut store = TaskStore::open(database.path()).expect("fixture Store opens");
        seed_v2_task(&mut store);
    }
    let mut legacy = legacy_snapshot();
    legacy.candidates[0]
        .payload
        .insert("file_size_remote".to_owned(), json!(14_252_244_992_u64));
    replace_stored_snapshot(
        &database,
        &serde_json::to_string(&legacy).expect("legacy snapshot serializes"),
    );

    let store = TaskStore::open(database.path()).expect("fixture Store reopens");
    let task = store
        .core_restore(false)
        .expect("legacy task restores")
        .tasks
        .into_iter()
        .find(|task| task.task_id == "codec-v2-task")
        .expect("compatible legacy task summary");
    assert_eq!(
        serde_json::to_value(task).expect("summary serializes")["source_total_bytes"],
        json!(14_252_244_992_u64)
    );
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().expect("value is an object").clone()
}

fn object<'a>(value: &'a Value, label: &str) -> &'a Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{label} is an object"))
}

fn string_field<'a>(value: &'a Map<String, Value>, field: &str) -> &'a str {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{field} is a string"))
}

fn value_field<'a>(value: &'a Map<String, Value>, field: &str) -> &'a Value {
    value
        .get(field)
        .unwrap_or_else(|| panic!("{field} is present"))
}

fn snapshot_item(item_id: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_string(),
        provider_id: "asf.sentinel1".to_string(),
        display_name: format!("fixture {item_id}"),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn legacy_snapshot() -> FrozenTaskSnapshot {
    FrozenTaskSnapshot {
        schema_version: 1,
        task_id: "codec-v2-task".to_string(),
        batch_id: "codec-v2-batch".to_string(),
        provider_id: "asf.sentinel1".to_string(),
        created_at: "2024-01-15T00:00:00Z".to_string(),
        aoi_name: "codec fixture".to_string(),
        aoi_geojson: Some(map(json!({
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]]
        }))),
        query: map(json!({"product_type": "SLC"})),
        candidates: vec![snapshot_item("codec-v2-source")],
        initial_selected_item_ids: vec!["codec-v2-source".to_string()],
        output_root: PathBuf::from("codec-output"),
        layout: DownloadLayout::Sentinel1Slc,
        operation: None,
        orbit_association: None,
    }
}

fn compact_snapshot(legacy: &FrozenTaskSnapshot) -> CompactTaskSnapshotV2 {
    CompactTaskSnapshotV2 {
        persistence_version: 2,
        schema_version: legacy.schema_version,
        task_id: legacy.task_id.clone(),
        batch_id: legacy.batch_id.clone(),
        provider_id: legacy.provider_id.clone(),
        created_at: legacy.created_at.clone(),
        aoi_name: legacy.aoi_name.clone(),
        aoi_geojson: legacy.aoi_geojson.clone(),
        query: legacy.query.clone(),
        selected_products: legacy.candidates.clone(),
        source_item_ids: Vec::new(),
        initial_selected_item_ids: legacy.initial_selected_item_ids.clone(),
        output_root: legacy.output_root.clone(),
        layout: legacy.layout.clone(),
        operation: legacy.operation.clone(),
        orbit_association: legacy.orbit_association.clone(),
    }
}

fn graph_fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/m1-b1b-transfer-graph")
        .join(name)
}

fn codec_fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/m1-b2-store-snapshot-codec")
        .join(name)
}

fn read_json(path: PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("fixture is readable"))
        .expect("fixture is JSON")
}

fn graph_payload(name: &str) -> Value {
    read_json(graph_fixture_path(name))
        .get("payload")
        .cloned()
        .expect("graph fixture has a payload")
}

fn v3_snapshot() -> FrozenTaskSnapshotV3 {
    serde_json::from_value(graph_payload("single-sentinel1-poeorb-valid.json"))
        .expect("single graph fixture is a V3 snapshot")
}

fn codec_fixture(name: &str) -> Value {
    read_json(codec_fixture_path(name))
}

fn base_snapshot_value(base: &str) -> Value {
    match base {
        "legacy_v2" => serde_json::to_value(legacy_snapshot()).expect("legacy V2 serializes"),
        "compact_v2" => serde_json::to_value(compact_snapshot(&legacy_snapshot()))
            .expect("compact V2 serializes"),
        "v3_single" => graph_payload("single-sentinel1-poeorb-valid.json"),
        "v3_shared" => graph_payload("multiple-sentinel1-shared-poeorb-valid.json"),
        other => panic!("unknown codec fixture base: {other}"),
    }
}

fn set_value(payload: &mut Value, path: &str, value: Value) {
    *payload
        .pointer_mut(path)
        .unwrap_or_else(|| panic!("fixture mutation path exists: {path}")) = value;
}

fn append_copy(payload: &mut Value, from: &str, path: &str) {
    let copied = payload
        .pointer(from)
        .unwrap_or_else(|| panic!("fixture copy source exists: {from}"))
        .clone();
    payload
        .pointer_mut(path)
        .unwrap_or_else(|| panic!("fixture copy target exists: {path}"))
        .as_array_mut()
        .unwrap_or_else(|| panic!("fixture copy target is an array: {path}"))
        .push(copied);
}

fn apply_mutations(payload: &mut Value, fixture: &Value) {
    let Some(mutations) = fixture.get("mutations") else {
        return;
    };
    for mutation in mutations
        .as_array()
        .expect("fixture mutations are an array")
    {
        let mutation = object(mutation, "fixture mutation");
        match string_field(mutation, "op") {
            "set" => set_value(
                payload,
                string_field(mutation, "path"),
                value_field(mutation, "value").clone(),
            ),
            "append_copy" => append_copy(
                payload,
                string_field(mutation, "from"),
                string_field(mutation, "path"),
            ),
            other => panic!("unknown fixture mutation: {other}"),
        }
    }
}

fn codec_fixture_snapshot_json(fixture: &Value) -> String {
    let fixture_object = object(fixture, "codec fixture");
    if let Some(raw_file) = fixture_object.get("raw_file").and_then(Value::as_str) {
        return fs::read_to_string(codec_fixture_path(raw_file)).expect("raw fixture is readable");
    }
    if let Some(snapshot_json) = fixture_object.get("snapshot_json").and_then(Value::as_str) {
        return snapshot_json.to_string();
    }
    let mut payload = base_snapshot_value(string_field(fixture_object, "base"));
    apply_mutations(&mut payload, fixture);
    serde_json::to_string(&payload).expect("fixture payload serializes")
}

fn expected_v3_issue(value: &str) -> V3SnapshotValidationIssue {
    match value {
        "structural" => V3SnapshotValidationIssue::Structural,
        "graph" => V3SnapshotValidationIssue::Graph,
        other => panic!("unknown V3 validation issue: {other}"),
    }
}

fn assert_codec_outcome(
    result: Result<StoredTaskSnapshot, StoreError>,
    expected: &Value,
    boundary: &str,
) {
    let expected = object(expected, "codec expectation");
    if let Some(outcome) = expected.get("outcome").and_then(Value::as_str) {
        let snapshot =
            result.unwrap_or_else(|error| panic!("{boundary} unexpectedly failed: {error}"));
        match outcome {
            "legacy_v2" => assert!(matches!(snapshot, StoredTaskSnapshot::LegacyV2(_))),
            "compact_v2" => assert!(matches!(snapshot, StoredTaskSnapshot::CompactV2(_))),
            "v3" => assert!(matches!(snapshot, StoredTaskSnapshot::V3(_))),
            other => panic!("unknown codec outcome: {other}"),
        }
        return;
    }

    let error = result.expect_err("fixture expects a codec error");
    assert_eq!(
        error.code(),
        string_field(expected, "error_code"),
        "{boundary} returns the expected safe code"
    );
    if let Some(issue) = expected.get("v3_validation_issue").and_then(Value::as_str) {
        assert_eq!(
            error.v3_snapshot_validation_issue(),
            Some(expected_v3_issue(issue)),
            "{boundary} preserves the V3 validation subtype"
        );
    }
}

fn v2_batch() -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: "codec-v2-batch".to_string(),
        provider_id: "asf.sentinel1".to_string(),
        created_at: "2024-01-15T00:00:00Z".to_string(),
        aoi_name: "codec fixture".to_string(),
        aoi_geojson: Some(map(json!({
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]]
        }))),
        query: map(json!({"product_type": "SLC"})),
        search_stats: None,
        candidates: vec![snapshot_item("codec-v2-source")],
    }
}

fn seed_v2_task(store: &mut TaskStore) {
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: v2_batch(),
            workbench: SearchWorkbenchState {
                batch_id: "codec-v2-batch".to_string(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: false,
        })
        .expect("V2 search batch is saved");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "codec-v2-task".to_string(),
            batch_id: "codec-v2-batch".to_string(),
            selected_item_ids: vec!["codec-v2-source".to_string()],
            checked_item_ids: vec!["codec-v2-source".to_string()],
            page_index: 0,
            page_size: 24,
            output_root: PathBuf::from("codec-output"),
            layout: DownloadLayout::Sentinel1Slc,
            concurrency: 2,
            created_at: "2024-01-15T00:00:00Z".to_string(),
        })
        .expect("V2 task is saved");
}

fn replace_stored_snapshot(database: &TempDatabase, snapshot_json: &str) {
    let connection = Connection::open(database.path()).expect("fixture database opens");
    let (
        status,
        revision,
        runtime_updated_at,
        state_json,
        checked_item_ids_json,
        page_index,
        page_size,
        view_updated_at,
    ): (String, i64, String, String, String, i64, i64, String) = connection
        .query_row(
            "SELECT r.status, r.revision, r.updated_at, r.state_json,                     v.checked_item_ids_json, v.page_index, v.page_size, v.updated_at              FROM task_runtime r JOIN task_view_state v ON v.task_id = r.task_id              WHERE r.task_id = 'codec-v2-task'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .expect("fixture V2 runtime row is readable");
    connection
        .execute(
            "DELETE FROM task_snapshots WHERE task_id = 'codec-v2-task'",
            [],
        )
        .expect("fixture V2 task row is removable");
    connection
        .execute(
            "INSERT INTO task_snapshots(                task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json             ) VALUES ('codec-v2-task', 'codec-v2-batch', 'asf.sentinel1',                        '2024-01-15T00:00:00Z', 'codec-output', 'sentinel1_slc', ?1)",
            [snapshot_json],
        )
        .expect("fixture snapshot row inserts");
    connection
        .execute(
            "INSERT INTO task_runtime(task_id, status, revision, updated_at, state_json)              VALUES ('codec-v2-task', ?1, ?2, ?3, ?4)",
            params![status, revision, runtime_updated_at, state_json],
        )
        .expect("fixture runtime row inserts");
    connection
        .execute(
            "INSERT INTO task_view_state(                task_id, checked_item_ids_json, page_index, page_size, updated_at             ) VALUES ('codec-v2-task', ?1, ?2, ?3, ?4)",
            params![checked_item_ids_json, page_index, page_size, view_updated_at],
        )
        .expect("fixture task view row inserts");
}

enum BusinessReader {
    LoadTask,
    LoadSchedulerTask,
    LoadSchedulerTasks,
    LoadDownloadSnapshot,
    Restore,
}

impl BusinessReader {
    fn name(&self) -> &'static str {
        match self {
            Self::LoadTask => "TaskStore::load_task",
            Self::LoadSchedulerTask => "TaskStore::load_scheduler_task",
            Self::LoadSchedulerTasks => "TaskStore::load_scheduler_tasks",
            Self::LoadDownloadSnapshot => "TaskStore::load_download_snapshot",
            Self::Restore => "TaskStore::restore(V2-only)",
        }
    }

    fn read(&self, store: &TaskStore) -> Result<(), StoreError> {
        match self {
            Self::LoadTask => store.load_task("codec-v2-task").map(|_| ()),
            Self::LoadSchedulerTask => store.load_scheduler_task("codec-v2-task").map(|_| ()),
            Self::LoadSchedulerTasks => store.load_scheduler_tasks().map(|_| ()),
            Self::LoadDownloadSnapshot => store.load_download_snapshot("codec-v2-task").map(|_| ()),
            Self::Restore => store
                .restore(&StoreRestoreRequest {
                    include_terminal: false,
                })
                .map(|_| ()),
        }
    }
}

fn assert_business_reader_outcome(snapshot_json: &str, expected: &Value, fixture_name: &str) {
    let expected = object(expected, "business reader expectation");
    for reader in [
        BusinessReader::LoadTask,
        BusinessReader::LoadSchedulerTask,
        BusinessReader::LoadSchedulerTasks,
        BusinessReader::LoadDownloadSnapshot,
        BusinessReader::Restore,
    ] {
        let database = TempDatabase::new();
        {
            let mut store = TaskStore::open(database.path()).expect("fixture Store opens");
            seed_v2_task(&mut store);
        }
        replace_stored_snapshot(&database, snapshot_json);
        let store = TaskStore::open(database.path()).expect("fixture Store reopens");
        let result = reader.read(&store);

        if expected.get("outcome").and_then(Value::as_str) == Some("accepted") {
            result.unwrap_or_else(|error| {
                panic!(
                    "{fixture_name} unexpectedly fails through {}: {error}",
                    reader.name()
                )
            });
        } else {
            let error = match result {
                Ok(())
                    if matches!(
                        reader,
                        BusinessReader::LoadSchedulerTask | BusinessReader::LoadSchedulerTasks
                    ) =>
                {
                    // UA0 isolates invalid/degraded rows from scheduler admission
                    // instead of allowing one historical row to poison the batch.
                    continue;
                }
                Ok(()) => panic!(
                    "{fixture_name} unexpectedly succeeds through {}",
                    reader.name()
                ),
                Err(error) => error,
            };
            assert_eq!(
                error.code(),
                string_field(expected, "error_code"),
                "{fixture_name} through {}",
                reader.name()
            );
        }
    }
}

fn expected_row_envelope_issue(value: &str) -> TaskSnapshotRowEnvelopeIssue {
    match value {
        "task_id" => TaskSnapshotRowEnvelopeIssue::TaskId,
        "batch_id" => TaskSnapshotRowEnvelopeIssue::BatchId,
        "provider_id" => TaskSnapshotRowEnvelopeIssue::ProviderId,
        "created_at" => TaskSnapshotRowEnvelopeIssue::CreatedAt,
        "output_root" => TaskSnapshotRowEnvelopeIssue::OutputRoot,
        "layout" => TaskSnapshotRowEnvelopeIssue::Layout,
        other => panic!("unknown row envelope issue: {other}"),
    }
}

fn layout_text(layout: &DownloadLayout) -> String {
    serde_json::to_value(layout)
        .expect("layout serializes")
        .as_str()
        .expect("layout serializes to a string")
        .to_string()
}

#[test]
fn codec_fixtures_cover_version_dispatch_and_safe_v3_subtypes() {
    for name in CODEC_FIXTURES {
        let fixture = codec_fixture(name);
        let expected = object(
            fixture.get("expected").expect("fixture has expectations"),
            "fixture expected",
        );
        let snapshot_json = codec_fixture_snapshot_json(&fixture);

        if let Some(expectation) = expected.get("decode_any") {
            assert_codec_outcome(
                decode_any_snapshot(&snapshot_json),
                expectation,
                &format!("{name}: decode_any_snapshot"),
            );
        }
        if let Some(expectation) = expected.get("v2_reader") {
            assert_codec_outcome(
                decode_v2_only_snapshot(&snapshot_json),
                expectation,
                &format!("{name}: decode_v2_only_snapshot"),
            );
        }
        if let Some(expectation) = expected.get("round_trip") {
            let decoded =
                decode_any_snapshot(&snapshot_json).expect("round-trip fixture first decodes");
            assert_codec_outcome(
                decode_any_snapshot(
                    &encode_task_snapshot(&decoded).expect("round-trip fixture encodes"),
                ),
                expectation,
                "V3 encode/decode round trip",
            );
        }
    }
}

#[test]
fn v2_business_readers_keep_their_historical_error_code_matrix() {
    for name in CODEC_FIXTURES {
        let fixture = codec_fixture(name);
        let expected = object(
            fixture.get("expected").expect("fixture has expectations"),
            "fixture expected",
        );
        let Some(expectation) = expected.get("business_readers") else {
            continue;
        };
        assert_business_reader_outcome(&codec_fixture_snapshot_json(&fixture), expectation, name);
    }
}

#[test]
fn v3_codec_errors_preserve_safe_subtypes_without_external_values() {
    let marker = "UNTRUSTED-SOURCE-ID://fixture.invalid/path?token=fixture";
    let mut graph_invalid = v3_snapshot();
    graph_invalid.transfer_items[0].transfer_item_id = marker.to_string();
    graph_invalid.transfer_items[1].transfer_item_id = marker.to_string();

    let error = decode_any_snapshot(
        &serde_json::to_string(&graph_invalid).expect("graph-invalid snapshot serializes"),
    )
    .expect_err("graph-invalid V3 snapshot is rejected");
    assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    assert_eq!(
        error.v3_snapshot_validation_issue(),
        Some(V3SnapshotValidationIssue::Graph)
    );
    assert!(!error.to_string().contains(marker));
    assert!(!format!("{error:?}").contains(marker));

    let mut structural_invalid = serde_json::to_value(v3_snapshot()).expect("V3 serializes");
    structural_invalid["untrusted_fixture_field"] = Value::String(marker.to_string());
    let error = decode_any_snapshot(&structural_invalid.to_string())
        .expect_err("unknown V3 field is rejected");
    assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    assert_eq!(
        error.v3_snapshot_validation_issue(),
        Some(V3SnapshotValidationIssue::Structural)
    );
    assert!(!error.to_string().contains(marker));
    assert!(!format!("{error:?}").contains(marker));
}

#[test]
fn v3_snapshot_row_envelope_round_trip_is_rolled_back_without_task_state_residue() {
    let database = TempDatabase::new();
    let snapshot = v3_snapshot();
    {
        let mut store = TaskStore::open(database.path()).expect("fixture Store opens");
        let mut batch = v2_batch();
        batch.batch_id = snapshot.batch_id.clone();
        batch.created_at = snapshot.created_at.clone();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch,
                workbench: SearchWorkbenchState {
                    batch_id: snapshot.batch_id.clone(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
                activate_workbench: false,
            })
            .expect("V2 search batch exists for the rollback-only fixture");
    }

    let snapshot_json =
        encode_task_snapshot(&StoredTaskSnapshot::V3(snapshot.clone())).expect("V3 encodes");
    let mut connection = Connection::open(database.path()).expect("fixture database opens");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys are enabled");
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .expect("rollback-only transaction opens");
    transaction
        .execute(
            "INSERT INTO task_snapshots(                task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                snapshot.task_id,
                snapshot.batch_id,
                snapshot.provider_id,
                snapshot.created_at,
                snapshot.output_root.to_string_lossy(),
                layout_text(&snapshot.layout),
                snapshot_json,
            ],
        )
        .expect("temporary V3 snapshot row inserts");

    let (row_task_id, row_batch_id, row_provider_id, row_created_at, row_output_root, row_layout, stored_json): (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    ) = transaction
        .query_row(
            "SELECT task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json              FROM task_snapshots WHERE task_id = ?1",
            [&snapshot.task_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .expect("temporary V3 snapshot row is readable");
    let decoded = decode_any_snapshot(&stored_json).expect("stored V3 snapshot decodes");
    assert_eq!(decoded, StoredTaskSnapshot::V3(snapshot));
    let row_layout: DownloadLayout =
        serde_json::from_value(Value::String(row_layout)).expect("row layout decodes");
    let row_output_root = PathBuf::from(row_output_root);

    validate_task_snapshot_row_envelope(
        TaskSnapshotRowEnvelope {
            task_id: &row_task_id,
            batch_id: &row_batch_id,
            provider_id: &row_provider_id,
            created_at: &row_created_at,
            output_root: &row_output_root,
            layout: &row_layout,
        },
        &decoded,
    )
    .expect("row envelope matches the decoded V3 snapshot");

    let marker = "UNTRUSTED-ROW-METADATA-MARKER";
    for name in ROW_ENVELOPE_FIXTURES {
        let fixture = codec_fixture(name);
        let expected = object(
            fixture
                .get("expected")
                .expect("row fixture has expectations"),
            "row fixture expected",
        );
        let mut task_id = row_task_id.clone();
        let mut batch_id = row_batch_id.clone();
        let mut provider_id = row_provider_id.clone();
        let mut created_at = row_created_at.clone();
        let mut output_root = row_output_root.clone();
        let mut layout = row_layout.clone();
        match string_field(expected, "row_envelope_issue") {
            "task_id" => task_id = marker.to_string(),
            "batch_id" => batch_id = marker.to_string(),
            "provider_id" => provider_id = marker.to_string(),
            "created_at" => created_at = marker.to_string(),
            "output_root" => output_root = PathBuf::from(marker),
            "layout" => layout = DownloadLayout::Direct,
            other => panic!("unknown row fixture issue: {other}"),
        }
        let error = validate_task_snapshot_row_envelope(
            TaskSnapshotRowEnvelope {
                task_id: &task_id,
                batch_id: &batch_id,
                provider_id: &provider_id,
                created_at: &created_at,
                output_root: &output_root,
                layout: &layout,
            },
            &decoded,
        )
        .expect_err("mismatched row metadata is rejected");
        assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
        assert_eq!(
            error.task_snapshot_row_envelope_issue(),
            Some(expected_row_envelope_issue(string_field(
                expected,
                "row_envelope_issue"
            )))
        );
        assert!(!error.to_string().contains(marker));
        assert!(!format!("{error:?}").contains(marker));
    }

    transaction.rollback().expect("temporary V3 row rolls back");

    for table in [
        "task_snapshots",
        "task_runtime",
        "task_view_state",
        "task_events",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("task table count is readable");
        assert_eq!(count, 0, "rollback removes every {table} row");
    }
}
