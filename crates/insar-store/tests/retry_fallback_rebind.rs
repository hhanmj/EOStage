use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreArchiveTaskRequest,
    StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_store::{
    StoreBatchPageRequest, StoreError, StoreRetryFallbackRequest, StoreTaskHistoryPageRequest,
    TaskStore,
};
use serde_json::{json, Map};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "sp-retry-rebind-{}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn item(id: &str, url: &str) -> SnapshotItem {
    let mut payload = Map::new();
    payload.insert("download_url".to_owned(), json!(url));
    SnapshotItem {
        item_id: id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: id.to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload,
        asset_access: None,
        execution_policy: None,
    }
}

fn batch(id: &str, url: &str, items: &[&str]) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        aoi_name: "fixture".to_owned(),
        aoi_geojson: None,
        query: json!({"dataset": "SENTINEL-1", "provider": "ASF"})
            .as_object()
            .unwrap()
            .clone(),
        search_stats: None,
        candidates: items.iter().map(|i| item(i, url)).collect(),
    }
}

fn save(store: &mut TaskStore, batch: FrozenSearchBatch) {
    let batch_id = batch.batch_id.clone();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: SearchWorkbenchState {
                batch_id,
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 3,
                revision: 0,
            },
            activate_workbench: false,
        })
        .unwrap();
}

const FRESH: &str = "batch-fallback-fresh";

fn search_request(batch: FrozenSearchBatch) -> StoreSearchBatchRequest {
    StoreSearchBatchRequest {
        workbench: SearchWorkbenchState {
            batch_id: batch.batch_id.clone(),
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 3,
            revision: 0,
        },
        batch,
        activate_workbench: false,
    }
}

/// A re-search batch with the task's frozen conditions at new URLs.
fn fallback(id: &str, created_at: &str, items: &[&str]) -> FrozenSearchBatch {
    let mut fallback = batch(id, "https://new", items);
    fallback.created_at = created_at.to_owned();
    fallback
}

fn offer(store: &mut TaskStore, revision: u64, batch: FrozenSearchBatch) {
    store
        .offer_retry_fallback_batch("t", revision, &search_request(batch))
        .unwrap();
}

fn code<T: std::fmt::Debug>(result: Result<T, StoreError>) -> &'static str {
    result.unwrap_err().code()
}

/// A failed two-product task frozen from `orig`, plus the fallback batch
/// `FRESH` frozen for that failure with the same products at new URLs.
/// Returns the failed revision.
fn failed_task(store: &mut TaskStore) -> u64 {
    save(store, batch("orig", "https://old", &["p0", "p1", "p2"]));
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "t".to_owned(),
            batch_id: "orig".to_owned(),
            selected_item_ids: vec!["p0".to_owned(), "p1".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 3,
            output_root: PathBuf::from("fixture-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-01-01T01:00:00Z".to_owned(),
        })
        .unwrap();
    let mut revision = 0;
    for item in ["p0", "p1"] {
        revision = store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "t".to_owned(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec![item.to_owned()],
                },
                event_id: format!("fail-{item}"),
                occurred_at: "2026-01-01T02:00:00Z".to_owned(),
            })
            .unwrap()
            .runtime
            .revision;
    }
    offer(
        store,
        revision,
        fallback(FRESH, "2026-01-01T02:30:00Z", &["p0", "p1", "p3"]),
    );
    revision
}

fn request(revision: u64, items: &[&str]) -> StoreRetryFallbackRequest {
    StoreRetryFallbackRequest {
        task_id: "t".to_owned(),
        expected_revision: revision,
        refreshed_batch_id: FRESH.to_owned(),
        item_ids: items.iter().map(|id| (*id).to_owned()).collect(),
        event_id: "fallback-1".to_owned(),
        occurred_at: "2026-01-01T03:00:00Z".to_owned(),
    }
}

fn url(item: &SnapshotItem) -> &str {
    item.payload["download_url"].as_str().unwrap()
}

#[test]
fn rebind_moves_products_to_refreshed_items_and_retries_the_same_task() {
    let db = Temp::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        let revision = failed_task(&mut store);
        let task = store
            .rebind_task_items_and_retry(&request(revision, &["p0", "p1"]))
            .expect("rebind and retry");
        assert_eq!(task.snapshot.task_id, "t");
        assert_eq!(task.runtime.status, TaskLifecycleStatus::Queued);
        assert_eq!(task.runtime.revision, revision + 1);
        // Replaying the same event is idempotent.
        let replay = store
            .rebind_task_items_and_retry(&request(revision, &["p0", "p1"]))
            .unwrap();
        assert_eq!(replay.runtime.revision, revision + 1);
        assert_eq!(store.task_snapshot_revision("t").unwrap(), 1);
    }
    let store = TaskStore::open(&db.0).unwrap();
    let task = store.load_task_record("t").unwrap().unwrap();
    assert_eq!(
        task.snapshot.batch_id, "orig",
        "original snapshot row stays"
    );
    for candidate in task
        .snapshot
        .candidates
        .iter()
        .filter(|candidate| task.runtime.selected_item_ids.contains(&candidate.item_id))
    {
        assert_eq!(url(candidate), "https://new", "{}", candidate.item_id);
    }
    let scheduled = store.load_scheduler_task("t").unwrap().unwrap();
    assert!(scheduled
        .snapshot
        .candidates
        .iter()
        .all(|candidate| url(candidate) == "https://new"));
    let fresh = store
        .load_batch_page(&StoreBatchPageRequest {
            batch_id: FRESH.to_owned(),
            revision: 0,
            page: 0,
            page_size: 3,
        })
        .unwrap();
    assert_eq!(fresh.rows[0].task_id.as_deref(), Some("t"));
    assert_eq!(fresh.rows[2].task_id, None);
    assert_eq!(
        store
            .load_search_batch("orig")
            .unwrap()
            .unwrap()
            .candidates
            .iter()
            .map(url)
            .collect::<Vec<_>>(),
        ["https://old"; 3],
        "frozen batches never change"
    );
}

#[test]
fn rebind_rejects_unrelated_batches_unselected_products_and_live_tasks() {
    let db = Temp::new();
    let mut store = TaskStore::open(&db.0).unwrap();
    let revision = failed_task(&mut store);
    let mut other = batch("other", "https://new", &["p0", "p1"]);
    other.query.insert("dataset".to_owned(), json!("ALOS-2"));
    save(&mut store, other);
    let mut unrelated = request(revision, &["p0"]);
    unrelated.refreshed_batch_id = "other".to_owned();
    assert_eq!(
        code(store.rebind_task_items_and_retry(&unrelated)),
        "RETRY_FALLBACK_CONFIRMATION_MISMATCH"
    );
    assert_eq!(
        code(store.rebind_task_items_and_retry(&request(revision, &["p2"]))),
        "RETRY_FALLBACK_ITEM_NOT_RETRYABLE"
    );
    assert_eq!(
        code(store.rebind_task_items_and_retry(&request(revision + 7, &["p0"]))),
        "V3_STORE_REVISION_CONFLICT"
    );
    assert_eq!(store.task_snapshot_revision("t").unwrap(), 0);
    assert_eq!(
        store.load_task_record("t").unwrap().unwrap().runtime.status,
        TaskLifecycleStatus::Failed
    );
    // A live (queued) task is neither offered a batch nor rebuilt.
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "live".to_owned(),
            batch_id: "orig".to_owned(),
            selected_item_ids: vec!["p2".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 3,
            output_root: PathBuf::from("fixture-output-live"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-01-01T01:00:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(
        code(store.offer_retry_fallback_batch(
            "live",
            0,
            &search_request(fallback(
                "batch-fallback-live",
                "2026-01-01T04:00:00Z",
                &["p2"]
            )),
        )),
        "RETRY_FALLBACK_TASK_NOT_FAILED"
    );
    let mut live = request(0, &["p2"]);
    live.task_id = "live".to_owned();
    assert_eq!(
        code(store.rebind_task_items_and_retry(&live)),
        "RETRY_FALLBACK_TASK_NOT_FAILED"
    );
}

#[test]
fn archived_failed_task_rebinds_and_leaves_history_on_its_original_id() {
    let db = Temp::new();
    let mut store = TaskStore::open(&db.0).unwrap();
    let revision = failed_task(&mut store);
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "t".to_owned(),
            expected_revision: revision,
            archived_at: "2026-01-01T02:30:00Z".to_owned(),
        })
        .unwrap();
    let history = |store: &TaskStore| {
        store
            .load_task_history_page(&StoreTaskHistoryPageRequest {
                page_size: 10,
                after_archived_at_epoch_ms: None,
                after_task_id: None,
            })
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.task.task_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(history(&store), ["t"]);
    let task = store
        .rebind_task_items_and_retry(&request(revision, &["p1"]))
        .unwrap();
    assert_eq!(task.runtime.status, TaskLifecycleStatus::Queued);
    assert!(history(&store).is_empty(), "history row removed on retry");
    let reloaded = store.load_task_record("t").unwrap().unwrap();
    let urls = reloaded
        .snapshot
        .candidates
        .iter()
        .filter(|candidate| candidate.item_id != "p2")
        .map(|candidate| (candidate.item_id.as_str(), url(candidate)))
        .collect::<Vec<_>>();
    assert_eq!(urls, [("p0", "https://old"), ("p1", "https://new")]);
}

#[test]
fn rebind_requires_the_fallback_batch_offered_for_this_failure() {
    let db = Temp::new();
    let mut store = TaskStore::open(&db.0).unwrap();
    let revision = failed_task(&mut store);
    // A newer re-search supersedes FRESH as the offered batch.
    let newer = "batch-fallback-newer";
    offer(
        &mut store,
        revision,
        fallback(newer, "2026-01-01T02:40:00Z", &["p0", "p1"]),
    );
    assert_eq!(
        code(store.rebind_task_items_and_retry(&request(revision, &["p0"]))),
        "RETRY_FALLBACK_CONFIRMATION_MISMATCH",
        "superseded fallback batch with identical conditions"
    );
    // Identical conditions but not a fallback re-search.
    save(
        &mut store,
        fallback("plain-same", "2026-01-01T02:50:00Z", &["p0"]),
    );
    // A fallback batch frozen before this failure.
    save(
        &mut store,
        fallback("batch-fallback-stale", "2026-01-01T01:30:00Z", &["p0"]),
    );
    for batch_id in ["plain-same", "batch-fallback-stale"] {
        let mut stale = request(revision, &["p0"]);
        stale.refreshed_batch_id = batch_id.to_owned();
        assert_eq!(
            code(store.rebind_task_items_and_retry(&stale)),
            "RETRY_FALLBACK_CONFIRMATION_MISMATCH",
            "{batch_id}"
        );
    }
    // A batch frozen before the failure is never offered.
    assert_eq!(
        code(store.offer_retry_fallback_batch(
            "t",
            revision,
            &search_request(fallback(
                "batch-fallback-early",
                "2026-01-01T01:59:59Z",
                &["p0"]
            )),
        )),
        "RETRY_FALLBACK_CONFIRMATION_MISMATCH"
    );
    assert_eq!(store.task_snapshot_revision("t").unwrap(), 0);
    assert_eq!(
        store.retry_fallback_offer("t").unwrap(),
        Some((newer.to_owned(), revision))
    );
    let mut current = request(revision, &["p0", "p1"]);
    current.refreshed_batch_id = newer.to_owned();
    store.rebind_task_items_and_retry(&current).unwrap();
    assert_eq!(store.task_snapshot_revision("t").unwrap(), 1);
    assert_eq!(
        store.retry_fallback_offer("t").unwrap(),
        None,
        "an offer is used once"
    );
}

#[test]
fn rebind_is_limited_to_failed_and_cancelled_products() {
    let db = Temp::new();
    let mut store = TaskStore::open(&db.0).unwrap();
    save(
        &mut store,
        batch("orig", "https://old", &["p0", "p1", "p2"]),
    );
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "t".to_owned(),
            batch_id: "orig".to_owned(),
            selected_item_ids: vec!["p0".to_owned(), "p1".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 3,
            output_root: PathBuf::from("fixture-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-01-01T01:00:00Z".to_owned(),
        })
        .unwrap();
    let mut revision = 0;
    for (item, kind) in [
        ("p0", TaskStateCommandKind::MarkCompleted),
        ("p1", TaskStateCommandKind::MarkFailed),
    ] {
        revision = store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "t".to_owned(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind,
                    item_ids: vec![item.to_owned()],
                },
                event_id: format!("finish-{item}"),
                occurred_at: "2026-01-01T02:00:00Z".to_owned(),
            })
            .unwrap()
            .runtime
            .revision;
    }
    assert_eq!(
        store.load_task_record("t").unwrap().unwrap().runtime.status,
        TaskLifecycleStatus::Failed
    );
    offer(
        &mut store,
        revision,
        fallback(FRESH, "2026-01-01T02:30:00Z", &["p0", "p1"]),
    );
    assert_eq!(
        code(store.rebind_task_items_and_retry(&request(revision, &["p0", "p1"]))),
        "RETRY_FALLBACK_ITEM_NOT_RETRYABLE",
        "a completed product is never rebound"
    );
    assert_eq!(store.task_snapshot_revision("t").unwrap(), 0);
    store
        .rebind_task_items_and_retry(&request(revision, &["p1"]))
        .unwrap();
    let task = store.load_task_record("t").unwrap().unwrap();
    let urls = task
        .snapshot
        .candidates
        .iter()
        .filter(|candidate| candidate.item_id != "p2")
        .map(|candidate| (candidate.item_id.as_str(), url(candidate)))
        .collect::<Vec<_>>();
    assert_eq!(urls, [("p0", "https://old"), ("p1", "https://new")]);
    assert_eq!(task.runtime.completed_item_ids, ["p0"]);
}
