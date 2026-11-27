use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    freeze_burst_task_items, stable_asset_key, AppendTaskFromSourceBatchRequest, AssetRole,
    BurstAssetPolicy, CreateTaskFromHistoricalSourceBatchRequest, CreateTaskFromWorkbenchRequest,
    DownloadLayout, FrozenSearchBatch, ProviderAsset, SearchWorkbenchState, SnapshotItem,
    StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskStateCommandKind,
};
use insar_store::{
    BurstAppendReceiptStatus, HistoricalSuccessorReceiptStatus, SourceAppendReceiptStatus,
    SourceMutationCommitStatus, SourceMutationQuery, SourceMutationStatus,
    SourceMutationTargetStatus, StoreAppendExplicitTaskItemsRequest, StoreBurstAppendIdentity,
    TaskStore,
};
use rusqlite::Connection;
use serde_json::{json, Map, Value};

const BATCH_ID: &str = "source-receipts-batch";
const CREATED_AT: &str = "2026-11-04T00:00:00Z";

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-source-receipts-{label}-{}-{nonce}.sqlite3",
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

fn candidate(id: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: id.to_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: format!("Receipt item {id}"),
        acquisition_datetime: Some("2026-11-04T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: map(json!({"dataset": "SENTINEL-1"})),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn burst_parent(id: &str) -> SnapshotItem {
    let primary_key = "burst.tiff";
    let metadata_key = "burst.xml";
    let assets = vec![
        ProviderAsset {
            asset_key: Some(primary_key.to_owned()),
            stable_asset_key: Some(stable_asset_key(id, primary_key).unwrap()),
            role: AssetRole::Primary,
            url: Some(format!("https://sentinel1-burst.asf.alaska.edu/{id}.tiff")),
            file_name: Some(format!("{id}.tiff")),
            media_type: Some("image/tiff".to_owned()),
            size_bytes: Some(4096),
            native_roles: vec!["USE SERVICE API".to_owned()],
            access_descriptor: None,
        },
        ProviderAsset {
            asset_key: Some(metadata_key.to_owned()),
            stable_asset_key: Some(stable_asset_key(id, metadata_key).unwrap()),
            role: AssetRole::Metadata,
            url: Some(format!("https://sentinel1-burst.asf.alaska.edu/{id}.xml")),
            file_name: Some(format!("{id}.xml")),
            media_type: Some("application/xml".to_owned()),
            size_bytes: Some(512),
            native_roles: vec!["USE SERVICE API".to_owned()],
            access_descriptor: None,
        },
    ];
    SnapshotItem {
        item_id: id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: id.to_owned(),
        acquisition_datetime: Some("2026-11-04T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: map(json!({
            "dataset": "SLC-BURST",
            "product_family": "BURST",
            "assets": assets,
        })),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed(database: &TempDatabase) -> TaskStore {
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: BATCH_ID.to_owned(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: CREATED_AT.to_owned(),
                aoi_name: "Receipt AOI".to_owned(),
                aoi_geojson: None,
                query: map(json!({"dataset": "SENTINEL-1"})),
                search_stats: None,
                candidates: vec![
                    candidate("ITEM-01"),
                    candidate("ITEM-02"),
                    candidate("ITEM-03"),
                    candidate("ITEM-04"),
                ],
            },
            workbench: SearchWorkbenchState {
                batch_id: BATCH_ID.to_owned(),
                checked_item_ids: vec!["ITEM-01".to_owned(), "ITEM-02".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
}

fn assert_err_contains<T>(result: Result<T, rusqlite::Error>, needle: &str) {
    let err = match result {
        Ok(_) => panic!("expected error containing {needle:?}, got Ok"),
        Err(err) => err.to_string(),
    };
    assert!(
        err.contains(needle),
        "expected error containing {needle:?}, got {err:?}"
    );
}

#[test]
fn source_append_historical_successor_and_reacquire_receipts_verify_identity_and_detect_conflicts()
{
    let database = TempDatabase::new("receipt-status");
    let mut store = seed(&database);
    let output_root = database.path().parent().unwrap().join("receipt-output");

    store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "task-active".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 0,
            exact_item_id: None,
            output_root: output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-11-04T00:01:00Z".to_owned(),
        })
        .unwrap();

    // 1. Source batch append receipt
    let append_req = AppendTaskFromSourceBatchRequest {
        task_id: "task-active".to_owned(),
        expected_task_revision: 0,
        item_ids: vec!["ITEM-03".to_owned()],
        event_id: "evt-append-1".to_owned(),
        occurred_at: "2026-11-04T00:02:00Z".to_owned(),
    };
    assert_eq!(
        store.source_append_receipt_status(&append_req).unwrap(),
        SourceAppendReceiptStatus::Unknown
    );
    store.append_task_from_source_batch(&append_req).unwrap();
    assert_eq!(
        store.source_append_receipt_status(&append_req).unwrap(),
        SourceAppendReceiptStatus::Committed
    );
    assert_eq!(
        store
            .query_source_mutation_status(SourceMutationQuery::SourceAppend(&append_req))
            .unwrap(),
        SourceMutationStatus {
            commit: SourceMutationCommitStatus::Committed,
            target: SourceMutationTargetStatus::Present,
        }
    );

    let mut append_conflict = append_req.clone();
    append_conflict.item_ids = vec!["ITEM-04".to_owned()];
    assert_eq!(
        store
            .source_append_receipt_status(&append_conflict)
            .unwrap(),
        SourceAppendReceiptStatus::Conflict
    );

    // 2. Reacquire items receipt (MarkSkipped -> ReacquireItems)
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-active".to_owned(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkSkipped,
                item_ids: vec!["ITEM-01".to_owned()],
            },
            event_id: "evt-skip-1".to_owned(),
            occurred_at: "2026-11-04T00:03:00Z".to_owned(),
        })
        .unwrap();

    let reacquire_req = StoreUpdateTaskRequest {
        task_id: "task-active".to_owned(),
        expected_revision: 2,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::ReacquireItems,
            item_ids: vec!["ITEM-01".to_owned()],
        },
        event_id: "evt-reacquire-1".to_owned(),
        occurred_at: "2026-11-04T00:04:00Z".to_owned(),
    };
    assert_eq!(
        store
            .query_source_mutation_status(SourceMutationQuery::Reacquire(&reacquire_req))
            .unwrap(),
        SourceMutationStatus {
            commit: SourceMutationCommitStatus::Unknown,
            target: SourceMutationTargetStatus::Present,
        }
    );
    store.update_task(&reacquire_req).unwrap();
    assert_eq!(
        store
            .query_source_mutation_status(SourceMutationQuery::Reacquire(&reacquire_req))
            .unwrap(),
        SourceMutationStatus {
            commit: SourceMutationCommitStatus::Committed,
            target: SourceMutationTargetStatus::Present,
        }
    );

    let mut reacquire_conflict = reacquire_req.clone();
    reacquire_conflict.occurred_at = "2026-11-04T00:04:59Z".to_owned();
    assert_eq!(
        store
            .query_source_mutation_status(SourceMutationQuery::Reacquire(&reacquire_conflict))
            .unwrap(),
        SourceMutationStatus {
            commit: SourceMutationCommitStatus::Conflict,
            target: SourceMutationTargetStatus::Present,
        }
    );

    // 3. Historical successor receipt (after moving source task to terminal Cancelled)
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-active".to_owned(),
            expected_revision: 3,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: vec![],
            },
            event_id: "evt-cancel-all".to_owned(),
            occurred_at: "2026-11-04T00:05:00Z".to_owned(),
        })
        .unwrap();

    let hist_req = CreateTaskFromHistoricalSourceBatchRequest {
        source_task_id: "task-active".to_owned(),
        expected_source_task_revision: 4,
        new_task_id: "task-successor".to_owned(),
        item_ids: vec!["ITEM-04".to_owned()],
        output_root: output_root.join("successor"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-11-04T00:06:00Z".to_owned(),
        reacquire_item_ids: vec![],
    };
    assert_eq!(
        store
            .historical_successor_receipt_status(&hist_req)
            .unwrap(),
        HistoricalSuccessorReceiptStatus::Unknown
    );
    store
        .create_task_from_historical_source_batch(&hist_req)
        .unwrap();
    assert_eq!(
        store
            .historical_successor_receipt_status(&hist_req)
            .unwrap(),
        HistoricalSuccessorReceiptStatus::Committed
    );
    assert_eq!(
        store
            .query_source_mutation_status(SourceMutationQuery::HistoricalSuccessor(&hist_req))
            .unwrap(),
        SourceMutationStatus {
            commit: SourceMutationCommitStatus::Committed,
            target: SourceMutationTargetStatus::Present,
        }
    );

    let mut hist_conflict = hist_req.clone();
    hist_conflict.concurrency = 2;
    assert_eq!(
        store
            .historical_successor_receipt_status(&hist_conflict)
            .unwrap(),
        HistoricalSuccessorReceiptStatus::Conflict
    );
}

#[test]
fn burst_append_receipt_writes_event_then_both_receipt_domains_and_reserves_slot() {
    let database = TempDatabase::new("burst-append");
    let mut store = TaskStore::open(database.path()).unwrap();
    let parent_a = burst_parent("BURST-A");
    let parent_b = burst_parent("BURST-B");
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "burst-batch".to_owned(),
                provider_id: "asf.catalog".to_owned(),
                created_at: CREATED_AT.to_owned(),
                aoi_name: "Burst AOI".to_owned(),
                aoi_geojson: None,
                query: map(json!({"dataset": "SLC-BURST"})),
                search_stats: None,
                candidates: vec![parent_a.clone(), parent_b.clone()],
            },
            workbench: SearchWorkbenchState {
                batch_id: "burst-batch".to_owned(),
                checked_item_ids: vec!["BURST-A".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();

    let initial_items = freeze_burst_task_items(&[&parent_a], BurstAssetPolicy::All).unwrap();
    let initial_ids: Vec<String> = initial_items
        .iter()
        .map(|item| item.item_id.clone())
        .collect();
    store
        .create_task_with_explicit_items(
            &StoreCreateTaskRequest {
                task_id: "burst-task".to_owned(),
                batch_id: "burst-batch".to_owned(),
                selected_item_ids: initial_ids.clone(),
                checked_item_ids: initial_ids,
                page_index: 0,
                page_size: 24,
                output_root: database.path().parent().unwrap().join("burst-out"),
                layout: DownloadLayout::Direct,
                concurrency: 2,
                created_at: "2026-11-04T00:01:00Z".to_owned(),
            },
            0,
            &["BURST-A".to_owned()],
            &initial_items,
        )
        .unwrap();

    let appended_items = freeze_burst_task_items(&[&parent_b], BurstAssetPolicy::All).unwrap();
    let identity = StoreBurstAppendIdentity {
        request_id: "req-burst-append-1".to_owned(),
        task_id: "burst-task".to_owned(),
        selected_product_ids: vec!["BURST-B".to_owned()],
        expected_task_revision: 0,
        event_id: "evt-burst-append-1".to_owned(),
        occurred_at: "2026-11-04T00:02:00Z".to_owned(),
    };
    assert_eq!(
        store.burst_append_receipt_status(&identity).unwrap(),
        BurstAppendReceiptStatus::Unknown
    );

    store
        .append_burst_items_with_receipt(
            &StoreAppendExplicitTaskItemsRequest {
                task_id: "burst-task".to_owned(),
                expected_revision: 0,
                event_id: "evt-burst-append-1".to_owned(),
                occurred_at: "2026-11-04T00:02:00Z".to_owned(),
                source_item_ids: vec!["BURST-B".to_owned()],
                task_items: appended_items,
            },
            None,
            &identity,
        )
        .unwrap();

    assert_eq!(
        store.burst_append_receipt_status(&identity).unwrap(),
        BurstAppendReceiptStatus::Committed
    );
    assert_eq!(
        store
            .query_source_mutation_status(SourceMutationQuery::BurstAppend(&identity))
            .unwrap(),
        SourceMutationStatus {
            commit: SourceMutationCommitStatus::Committed,
            target: SourceMutationTargetStatus::Present,
        }
    );

    let mut conflict = identity.clone();
    conflict.request_id = "req-burst-append-other".to_owned();
    assert_eq!(
        store.burst_append_receipt_status(&conflict).unwrap(),
        BurstAppendReceiptStatus::Conflict
    );
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    let slot: (String, String, i64) = connection
        .query_row(
            "SELECT event_id, task_id, sequence FROM source_receipt_event_slots WHERE event_id = 'evt-burst-append-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        slot,
        ("evt-burst-append-1".to_owned(), "burst-task".to_owned(), 1)
    );
}

#[test]
fn receipt_tables_and_event_slots_are_immutable_and_block_event_id_or_slot_reuse_after_pruning() {
    let database = TempDatabase::new("triggers");
    let mut store = seed(&database);
    let output_root = database.path().parent().unwrap().join("trigger-output");

    store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "task-1".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 0,
            exact_item_id: Some("ITEM-01".to_owned()),
            output_root,
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-11-04T00:01:00Z".to_owned(),
        })
        .unwrap();

    let append_req = AppendTaskFromSourceBatchRequest {
        task_id: "task-1".to_owned(),
        expected_task_revision: 0,
        item_ids: vec!["ITEM-02".to_owned()],
        event_id: "evt-reserved-append".to_owned(),
        occurred_at: "2026-11-04T00:02:00Z".to_owned(),
    };
    store.append_task_from_source_batch(&append_req).unwrap();
    drop(store);

    let connection = Connection::open(database.path()).unwrap();

    // Verify slot was automatically reserved by `source_receipt_reserve_slot`.
    let slot: (String, i64) = connection
        .query_row(
            "SELECT task_id, sequence FROM source_receipt_event_slots WHERE event_id = 'evt-reserved-append'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(slot, ("task-1".to_owned(), 1));

    // Receipts and event slots are append-only: UPDATE, DELETE, and INSERT OR REPLACE fail closed with explicit trigger messages.
    assert_err_contains(
        connection.execute(
            "DELETE FROM source_mutation_receipts WHERE operation_key = 'evt-reserved-append'",
            [],
        ),
        "source receipts must be retained",
    );
    assert_err_contains(
        connection.execute(
            "UPDATE source_mutation_receipts SET committed_revision = 99 WHERE operation_key = 'evt-reserved-append'",
            [],
        ),
        "source receipts are immutable",
    );
    assert_err_contains(
        connection.execute(
            "INSERT OR REPLACE INTO source_mutation_receipts(operation, operation_key, identity_version, salt, identity_sha256, committed_revision) VALUES ('source_batch_append', 'evt-reserved-append', 1, randomblob(32), randomblob(32), 1)",
            [],
        ),
        "source receipt identity already exists",
    );

    assert_err_contains(
        connection.execute(
            "DELETE FROM source_receipt_event_slots WHERE event_id = 'evt-reserved-append'",
            [],
        ),
        "source receipt event slots must be retained",
    );
    assert_err_contains(
        connection.execute(
            "UPDATE source_receipt_event_slots SET sequence = 99 WHERE event_id = 'evt-reserved-append'",
            [],
        ),
        "source receipt event slots are immutable",
    );
    assert_err_contains(
        connection.execute(
            "INSERT OR REPLACE INTO source_receipt_event_slots(event_id, task_id, sequence) VALUES ('evt-reserved-append', 'task-1', 99)",
            [],
        ),
        "source receipt event slot already exists",
    );

    // Even if the original event row in `task_events` is pruned, neither its `event_id`
    // nor its `(task_id, sequence)` slot can be reused by a new or updated event.
    connection
        .execute(
            "DELETE FROM task_events WHERE event_id = 'evt-reserved-append'",
            [],
        )
        .unwrap();

    // Reusing the reserved `event_id` with a different sequence is rejected.
    assert_err_contains(
        connection.execute(
            "INSERT INTO task_events(event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json)
             VALUES ('evt-reserved-append', 'task-1', 99, '2026-11-04T00:09:00Z', 'pause_task', NULL, '{}', NULL)",
            [],
        ),
        "event identity is reserved by a source receipt",
    );

    // Reusing the reserved `(task_id, sequence)` with a different `event_id` is rejected.
    assert_err_contains(
        connection.execute(
            "INSERT INTO task_events(event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json)
             VALUES ('evt-other', 'task-1', 1, '2026-11-04T00:09:00Z', 'pause_task', NULL, '{}', NULL)",
            [],
        ),
        "event identity is reserved by a source receipt",
    );
    drop(connection);

    // And the receipt still verifies as Committed after its `task_events` row was pruned!
    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        store.source_append_receipt_status(&append_req).unwrap(),
        SourceAppendReceiptStatus::Committed
    );
}

#[test]
fn schema_42_to_43_backfills_surviving_receipt_event_slots_and_conservatively_rejects_occupied_missing_event_sequence(
) {
    let database = TempDatabase::new("v42-to-v43-migration");
    let mut store = seed(&database);
    let output_root = database
        .path()
        .parent()
        .unwrap()
        .join("v42-migration-output");

    store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "task-v42".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 0,
            exact_item_id: Some("ITEM-01".to_owned()),
            output_root,
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-11-04T00:01:00Z".to_owned(),
        })
        .unwrap();

    let surviving_req = AppendTaskFromSourceBatchRequest {
        task_id: "task-v42".to_owned(),
        expected_task_revision: 0,
        item_ids: vec!["ITEM-02".to_owned()],
        event_id: "evt-surviving".to_owned(),
        occurred_at: "2026-11-04T00:02:00Z".to_owned(),
    };
    store.append_task_from_source_batch(&surviving_req).unwrap();

    let pruned_req = AppendTaskFromSourceBatchRequest {
        task_id: "task-v42".to_owned(),
        expected_task_revision: 1,
        item_ids: vec!["ITEM-03".to_owned()],
        event_id: "evt-pruned-before-v43".to_owned(),
        occurred_at: "2026-11-04T00:03:00Z".to_owned(),
    };
    store.append_task_from_source_batch(&pruned_req).unwrap();
    drop(store);

    // Reconstruct a real schema 42 state where:
    // (a) `evt-surviving` has both its receipt and its `task_events` row;
    // (b) `evt-pruned-before-v43` has its receipt at sequence 2, but its `task_events` row was deleted before upgrading to 43.
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER source_receipt_event_reservation_insert;
             DROP TRIGGER source_receipt_event_reservation_update;
             DROP TRIGGER source_receipt_no_replace;
             DROP TRIGGER source_receipt_no_delete;
             DROP TRIGGER source_receipt_no_update;
             DROP TRIGGER source_receipt_reserve_slot;
             DROP TRIGGER source_receipt_slot_no_replace;
             DROP TRIGGER source_receipt_slot_no_delete;
             DROP TRIGGER source_receipt_slot_no_update;
             DROP TABLE source_receipt_event_slots;
             DROP TABLE source_receipt_schema_marker;
             DELETE FROM task_events WHERE event_id = 'evt-pruned-before-v43';
             PRAGMA user_version = 42;",
        )
        .unwrap();
    drop(connection);

    // Upgrade from 42 -> 43.
    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    assert_eq!(
        store.source_append_receipt_status(&surviving_req).unwrap(),
        SourceAppendReceiptStatus::Committed
    );
    assert_eq!(
        store.source_append_receipt_status(&pruned_req).unwrap(),
        SourceAppendReceiptStatus::Committed
    );
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    // (a) `evt-surviving` was backfilled into `source_receipt_event_slots`.
    let surviving_slot: Option<(String, i64)> = connection
        .query_row(
            "SELECT task_id, sequence FROM source_receipt_event_slots WHERE event_id = 'evt-surviving'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    assert_eq!(surviving_slot, Some(("task-v42".to_owned(), 1)));

    // (b) `evt-pruned-before-v43` had no surviving event row, so its slot could not be reconstructed.
    let pruned_slot_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM source_receipt_event_slots WHERE event_id = 'evt-pruned-before-v43'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pruned_slot_count, 0);

    // If another event occupies `(task-v42, 2)`, querying `pruned_req` conservatively rejects the occupied sequence.
    connection
        .execute(
            "INSERT INTO task_events(event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json)
             VALUES ('evt-colliding-seq-2', 'task-v42', 2, '2026-11-04T00:09:00Z', 'pause_task', NULL, '{}', NULL)",
            [],
        )
        .unwrap();
    drop(connection);

    let store = TaskStore::open_existing(database.path()).unwrap();
    let err = format!(
        "{:?}",
        store.source_append_receipt_status(&pruned_req).unwrap_err()
    );
    assert!(
        err.contains("receipt event sequence is occupied"),
        "expected occupied sequence error, got {err:?}"
    );
}
