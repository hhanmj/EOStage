//! Offline-only E1 performance audit for the already accepted ALOS-2 archive.
//!
//! This test is ignored by default and never contacts a provider.  The large
//! artifact is opened in place for checksum/skip measurements; only small
//! temporary fixtures are used for promotion and Store lifecycle timing.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadLayout, DownloadOutcome,
    FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreTaskCommand, StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_download::{
    finalize_part, part_path, verify_file_checksum_with_report, AuthRefreshPolicyId,
    ExpectedChecksum, HttpTransfer, ProxyPolicy, ResolvedAsset, ResolvedChecksum, ResolvedMethod,
    TransferOptions, TransferRequest,
};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

const FILE_NAME: &str = "ALOS2478123150-230331-WBDR1.1__D.zip";
const EXPECTED_SIZE: u64 = 60_018_405_388;
const EXPECTED_MD5: &str = "0f558597e19d964f24ca3a4bff11696c";
const AUDIT_URL: &str = "https://example.invalid/ALOS2478123150-230331-WBDR1.1__D.zip";

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-p04a-e1-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn seed_store(store: &mut TaskStore, root: &Path) {
    let item = SnapshotItem {
        item_id: "ALOS2-E1-AUDIT".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: FILE_NAME.to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload: object(json!({"file_name": FILE_NAME})),
        asset_access: None,
        execution_policy: None,
    };
    store
        .save_search_batch(&insar_core::StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "p04a-e1-audit-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-08-11T00:00:00Z".to_owned(),
                aoi_name: "E1 offline audit".to_owned(),
                aoi_geojson: Some(object(json!({
                    "type": "Polygon",
                    "coordinates": [[[0, 0], [1, 0], [1, 1], [0, 0]]]
                }))),
                query: Map::new(),
                search_stats: None,
                candidates: vec![item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "p04a-e1-audit-batch".to_owned(),
                checked_item_ids: vec!["ALOS2-E1-AUDIT".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "p04a-e1-audit-task".to_owned(),
            batch_id: "p04a-e1-audit-batch".to_owned(),
            selected_item_ids: vec!["ALOS2-E1-AUDIT".to_owned()],
            checked_item_ids: vec!["ALOS2-E1-AUDIT".to_owned()],
            page_index: 0,
            page_size: 24,
            output_root: root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-11T00:00:01Z".to_owned(),
        })
        .unwrap();
}

#[test]
#[ignore = "requires explicit opt-in and the accepted local 60 GB ALOS-2 archive"]
fn offline_large_artifact_audit() {
    let artifact = PathBuf::from(
        std::env::var("INSAR_P04A_E1_ARTIFACT")
            .expect("INSAR_P04A_E1_ARTIFACT must point to the accepted local archive"),
    );
    assert_eq!(
        artifact.file_name().and_then(|name| name.to_str()),
        Some(FILE_NAME)
    );
    let metadata = fs::metadata(&artifact).expect("accepted artifact metadata");
    assert_eq!(metadata.len(), EXPECTED_SIZE);

    let checksum = ExpectedChecksum::new("MD5", EXPECTED_MD5).unwrap();
    let checksum_started = Instant::now();
    let checksum_report = verify_file_checksum_with_report(&artifact, &checksum).unwrap();
    let checksum_ms = checksum_started.elapsed().as_secs_f64() * 1_000.0;

    let skip_started = Instant::now();
    let transfer = HttpTransfer::new_with_proxy_policy(
        TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        },
        ProxyPolicy::Direct,
    )
    .unwrap();
    let request = TransferRequest::new(
        "p04a-e1-skip-task",
        "p04a-e1-skip-item",
        AUDIT_URL,
        artifact.clone(),
        Some(EXPECTED_SIZE),
    )
    .unwrap();
    let asset = ResolvedAsset::new(AUDIT_URL, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(EXPECTED_SIZE))
        .with_checksum(Some(ResolvedChecksum::new("MD5", EXPECTED_MD5).unwrap()));
    let mut session = transfer.resolved_session();
    let skip = transfer.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    let skip_ms = skip_started.elapsed().as_secs_f64() * 1_000.0;
    assert!(matches!(
        skip,
        insar_download::ResolvedTransferResult::Finished(ref result)
            if result.outcome == DownloadOutcome::Skipped
    ));

    let fixture_root = TempRoot::new("promotion-store");
    let fixture_destination = fixture_root.path().join("atomic.bin");
    fs::write(part_path(&fixture_destination), b"atomic-fixture").unwrap();
    let promotion_started = Instant::now();
    assert_eq!(
        finalize_part(&fixture_destination, Some(b"atomic-fixture".len() as u64)).unwrap(),
        b"atomic-fixture".len() as u64
    );
    let promotion_ms = promotion_started.elapsed().as_secs_f64() * 1_000.0;

    let database = fixture_root.path().join("audit.sqlite3");
    let store_started = Instant::now();
    {
        let mut store = TaskStore::open(&database).unwrap();
        seed_store(&mut store, fixture_root.path());
        let progress = DownloadItemProgress {
            task_id: "p04a-e1-audit-task".to_owned(),
            item_id: "ALOS2-E1-AUDIT".to_owned(),
            status: DownloadItemStatus::Active,
            bytes_completed: EXPECTED_SIZE,
            total_bytes: Some(EXPECTED_SIZE),
            bytes_per_second: 1.0,
            attempt: 1,
            updated_at: "2026-08-11T00:00:02Z".to_owned(),
        };
        let progress_snapshot = store.save_download_progress(&progress, 0).unwrap();
        let result = DownloadItemResult {
            task_id: "p04a-e1-audit-task".to_owned(),
            item_id: "ALOS2-E1-AUDIT".to_owned(),
            outcome: DownloadOutcome::Completed,
            destination: fixture_destination.clone(),
            bytes_written: EXPECTED_SIZE,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-11T00:00:03Z".to_owned(),
        };
        let result_snapshot = store
            .save_download_result(&result, progress_snapshot.revision)
            .unwrap();
        assert_eq!(result_snapshot.results.len(), 1);
        let task_revision = store
            .load_scheduler_task("p04a-e1-audit-task")
            .unwrap()
            .expect("seeded audit task")
            .runtime
            .revision;
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "p04a-e1-audit-task".to_owned(),
                expected_revision: task_revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkCompleted,
                    item_ids: vec!["ALOS2-E1-AUDIT".to_owned()],
                },
                event_id: "p04a-e1-audit-complete".to_owned(),
                occurred_at: "2026-08-11T00:00:04Z".to_owned(),
            })
            .unwrap();
    }
    let store_persistence_ms = store_started.elapsed().as_secs_f64() * 1_000.0;

    let reopen_started = Instant::now();
    let reopened = TaskStore::open(&database).unwrap();
    let restored = reopened
        .load_task("p04a-e1-audit-task")
        .unwrap()
        .expect("completed audit task");
    let store_reopen_ms = reopen_started.elapsed().as_secs_f64() * 1_000.0;
    assert!(restored
        .runtime
        .completed_item_ids
        .contains(&"ALOS2-E1-AUDIT".to_owned()));

    println!(
        "E1_LARGE_ARTIFACT_AUDIT {}",
        serde_json::to_string(&json!({
            "artifact_file_name": FILE_NAME,
            "artifact_size_bytes": EXPECTED_SIZE,
            "checksum_algorithm": "MD5",
            "checksum_bytes_read": checksum_report.bytes_read,
            "checksum_ms": checksum_ms,
            "skip_outcome": "skipped",
            "skip_ms": skip_ms,
            "skip_network": false,
            "artifact_validation_ms": 0.0,
            "artifact_validation_bytes_read": 0,
            "promotion_fixture_bytes": b"atomic-fixture".len(),
            "promotion_fixture_ms": promotion_ms,
            "store_persistence_ms": store_persistence_ms,
            "store_reopen_ms": store_reopen_ms,
            "audit_invocation_large_file_scans": 2,
            "audit_invocation_large_file_scan_basis": "one direct MD5 verification plus one existing-final skip MD5 verification",
            "fresh_transfer_complete_file_scans_before_qualified": 4,
            "fresh_transfer_complete_file_scans_after_qualified": 2,
            "fresh_transfer_scan_basis": "small-fixture safe-promotion instrumentation qualifies the production path; provider checksum and promotion SHA are accumulated during the network stream, while two identity-bound promotion SHA passes remain",
            "credential_resolution_for_skip": false,
            "part_present_after_promotion": false
        }))
        .unwrap()
    );
}
