use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    sar_search_cache_key, AsfCacheEntry, AsfSceneMetadata, AsfSearchQuery, FrozenSearchBatch,
    SarSearchQuery, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest,
};
use insar_store::{StoreSearchBatchStagingRequest, StoredAdministrativeBoundary, TaskStore};
use serde_json::{json, Map, Value};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-cache-maint-{label}-{}-{nonce}.sqlite3",
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

fn sar_query(target: u64) -> SarSearchQuery {
    serde_json::from_value(json!({
        "provider": "ASF",
        "dataset": "SENTINEL-1",
        "platforms": ["Sentinel-1A"],
        "processing_levels": ["SLC"],
        "beam_modes": ["IW"],
        "flight_direction": "ASCENDING",
        "max_results": target
    }))
    .unwrap()
}

fn snapshot_item(id: &str, payload_padding: usize) -> SnapshotItem {
    let mut payload = Map::new();
    if payload_padding > 0 {
        payload.insert("pad".to_owned(), Value::String("x".repeat(payload_padding)));
    }
    SnapshotItem {
        item_id: id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: id.to_owned(),
        acquisition_datetime: Some("2026-08-18T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload,
        asset_access: None,
        execution_policy: None,
    }
}

fn batch_request(
    batch_id: &str,
    query: &SarSearchQuery,
    activate_workbench: bool,
    payload_padding: usize,
) -> StoreSearchBatchRequest {
    StoreSearchBatchRequest {
        activate_workbench,
        batch: FrozenSearchBatch {
            schema_version: 1,
            batch_id: batch_id.to_owned(),
            provider_id: "asf.catalog".to_owned(),
            created_at: "2026-08-18T00:00:00Z".to_owned(),
            aoi_name: "Test AOI".to_owned(),
            aoi_geojson: None,
            query: serde_json::to_value(query)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
            search_stats: None,
            candidates: vec![snapshot_item(&format!("{batch_id}-item"), payload_padding)],
        },
        workbench: SearchWorkbenchState {
            batch_id: batch_id.to_owned(),
            checked_item_ids: Vec::new(),
            page_index: 0,
            page_size: 24,
            revision: 0,
        },
    }
}

fn boundary_entry(code: &str, cached_at: &str) -> StoredAdministrativeBoundary {
    let geom = json!({
        "type": "Polygon",
        "coordinates": [[[109.0, 30.0], [110.0, 30.0], [110.0, 31.0], [109.0, 31.0], [109.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone();
    StoredAdministrativeBoundary {
        provider: "datav".to_owned(),
        provider_object_id: code.to_owned(),
        name: format!("区划{code}"),
        display_name: format!("测试省 / 区划{code}"),
        country_code: Some("CN".to_owned()),
        admin_level: Some("district".to_owned()),
        geometry: geom,
        bbox: [109.0, 30.0, 110.0, 31.0],
        source: "DataV areas_v3".to_owned(),
        attribution: None,
        cached_at: cached_at.to_owned(),
    }
}

fn asf_entry(key: &str, scene_count: usize) -> AsfCacheEntry {
    let scenes = (0..scene_count)
        .map(|idx| AsfSceneMetadata {
            scene_id: format!("{key}-scene-{idx}"),
            url: None,
            platform: "S1A".to_owned(),
            product_type: "SLC".to_owned(),
            beam_mode: "IW".to_owned(),
            polarization: "DV".to_owned(),
            acquisition_datetime: "2026-08-18T00:00:00Z".to_owned(),
            absolute_orbit: Some(50_000 + idx as u64),
            path: Some(42),
            frame: Some(456),
            orbit_direction: Some("ASCENDING".to_owned()),
            file_size_remote: None,
            footprint_bbox: None,
            footprint_geojson: None,
            provider_product_id: None,
            file_name: None,
            processing_level: None,
            acquisition_stop: None,
            browse_url: None,
            thumbnail_url: None,
            checksum_algorithm: None,
            checksum_value: None,
            center_latitude: None,
            center_longitude: None,
        })
        .collect::<Vec<_>>();
    AsfCacheEntry {
        cache_key: key.to_owned(),
        query: AsfSearchQuery {
            version: 1,
            dataset: "SENTINEL-1".to_owned(),
            bbox: None,
            aoi: String::new(),
            aoi_geojson: None,
            start: "2026-01-01".to_owned(),
            end: "2026-08-18".to_owned(),
            product_type: "SLC".to_owned(),
            beam_mode: "IW".to_owned(),
            polarization: String::new(),
            orbit_direction: String::new(),
            relative_orbit: None,
            frame: None,
            max_results: Some(scene_count as u64),
        },
        scenes,
        complete: true,
        total_count: Some(scene_count as u64),
    }
}

#[test]
fn asf_cache_prunes_by_lru_entry_cap_and_ttl() {
    let db = TempDatabase::new("asf-bounds");
    let mut store = TaskStore::open(db.path()).unwrap();

    for idx in 0..5_u64 {
        store
            .upsert_asf_cache_entry(&asf_entry(&format!("key-{idx}"), 1), 100_000 + idx * 10)
            .unwrap();
    }

    let removed = store
        .prune_asf_cache_bounds(100_040, 86_400, 3, 10 * 1024 * 1024)
        .unwrap();
    assert_eq!(removed, 2);

    let remaining = store.load_fresh_asf_cache_entries(100_040, 86_400).unwrap();
    let keys: Vec<&str> = remaining.iter().map(|e| e.cache_key.as_str()).collect();
    assert_eq!(keys, vec!["key-4", "key-3", "key-2"]);
}

#[test]
fn admin_boundary_cache_prunes_by_ttl_and_lru_cap() {
    let db = TempDatabase::new("admin-bounds");
    let mut store = TaskStore::open(db.path()).unwrap();

    store
        .upsert_administrative_boundary(&boundary_entry("340101", "2026-01-01T00:00:00Z"))
        .unwrap();
    store
        .upsert_administrative_boundary(&boundary_entry("340102", "2026-08-01T00:00:00Z"))
        .unwrap();
    store
        .upsert_administrative_boundary(&boundary_entry("340103", "2026-08-15T00:00:00Z"))
        .unwrap();
    store
        .upsert_administrative_boundary(&boundary_entry("340104", "2026-08-20T00:00:00Z"))
        .unwrap();

    let removed = store
        .prune_administrative_boundary_cache_bounds(
            Some("2026-07-01T00:00:00Z"),
            2,
            10 * 1024 * 1024,
        )
        .unwrap();
    assert_eq!(removed, 2);
    assert!(store
        .load_administrative_boundary("datav", "340101")
        .unwrap()
        .is_none());
    assert!(store
        .load_administrative_boundary("datav", "340102")
        .unwrap()
        .is_none());
    assert!(store
        .load_administrative_boundary("datav", "340103")
        .unwrap()
        .is_some());
    assert!(store
        .load_administrative_boundary("datav", "340104")
        .unwrap()
        .is_some());
}

#[test]
fn clear_disposable_caches_preserves_tasks_and_active_workbench_while_reclaiming_space() {
    let db = TempDatabase::new("safe-clear");
    let mut store = TaskStore::open(db.path()).unwrap();

    // 1. Batch bound to a real user download task (MUST be protected).
    let q_task = sar_query(10);
    let req_task = batch_request("batch-task-owned", &q_task, true, 512);
    let key_task = sar_search_cache_key(&q_task).unwrap();
    store
        .save_search_batch_with_sar_cache(&req_task, &key_task, 200_000, true)
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-protected-1".to_owned(),
            batch_id: "batch-task-owned".to_owned(),
            selected_item_ids: vec!["batch-task-owned-item".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: PathBuf::from("/tmp/eostage-test-out"),
            layout: insar_core::DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-18T00:01:00Z".to_owned(),
        })
        .unwrap();

    // 2. Unreferenced disposable search batches (older searches no longer active or tasked).
    for idx in 1..=4_u64 {
        let q = sar_query(10 + idx);
        let key = sar_search_cache_key(&q).unwrap();
        let req = batch_request(&format!("batch-disposable-{idx}"), &q, false, 8_192);
        store
            .save_search_batch_with_sar_cache(&req, &key, 200_000 + idx * 10, true)
            .unwrap();
    }

    // 3. New active workbench batch (MUST be protected).
    let q_active = sar_query(99);
    let key_active = sar_search_cache_key(&q_active).unwrap();
    let req_active = batch_request("batch-active-wb", &q_active, true, 512);
    store
        .save_search_batch_with_sar_cache(&req_active, &key_active, 200_100, true)
        .unwrap();

    // 4. Legacy ASF cache & admin boundary cache rows.
    store
        .upsert_asf_cache_entry(&asf_entry("asf-legacy-1", 2), 200_100)
        .unwrap();
    store
        .upsert_administrative_boundary(&boundary_entry("340100", "2026-08-18T00:00:00Z"))
        .unwrap();

    let stats_before = store.inspect_disposable_cache_stats().unwrap();
    assert!(stats_before.search_cache_bytes > 30_000);
    assert!(stats_before.search_cache_entries >= 4);
    assert!(stats_before.admin_boundary_cache_bytes > 0);
    assert_eq!(stats_before.admin_boundary_cache_entries, 1);

    // Clear disposable search cache and admin boundary cache.
    let cleared_search = store.clear_disposable_search_cache().unwrap();
    assert!(cleared_search.removed_entries >= 4);
    assert!(cleared_search.reclaimed_bytes > 30_000);

    let cleared_admin = store.clear_administrative_boundary_cache().unwrap();
    assert_eq!(cleared_admin.removed_entries, 1);
    assert!(cleared_admin.reclaimed_bytes > 0);

    let vacuumed = store.reclaim_sqlite_space(true).unwrap();
    assert!(vacuumed);

    let stats_after = store.inspect_disposable_cache_stats().unwrap();
    assert_eq!(stats_after.search_cache_bytes, 0);
    assert_eq!(stats_after.search_cache_entries, 0);
    assert_eq!(stats_after.admin_boundary_cache_bytes, 0);
    assert_eq!(stats_after.admin_boundary_cache_entries, 0);

    // Protected user data MUST remain 100% intact.
    let wb = store
        .load_workbench()
        .unwrap()
        .expect("active workbench intact");
    assert_eq!(wb.batch.batch_id, "batch-active-wb");
    let task = store
        .load_task_record("task-protected-1")
        .unwrap()
        .expect("download task intact");
    assert_eq!(task.snapshot.batch_id, "batch-task-owned");
}

#[test]
fn disposable_search_cache_evicts_24h_expired_and_abandoned_staging_while_keeping_loading_and_pins()
{
    let db = TempDatabase::new("ttl-and-staging");
    let mut store = TaskStore::open(db.path()).unwrap();

    // Seed an active workbench batch with an old created_at (must be protected).
    let q_wb = sar_query(1);
    let mut req_wb = batch_request("batch-wb-old", &q_wb, true, 512);
    req_wb.batch.created_at = "2026-08-01T00:00:00Z".to_owned();
    store.save_search_batch(&req_wb).unwrap();

    // Seed an expired unreferenced batch (`2026-08-01`) vs a fresh unreferenced batch (`2026-11-20`).
    let q_old = sar_query(2);
    let mut req_old = batch_request("batch-disp-old", &q_old, false, 1024);
    req_old.batch.created_at = "2026-08-01T00:00:00Z".to_owned();
    store.save_search_batch(&req_old).unwrap();

    let q_fresh = sar_query(3);
    let mut req_fresh = batch_request("batch-disp-fresh", &q_fresh, false, 1024);
    req_fresh.batch.created_at = "2026-11-20T00:00:00Z".to_owned();
    store.save_search_batch(&req_fresh).unwrap();

    // Seed an abandoned staging row (from an interrupted search) AND an active loading staging row.
    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: "req-abandoned".to_owned(),
            max_candidates: 10,
            batch_header: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-staging-abandoned".to_owned(),
                provider_id: "asf.catalog".to_owned(),
                created_at: "2026-11-19T00:00:00Z".to_owned(),
                aoi_name: "Test AOI".to_owned(),
                aoi_geojson: None,
                query: serde_json::to_value(sar_query(4))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
                search_stats: None,
                candidates: Vec::new(),
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-staging-abandoned".to_owned(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .append_search_batch_staging_candidates(
            "req-abandoned",
            &[snapshot_item("abandoned-item", 2048)],
        )
        .unwrap();

    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: "req-loading-active".to_owned(),
            max_candidates: 10,
            batch_header: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-staging-loading".to_owned(),
                provider_id: "asf.catalog".to_owned(),
                created_at: "2026-11-20T00:00:00Z".to_owned(),
                aoi_name: "Test AOI".to_owned(),
                aoi_geojson: None,
                query: serde_json::to_value(sar_query(5))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
                search_stats: None,
                candidates: Vec::new(),
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-staging-loading".to_owned(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .append_search_batch_staging_candidates(
            "req-loading-active",
            &[snapshot_item("loading-item", 2048)],
        )
        .unwrap();
    {
        let conn = rusqlite::Connection::open(db.path()).unwrap();
        conn.execute(
            "INSERT INTO search_batch_progress(batch_id, request_id, state, remote_total, updated_at)
             VALUES ('batch-staging-loading', 'req-loading-active', 'loading', 10, '2026-11-20T00:00:00Z')",
            [],
        )
        .unwrap();
    }

    // `now_epoch` corresponding to 2026-11-20T12:00:00Z (~1_795_176_000) evicts 2026-08-01 + abandoned staging,
    // while keeping active workbench, fresh batches, and active `'loading'` staging intact.
    let now_epoch = 1_795_176_000_u64;
    let outcome = store
        .prune_disposable_search_cache_bounds(
            now_epoch,
            TaskStore::SEARCH_CACHE_DEFAULT_TTL_SECONDS,
            TaskStore::SEARCH_CACHE_MAX_ENTRIES,
            TaskStore::SEARCH_CACHE_MAX_BYTES,
        )
        .unwrap();
    assert!(outcome.removed_entries >= 2);
    assert!(outcome.reclaimed_bytes > 2048);

    assert!(store.load_search_batch("batch-wb-old").unwrap().is_some());
    assert!(store
        .load_search_batch("batch-disp-fresh")
        .unwrap()
        .is_some());
    assert!(store.load_search_batch("batch-disp-old").unwrap().is_none());
    assert_eq!(store.count_staging_rows().unwrap(), 1);
}

#[test]
fn uncached_save_and_staging_seal_enforce_shared_asf_plus_batch_bounds() {
    let db = TempDatabase::new("uncached-bounds");
    let mut store = TaskStore::open(db.path()).unwrap();

    // Seed 4 legacy ASF cache entries so shared budget leaves room for at most 60 batches ->
    // inserting 70 batches via uncached save_search_batch triggers cap enforcement cleanly.
    for idx in 0..4_u64 {
        store
            .upsert_asf_cache_entry(&asf_entry(&format!("asf-shared-{idx}"), 1), 100_000 + idx)
            .unwrap();
    }

    for idx in 0..70_u64 {
        let q = sar_query(100 + idx);
        let mut req = batch_request(&format!("batch-uncached-{idx:03}"), &q, false, 64);
        req.batch.created_at = format!("2026-11-20T{:02}:{:02}:00Z", idx / 60, idx % 60);
        store.save_search_batch(&req).unwrap();
    }

    // Also seal a new batch via uncached streaming staging seal and confirm cap still holds.
    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: "req-stream-seal".to_owned(),
            max_candidates: 10,
            batch_header: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-stream-sealed-last".to_owned(),
                provider_id: "asf.catalog".to_owned(),
                created_at: "2026-11-20T05:00:00Z".to_owned(),
                aoi_name: "Test AOI".to_owned(),
                aoi_geojson: None,
                query: serde_json::to_value(sar_query(999))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
                search_stats: None,
                candidates: Vec::new(),
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-stream-sealed-last".to_owned(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .append_search_batch_staging_candidates(
            "req-stream-seal",
            &[snapshot_item("stream-sealed-item", 64)],
        )
        .unwrap();
    store
        .seal_search_batch_staging_streaming("req-stream-seal", None, false)
        .unwrap();

    let stats = store.inspect_disposable_cache_stats().unwrap();
    assert!(
        stats.search_cache_entries <= TaskStore::SEARCH_CACHE_MAX_ENTRIES as u64,
        "shared search cache entries exceeded cap: {}",
        stats.search_cache_entries
    );
    assert!(store
        .load_search_batch("batch-stream-sealed-last")
        .unwrap()
        .is_some());
}

#[test]
fn pending_request_id_source_view_protects_interrupted_batch_from_prune_and_clear() {
    let db = TempDatabase::new("pending-source-view");
    let mut store = TaskStore::open(db.path()).unwrap();

    // Seed a source view still pointing at a request via pending_request_id (batch_id is NULL)
    // and an interrupted batch registered under that request in search_batch_progress.
    store
        .mark_source_view_pending(
            "science.hls.l30.v2",
            "req-pending-interrupted",
            "2026-08-01T00:00:00Z",
        )
        .unwrap();
    let mut req_pending = batch_request("batch-pending-interrupted", &sar_query(808), false, 512);
    req_pending.batch.created_at = "2026-08-01T00:00:00Z".to_owned();
    store.save_search_batch(&req_pending).unwrap();
    {
        let conn = rusqlite::Connection::open(db.path()).unwrap();
        conn.execute(
            "INSERT INTO search_batch_progress(batch_id, request_id, state, remote_total, updated_at)
             VALUES ('batch-pending-interrupted', 'req-pending-interrupted', 'interrupted', 100, '2026-08-01T00:00:00Z')",
            [],
        )
        .unwrap();
    }

    let now_epoch = 1_795_176_000_u64;
    store
        .prune_disposable_search_cache_bounds(
            now_epoch,
            TaskStore::SEARCH_CACHE_DEFAULT_TTL_SECONDS,
            TaskStore::SEARCH_CACHE_MAX_ENTRIES,
            TaskStore::SEARCH_CACHE_MAX_BYTES,
        )
        .unwrap();
    assert!(
        store
            .load_search_batch("batch-pending-interrupted")
            .unwrap()
            .is_some(),
        "prune must preserve batch linked by source_views.pending_request_id"
    );

    store.clear_disposable_search_cache().unwrap();
    assert!(
        store
            .load_search_batch("batch-pending-interrupted")
            .unwrap()
            .is_some(),
        "clear must preserve batch linked by source_views.pending_request_id"
    );
}
