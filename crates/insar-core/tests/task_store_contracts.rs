// The full `TaskOverviewTask` payload fixture is one large `json!` literal;
// serde_json's macro expansion needs a higher recursion limit than the
// default for it.
#![recursion_limit = "256"]

use std::fs;
use std::path::PathBuf;

use insar_core::{
    DownloadLayout, FrozenSearchBatch, FrozenTaskSnapshot, SnapshotItem, StoreTaskPage,
    TaskLifecycleStatus, TaskOverviewHistoryItem, TaskOverviewTask, TaskRuntimeSnapshot,
    V3_CONTRACT_VERSION, V3_PROTOCOL_MINOR, V3_PROTOCOL_MIN_MINOR,
};
use serde_json::{json, Map, Value};

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_task_store_contracts.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn candidates(section: &Value) -> Vec<SnapshotItem> {
    let prefix = section["candidate_prefix"].as_str().unwrap();
    let provider_id = section["provider_id"].as_str().unwrap();
    (0..section["candidate_count"].as_u64().unwrap())
        .map(|index| SnapshotItem {
            item_id: format!("{prefix}{index:03}"),
            provider_id: provider_id.to_string(),
            display_name: format!("candidate {index}"),
            acquisition_datetime: Some("2024-01-01T00:00:00Z".to_string()),
            footprint_geojson: Some(map(json!({
                "type": "Point",
                "coordinates": [120.0, 30.0]
            }))),
            metadata: map(json!({"path": index % 175 + 1})),
            payload: map(json!({"source": "golden"})),
            asset_access: None,
            execution_policy: None,
        })
        .collect()
}

fn task_page_value() -> Value {
    json!({
        "task_id": "task-byte-facts",
        "revision": 9,
        "page_index": 0,
        "page_size": 100,
        "total_count": 1,
        "items": [{
            "ordinal": 0,
            "item": {
                "item_id": "scene-1",
                "provider_id": "ASF",
                "display_name": "scene-1"
            },
            "checked": true,
            "status": "completed",
            "bytes_completed": 42,
            "total_bytes": 100,
            "bytes_per_second": 0.0,
            "attempt": 1,
            "destination": null,
            "failure": null,
            "finished_at": null,
            "retryable": false
        }]
    })
}

#[test]
fn sentinel1_and_orbit_use_one_frozen_snapshot_contract() {
    let fixture = fixture();
    assert_eq!(V3_CONTRACT_VERSION, "3.43.0");
    assert_eq!(V3_PROTOCOL_MINOR, 42);
    assert_eq!(V3_PROTOCOL_MIN_MINOR, 4);

    for key in ["sentinel1", "orbit"] {
        let section = &fixture[key];
        let batch = FrozenSearchBatch {
            schema_version: 1,
            batch_id: section["batch_id"].as_str().unwrap().to_string(),
            provider_id: section["provider_id"].as_str().unwrap().to_string(),
            created_at: "2024-01-15T00:00:00Z".to_string(),
            aoi_name: section["aoi_name"].as_str().unwrap().to_string(),
            aoi_geojson: Some(map(section["aoi_geojson"].clone())),
            query: map(json!({"product": if key == "sentinel1" { "SLC" } else { "POEORB" }})),
            search_stats: None,
            candidates: candidates(section),
        };
        batch.validate().unwrap();
        let task = FrozenTaskSnapshot {
            schema_version: 1,
            task_id: format!("task-{key}"),
            batch_id: batch.batch_id.clone(),
            provider_id: batch.provider_id.clone(),
            created_at: batch.created_at.clone(),
            aoi_name: batch.aoi_name.clone(),
            aoi_geojson: Some(
                batch
                    .aoi_geojson
                    .clone()
                    .expect("fixture task retains the batch AOI"),
            ),
            query: batch.query.clone(),
            candidates: batch.candidates.clone(),
            initial_selected_item_ids: section["initial_selected_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect(),
            output_root: PathBuf::from(fixture["shared_output_root"].as_str().unwrap()),
            layout: if key == "sentinel1" {
                DownloadLayout::Sentinel1Slc
            } else {
                DownloadLayout::SentinelOrbitAuxPoeorb
            },
            operation: None,
            orbit_association: None,
        };
        task.validate().unwrap();
        assert_eq!(task.candidates.len() as u64, section["candidate_count"]);
        assert_eq!(task.aoi_geojson, batch.aoi_geojson);
    }
}

#[test]
fn task_page_new_reader_defaults_old_byte_facts_and_round_trips_new_values() {
    let old_page: StoreTaskPage =
        serde_json::from_value(task_page_value()).expect("old task page remains readable");
    assert_eq!(old_page.source_total_bytes, None);
    assert_eq!(old_page.network_bytes_received, None);
    assert_eq!(old_page.items[0].source_total_bytes, None);

    let old_serialized = serde_json::to_value(&old_page).expect("old task page serializes");
    assert_eq!(old_serialized["source_total_bytes"], Value::Null);
    assert_eq!(old_serialized["network_bytes_received"], Value::Null);
    assert_eq!(
        old_serialized["items"][0]["source_total_bytes"],
        Value::Null
    );
    let old_round_trip: StoreTaskPage =
        serde_json::from_value(old_serialized).expect("serialized old page remains readable");
    assert_eq!(old_round_trip, old_page);

    let mut new_value = task_page_value();
    new_value["source_total_bytes"] = json!(u64::MAX);
    new_value["network_bytes_received"] = json!(0);
    new_value["items"][0]["source_total_bytes"] = json!(u64::MAX);
    let new_page: StoreTaskPage =
        serde_json::from_value(new_value).expect("new task byte facts deserialize");
    assert_eq!(new_page.source_total_bytes, Some(u64::MAX));
    assert_eq!(new_page.network_bytes_received, Some(0));
    assert_eq!(new_page.items[0].source_total_bytes, Some(u64::MAX));

    let new_serialized = serde_json::to_value(&new_page).expect("new task page serializes");
    assert_eq!(new_serialized["source_total_bytes"], json!(u64::MAX));
    assert_eq!(new_serialized["network_bytes_received"], json!(0));
    assert_eq!(
        new_serialized["items"][0]["source_total_bytes"],
        json!(u64::MAX)
    );
    assert_eq!(new_serialized["items"][0]["total_bytes"], json!(100));
    let new_round_trip: StoreTaskPage =
        serde_json::from_value(new_serialized).expect("serialized new page remains readable");
    assert_eq!(new_round_trip, new_page);
}

#[test]
fn task_page_byte_facts_reject_invalid_and_unknown_json_fields() {
    for field in ["source_total_bytes", "network_bytes_received"] {
        for invalid in [
            json!(-1),
            json!(1.0),
            json!(1.5),
            json!("1"),
            json!("invalid"),
            json!(true),
            json!(false),
        ] {
            let mut value = task_page_value();
            value[field] = invalid;
            assert!(serde_json::from_value::<StoreTaskPage>(value).is_err());
        }
    }
    for invalid in [
        json!(-1),
        json!(1.0),
        json!(1.5),
        json!("1"),
        json!("invalid"),
        json!(true),
        json!(false),
    ] {
        let mut value = task_page_value();
        value["items"][0]["source_total_bytes"] = invalid;
        assert!(serde_json::from_value::<StoreTaskPage>(value).is_err());
    }

    let mut unknown_page_field = task_page_value();
    unknown_page_field["unexpected_byte_fact"] = json!(1);
    assert!(serde_json::from_value::<StoreTaskPage>(unknown_page_field).is_err());

    let mut unknown_item_field = task_page_value();
    unknown_item_field["items"][0]["unexpected_byte_fact"] = json!(1);
    assert!(serde_json::from_value::<StoreTaskPage>(unknown_item_field).is_err());
}

/// Downloader D3: the task page item's local reuse decision is optional
/// (absent and null read as none), strict, and round-trips; the reacquire
/// command kind and the successor's reacquire list have stable names.
#[test]
fn task_page_local_reuse_is_optional_strict_and_round_trips() {
    let old_page: StoreTaskPage = serde_json::from_value(task_page_value()).unwrap();
    assert_eq!(old_page.items[0].local_reuse, None);
    assert_eq!(
        serde_json::to_value(&old_page).unwrap()["items"][0]["local_reuse"],
        Value::Null
    );
    let mut value = task_page_value();
    value["items"][0]["local_reuse"] = json!({
        "disposition": "renamed_conflict",
        "grade": null,
        "conflict_reason": "size_mismatch",
        "conflict_file_name": "a.zip.eostage-conflict-20261020T000000Z"
    });
    let page: StoreTaskPage = serde_json::from_value(value).unwrap();
    let decision = page.items[0].local_reuse.clone().unwrap();
    assert_eq!(
        decision.disposition,
        insar_core::LocalReuseDisposition::RenamedConflict
    );
    let round_trip: StoreTaskPage =
        serde_json::from_value(serde_json::to_value(&page).unwrap()).unwrap();
    assert_eq!(round_trip, page);
    let mut reused = task_page_value();
    reused["items"][0]["local_reuse"] =
        json!({"disposition": "reused", "grade": "size_and_format"});
    let reused: StoreTaskPage = serde_json::from_value(reused).unwrap();
    assert_eq!(
        reused.items[0].local_reuse.as_ref().unwrap().grade,
        Some(insar_core::LocalReuseGrade::SizeAndFormat)
    );
    for invalid in [
        json!({"disposition": "guessed"}),
        json!({"disposition": "reused", "grade": "probably"}),
        json!({"disposition": "reused", "unexpected": 1}),
        json!({"grade": "size_and_format"}),
        json!("reused"),
    ] {
        let mut value = task_page_value();
        value["items"][0]["local_reuse"] = invalid;
        assert!(serde_json::from_value::<StoreTaskPage>(value).is_err());
    }
    assert_eq!(
        serde_json::to_value(insar_core::TaskStateCommandKind::ReacquireItems).unwrap(),
        json!("reacquire_items")
    );
    let successor: insar_core::CreateTaskFromHistoricalSourceBatchRequest =
        serde_json::from_value(json!({
            "source_task_id": "a",
            "expected_source_task_revision": 1,
            "new_task_id": "b",
            "item_ids": ["x"],
            "output_root": "D:/out",
            "layout": "direct",
            "concurrency": 1,
            "created_at": "2026-10-20T00:00:00Z"
        }))
        .unwrap();
    assert!(
        successor.reacquire_item_ids.is_empty(),
        "absent reads as none"
    );
}

#[test]
fn legacy_snapshot_item_without_asset_access_remains_readable() {
    let item: SnapshotItem = serde_json::from_value(json!({
        "item_id": "legacy",
        "provider_id": "asf.catalog",
        "display_name": "legacy.zip"
    }))
    .expect("legacy SnapshotItem deserializes");

    assert!(item.asset_access.is_none());
    assert!(item.execution_policy.is_none());
}

#[test]
fn runtime_contract_rejects_overlap_and_excess_concurrency() {
    let mut runtime = TaskRuntimeSnapshot {
        task_id: "task-sentinel1".to_string(),
        status: TaskLifecycleStatus::Paused,
        selected_item_ids: vec!["a".to_string(), "b".to_string()],
        checked_item_ids: vec!["a".to_string()],
        page_index: 0,
        page_size: 24,
        active_item_ids: vec![],
        paused_item_ids: vec!["a".to_string()],
        queued_item_ids: vec!["b".to_string()],
        completed_item_ids: vec![],
        failed_item_ids: vec![],
        skipped_item_ids: vec![],
        cancelled_item_ids: vec![],
        task_paused: true,
        concurrency: 1,
        revision: 3,
        updated_at: "2024-01-15T01:00:00Z".to_string(),
    };
    runtime.validate().unwrap();

    runtime.active_item_ids.push("a".to_string());
    assert!(runtime
        .validate()
        .unwrap_err()
        .contains("mutually exclusive"));
    runtime.paused_item_ids.clear();
    runtime.queued_item_ids.clear();
    runtime.active_item_ids.push("b".to_string());
    assert!(runtime
        .validate()
        .unwrap_err()
        .contains("exceeds concurrency"));
}

#[test]
fn persisted_contracts_reject_nested_credentials() {
    let fixture = fixture();
    let section = &fixture["orbit"];
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: section["batch_id"].as_str().unwrap().to_string(),
        provider_id: section["provider_id"].as_str().unwrap().to_string(),
        created_at: "2024-01-15T00:00:00Z".to_string(),
        aoi_name: section["aoi_name"].as_str().unwrap().to_string(),
        aoi_geojson: Some(map(section["aoi_geojson"].clone())),
        query: map(json!({"nested": [{"earthdata_token": "forbidden"}]})),
        search_stats: None,
        candidates: candidates(section),
    };

    assert!(batch.validate().unwrap_err().contains("sensitive"));
}

/// Orbit scheduled-fetch S4: the task page item carries the persisted
/// recheck projection. Old payloads without `orbit_recheck` keep reading as
/// None (old reader / new writer and new reader / old writer both work);
/// a present value round-trips with its full schedule facts and rejects
/// unknown fields.
#[test]
fn task_page_item_orbit_recheck_is_optional_strict_and_round_trips() {
    let old_page: StoreTaskPage = serde_json::from_value(task_page_value()).unwrap();
    assert_eq!(old_page.items[0].orbit_recheck, None);
    assert_eq!(
        serde_json::to_value(&old_page).unwrap()["items"][0]["orbit_recheck"],
        Value::Null
    );

    let mut value = task_page_value();
    value["items"][0]["orbit_recheck"] = json!({
        "orbit_type": "POEORB",
        "state": "waiting",
        "attempt": 2,
        "next_check_at": "2026-11-03T00:00:00Z",
        "deadline_at": "2026-12-22T00:00:00Z",
        "expected_file_name": "S1A_OPER_AUX_POEORB_OPOD_20261102T000000_V20261102T225942.EOF"
    });
    let page: StoreTaskPage = serde_json::from_value(value).unwrap();
    let recheck = page.items[0].orbit_recheck.clone().unwrap();
    assert_eq!(recheck.orbit_type, "POEORB");
    assert_eq!(recheck.state, "waiting");
    assert_eq!(recheck.attempt, 2);
    assert_eq!(recheck.next_check_at, "2026-11-03T00:00:00Z");
    assert_eq!(recheck.deadline_at, "2026-12-22T00:00:00Z");
    assert_eq!(
        recheck.expected_file_name.as_deref(),
        Some("S1A_OPER_AUX_POEORB_OPOD_20261102T000000_V20261102T225942.EOF")
    );

    let serialized = serde_json::to_value(&page).unwrap();
    assert_eq!(
        serialized["items"][0]["orbit_recheck"]["orbit_type"],
        json!("POEORB")
    );
    let round_trip: StoreTaskPage = serde_json::from_value(serialized).unwrap();
    assert_eq!(round_trip, page);

    // `expected_file_name` is optional: the EOF name is only known once the
    // provider has resolved the scene (SAR-REG-006 forbids guessing it).
    let mut without_name = task_page_value();
    without_name["items"][0]["orbit_recheck"] = json!({
        "orbit_type": "POEORB",
        "state": "waiting",
        "attempt": 0,
        "next_check_at": "2026-11-03T00:00:00Z",
        "deadline_at": "2026-12-22T00:00:00Z"
    });
    let page: StoreTaskPage = serde_json::from_value(without_name).unwrap();
    assert_eq!(
        page.items[0]
            .orbit_recheck
            .as_ref()
            .unwrap()
            .expected_file_name,
        None
    );

    let mut unknown = task_page_value();
    unknown["items"][0]["orbit_recheck"] = json!({
        "orbit_type": "POEORB",
        "state": "waiting",
        "attempt": 0,
        "next_check_at": "2026-11-03T00:00:00Z",
        "deadline_at": "2026-12-22T00:00:00Z",
        "unexpected": true
    });
    assert!(serde_json::from_value::<StoreTaskPage>(unknown).is_err());
}

/// The minimum valid overview task payload: every required field of the
/// current contract, used to prove the new S4 fields stay optional.
fn overview_task_value() -> Value {
    json!({
        "task_id": "task-overview",
        "batch_id": "batch-1",
        "provider_id": "asf.sentinel1",
        "dataset_id": null,
        "concurrency": null,
        "task_type": null,
        "created_at": "2026-11-01T00:00:00Z",
        "status": "queued",
        "revision": 1,
        "updated_at": "2026-11-01T00:00:00Z",
        "selected_count": 1,
        "delivery_requested_count": null,
        "delivery_rejected_count": null,
        "checked_count": 1,
        "active_count": 0,
        "paused_count": 0,
        "queued_count": 1,
        "completed_count": 0,
        "failed_count": 0,
        "skipped_count": 0,
        "cancelled_count": 0,
        "terminal_bytes": 0,
        "active_bytes": 0,
        "other_bytes": 0,
        "transferred_bytes": 0,
        "total_bytes": null,
        "source_total_bytes": null,
        "bytes_per_second": 0.0,
        "transfer_active_millis": 0,
        "network_bytes_received": null,
        "reusable_verified_bytes": null,
        "reused_verified_bytes": null,
        "active_item": null,
        "active_progress": {
            "without_progress": 0,
            "transferring": 0,
            "idle": 0,
            "waiting": 0
        },
        "failure_codes": [],
        "local_scan": null,
        "output_root": "E:/out",
        "layout": "sentinel1_slc",
        "operation": null,
        "failure": null,
        "pause_reason": null,
        "actionable": true
    })
}

/// S4: `source_task_id` and `auxiliary_kind` project the companion-orbit
/// grouping and the `AUX_POEORB` row identity. Old payloads without them
/// keep reading as None; present values round-trip; unknown fields reject.
#[test]
fn overview_task_source_task_id_and_auxiliary_kind_are_optional_and_round_trip() {
    let old_task: TaskOverviewTask =
        serde_json::from_value(overview_task_value()).expect("old overview task remains readable");
    assert_eq!(old_task.source_task_id, None);
    assert_eq!(old_task.auxiliary_kind, None);
    assert_eq!(
        serde_json::to_value(&old_task).unwrap()["source_task_id"],
        Value::Null
    );
    assert_eq!(
        serde_json::to_value(&old_task).unwrap()["auxiliary_kind"],
        Value::Null
    );

    let mut value = overview_task_value();
    value["source_task_id"] = json!("task-unified-source");
    value["auxiliary_kind"] = json!("AUX_POEORB");
    let task: TaskOverviewTask = serde_json::from_value(value).unwrap();
    assert_eq!(task.source_task_id.as_deref(), Some("task-unified-source"));
    assert_eq!(task.auxiliary_kind.as_deref(), Some("AUX_POEORB"));
    let serialized = serde_json::to_value(&task).unwrap();
    assert_eq!(serialized["source_task_id"], json!("task-unified-source"));
    assert_eq!(serialized["auxiliary_kind"], json!("AUX_POEORB"));
    let round_trip: TaskOverviewTask = serde_json::from_value(serialized).unwrap();
    assert_eq!(round_trip, task);

    let mut unknown = overview_task_value();
    unknown["auxiliary_kind"] = json!("AUX_POEORB");
    unknown["unexpected_auxiliary"] = json!(true);
    assert!(serde_json::from_value::<TaskOverviewTask>(unknown).is_err());
}

/// The history item mirrors the queue task; old archived payloads without
/// `source_task_id` stay readable and new values round-trip.
#[test]
fn overview_history_item_source_task_id_is_optional_and_round_trips() {
    // The history item has no `pause_reason`; drop the queue-only field.
    let history_value = || {
        let mut value = overview_task_value();
        value.as_object_mut().unwrap().remove("pause_reason");
        value["archived_at"] = json!("2026-11-01T01:00:00Z");
        value
    };
    let value = history_value();
    let old_item: TaskOverviewHistoryItem =
        serde_json::from_value(value).expect("old history item remains readable");
    assert_eq!(old_item.source_task_id, None);

    let mut value = history_value();
    value["source_task_id"] = json!("task-unified-source");
    let item: TaskOverviewHistoryItem = serde_json::from_value(value).unwrap();
    assert_eq!(item.source_task_id.as_deref(), Some("task-unified-source"));
    let serialized = serde_json::to_value(&item).unwrap();
    assert_eq!(serialized["source_task_id"], json!("task-unified-source"));
    let round_trip: TaskOverviewHistoryItem = serde_json::from_value(serialized).unwrap();
    assert_eq!(round_trip, item);
}
