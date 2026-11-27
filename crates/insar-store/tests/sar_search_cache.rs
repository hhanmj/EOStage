use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    catalog_search_cache_key, sar_search_cache_key, CatalogSearchQuery, FrozenSearchBatch,
    FrozenSearchStats, SarSearchQuery, SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest,
};
use insar_store::{StoreSarCacheDecision, TaskStore};
use rusqlite::{params, Connection};
use serde_json::{json, Map};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-sar-cache-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[test]
fn background_count_updates_only_the_active_exact_query_revision() {
    let database = TempDatabase::new("background-count");
    let query = query(30);
    let fingerprint = sar_search_cache_key(&query).unwrap();
    let mut request = request("count-batch", &query);
    request.batch.search_stats = Some(FrozenSearchStats {
        matched_count: None,
        fetched_record_count: Some(2),
        mapped_record_count: Some(2),
        provider_returned_count: 2,
        frozen_candidate_count: 2,
        truncated: false,
        source_attempts: vec![],
        source: "ASF".to_owned(),
    });
    let mut store = TaskStore::open(database.path()).unwrap();
    store.save_search_batch(&request).unwrap();

    assert!(store
        .update_active_sar_matched_count("count-batch", 0, &fingerprint, 11_976)
        .unwrap());
    let active = store.load_workbench().unwrap().unwrap();
    let stats = active.batch.search_stats.unwrap();
    assert_eq!(stats.matched_count, Some(11_976));
    assert_eq!(stats.frozen_candidate_count, 2);
    assert!(stats.truncated);

    assert!(!store
        .update_active_sar_matched_count("count-batch", 1, &fingerprint, 99)
        .unwrap());
    assert!(!store
        .update_active_sar_matched_count("count-batch", 0, &"0".repeat(64), 99)
        .unwrap());
    assert_eq!(
        store
            .load_workbench()
            .unwrap()
            .unwrap()
            .batch
            .search_stats
            .unwrap()
            .matched_count,
        Some(11_976),
    );
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn query(target: u64) -> SarSearchQuery {
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

fn item(id: &str, acquired: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: id.to_owned(),
        acquisition_datetime: Some(acquired.to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn request(batch_id: &str, query: &SarSearchQuery) -> StoreSearchBatchRequest {
    let candidates = vec![
        item("ITEM-Z", "2024-02-03T00:00:00Z"),
        item("ITEM-A", "2024-02-02T00:00:00Z"),
    ];
    StoreSearchBatchRequest {
        activate_workbench: true,
        batch: FrozenSearchBatch {
            schema_version: 1,
            batch_id: batch_id.to_owned(),
            provider_id: "asf.catalog".to_owned(),
            created_at: "2026-08-18T00:00:00Z".to_owned(),
            aoi_name: "无 AOI".to_owned(),
            aoi_geojson: None,
            query: serde_json::to_value(query)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
            search_stats: None,
            candidates,
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

fn catalog_query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": "sar.nisar.l.rslc.provisional.v1",
        "provider_id": "asf.nasa.earthdata",
        "native_collection_id": "C2853145197-ASF",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[37.720070447767895, 10.85690570018474], [37.721296026021705, 10.85690570018474], [37.721296026021705, 10.857787104635449], [37.720070447767895, 10.85690570018474]]]
        },
        "cursor": null,
        "result_limit": 250,
        "filters": {
            "datetime": {"start":"2026-07-01T00:00:00Z", "end":"2026-08-01T00:00:00Z"},
            "production_configuration": "PR",
            "band": "L",
            "frequency_a_polarization": "HH"
        }
    }))
    .unwrap()
}

fn catalog_request(batch_id: &str, query: &CatalogSearchQuery) -> StoreSearchBatchRequest {
    let mut candidate = item("NISAR-ITEM", "2026-07-02T00:00:00Z");
    candidate.provider_id = query.provider_id.clone();
    StoreSearchBatchRequest {
        activate_workbench: true,
        batch: FrozenSearchBatch {
            schema_version: 1,
            batch_id: batch_id.to_owned(),
            provider_id: query.provider_id.clone(),
            created_at: "2026-09-04T00:00:00Z".to_owned(),
            aoi_name: "NISAR cache AOI".to_owned(),
            aoi_geojson: query.aoi_geojson.clone(),
            query: serde_json::to_value(query)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
            search_stats: None,
            candidates: vec![candidate],
        },
        workbench: SearchWorkbenchState {
            batch_id: batch_id.to_owned(),
            checked_item_ids: Vec::new(),
            page_index: 0,
            page_size: 250,
            revision: 0,
        },
    }
}

#[test]
fn immutable_batch_pointer_reuses_and_increments_without_candidate_duplication() {
    let database = TempDatabase::new("reuse");
    let cached_query = query(2);
    let cache_key = sar_search_cache_key(&cached_query).unwrap();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch_with_sar_cache(
                &request("batch-Z-A", &cached_query),
                &cache_key,
                1_000,
                false,
            )
            .unwrap();
        match store
            .load_sar_cache_decision(&cached_query, 1_001, 60)
            .unwrap()
        {
            StoreSarCacheDecision::Reuse { batch } => {
                assert_eq!(
                    batch
                        .candidates
                        .iter()
                        .map(|item| item.item_id.as_str())
                        .collect::<Vec<_>>(),
                    vec!["ITEM-Z", "ITEM-A"]
                );
            }
            other => panic!("expected reuse, got {other:?}"),
        }
        match store.load_sar_cache_decision(&query(3), 1_001, 60).unwrap() {
            StoreSarCacheDecision::Incremental {
                source_batch,
                older_than,
                missing_results,
            } => {
                assert_eq!(source_batch.batch_id, "batch-Z-A");
                assert_eq!(older_than, "2024-02-02T00:00:00Z");
                assert_eq!(missing_results, 1);
            }
            other => panic!("expected incremental, got {other:?}"),
        }
    }

    let store = TaskStore::open(database.path()).unwrap();
    assert!(matches!(
        store
            .load_sar_cache_decision(&cached_query, 1_001, 60)
            .unwrap(),
        StoreSarCacheDecision::Reuse { .. }
    ));
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    let cache_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='sar_search_batch_cache'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!cache_sql.contains("snapshot_json"));
    assert!(!cache_sql.contains("candidate"));
    let pointer: String = connection
        .query_row(
            "SELECT source_batch_id FROM sar_search_batch_cache WHERE cache_key=?1",
            [&cache_key],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pointer, "batch-Z-A");
}

#[test]
fn expiry_foreign_rows_and_failed_publication_never_activate_cache() {
    let database = TempDatabase::new("fail-closed");
    let cached_query = query(2);
    let cache_key = sar_search_cache_key(&cached_query).unwrap();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch_with_sar_cache(
                &request("fresh-batch", &cached_query),
                &cache_key,
                1_000,
                false,
            )
            .unwrap();
        assert!(matches!(
            store
                .load_sar_cache_decision(&cached_query, 1_061, 60)
                .unwrap(),
            StoreSarCacheDecision::Miss
        ));
        assert!(store
            .save_search_batch_with_sar_cache(
                &request("must-rollback", &cached_query),
                "not-a-sha256",
                1_100,
                false,
            )
            .is_err());
    }
    let connection = Connection::open(database.path()).unwrap();
    let rolled_back: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM search_batches WHERE batch_id='must-rollback'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rolled_back, 0);
    connection.execute("PRAGMA foreign_keys=OFF", []).unwrap();
    connection
        .execute(
            "INSERT OR REPLACE INTO sar_search_batch_cache(cache_key,source_batch_id,updated_at_epoch,complete) VALUES(?1,'missing-batch',2000,0)",
            [&cache_key],
        )
        .unwrap();
    drop(connection);
    let store = TaskStore::open(database.path()).unwrap();
    assert!(matches!(
        store
            .load_sar_cache_decision(&cached_query, 2_001, 60)
            .unwrap(),
        StoreSarCacheDecision::Miss
    ));
}

#[test]
fn malformed_referenced_batch_fails_closed() {
    let database = TempDatabase::new("malformed");
    let cached_query = query(2);
    let cache_key = sar_search_cache_key(&cached_query).unwrap();
    let store = TaskStore::open(database.path()).unwrap();
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    connection.execute("PRAGMA foreign_keys=ON", []).unwrap();
    connection
        .execute(
            "INSERT INTO search_batches(batch_id,provider_id,created_at,snapshot_json) VALUES('bad-batch','asf.catalog','2026-08-18T00:00:00Z','{}')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO sar_search_batch_cache(cache_key,source_batch_id,updated_at_epoch,complete) VALUES(?1,'bad-batch',1000,0)",
            params![cache_key],
        )
        .unwrap();
    drop(connection);
    let store = TaskStore::open(database.path()).unwrap();
    assert!(store
        .load_sar_cache_decision(&cached_query, 1_001, 60)
        .is_err());
}

#[test]
fn typed_catalogue_cache_expires_at_the_existing_sar_ttl_boundary() {
    let database = TempDatabase::new("typed-catalogue-expiry");
    let query = catalog_query();
    let cache_key = catalog_search_cache_key(&query).unwrap();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&catalog_request("typed-catalogue-batch", &query))
            .unwrap();
    }
    let connection = Connection::open(database.path()).unwrap();
    connection.execute("PRAGMA foreign_keys=ON", []).unwrap();
    connection
        .execute(
            "INSERT INTO sar_search_batch_cache(cache_key,source_batch_id,updated_at_epoch,complete) VALUES(?1,'typed-catalogue-batch',1000,1)",
            [&cache_key],
        )
        .unwrap();
    drop(connection);

    let store = TaskStore::open(database.path()).unwrap();
    let fresh = store
        .load_catalog_cache_batch(&query, 1_060, 60)
        .unwrap()
        .expect("cache remains valid through the configured TTL boundary");
    assert_eq!(fresh.batch_id, "typed-catalogue-batch");
    assert!(store
        .load_catalog_cache_batch(&query, 1_061, 60)
        .unwrap()
        .is_none());
}

#[test]
fn typed_catalogue_cache_misses_for_each_bound_identity_dimension() {
    let database = TempDatabase::new("typed-catalogue-key-dimensions");
    let query = catalog_query();
    let cache_key = catalog_search_cache_key(&query).unwrap();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&catalog_request("typed-catalogue-key-batch", &query))
            .unwrap();
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute(
            "INSERT INTO sar_search_batch_cache(cache_key,source_batch_id,updated_at_epoch,complete) VALUES(?1,'typed-catalogue-key-batch',1000,1)",
            [&cache_key],
        )
        .unwrap();
    drop(connection);

    let store = TaskStore::open(database.path()).unwrap();
    assert!(store
        .load_catalog_cache_batch(&query, 1_001, 60)
        .unwrap()
        .is_some());
    type QueryMutation = Box<dyn Fn(&mut CatalogSearchQuery)>;
    let mutations: Vec<QueryMutation> = vec![
        Box::new(|query| query.provider_id = "other.provider".to_owned()),
        Box::new(|query| query.native_collection_id = "OTHER-COLLECTION".to_owned()),
        Box::new(|query| query.logical_dataset = "sar.nisar.s.rslc.provisional.v1".to_owned()),
        Box::new(|query| query.cursor = Some("next-page".to_owned())),
        Box::new(|query| query.result_limit = Some(251)),
        Box::new(|query| {
            query.filters.insert("band".to_owned(), json!("S"));
        }),
        Box::new(|query| {
            query.filters.insert(
                "datetime".to_owned(),
                json!({"start":"2026-07-02T00:00:00Z", "end":"2026-08-01T00:00:00Z"}),
            );
        }),
        Box::new(|query| {
            query
                .filters
                .insert("frequency_a_polarization".to_owned(), json!("HH+HV"));
        }),
        Box::new(|query| {
            query.aoi_geojson = Some(
                json!({
                    "type": "Polygon",
                    "coordinates": [[[38.0, 11.0], [38.1, 11.0], [38.1, 11.1], [38.0, 11.0]]]
                })
                .as_object()
                .unwrap()
                .clone(),
            );
        }),
    ];
    for mutation in mutations {
        let mut changed = query.clone();
        mutation(&mut changed);
        assert!(store
            .load_catalog_cache_batch(&changed, 1_001, 60)
            .unwrap()
            .is_none());
    }
}

/// The restricted legacy missions (ERS/JERS-1/RADARSAT-1) moved to
/// authenticated CMR on 2026-10-31 and their cache keys changed with it (see
/// the route-generation tests in insar-core). Pre-fix rows — which could only
/// ever hold a zero-row "complete" answer from the anonymous /param route —
/// carry the old key and therefore miss on every new lookup. This test pins
/// the store side: a row the authenticated route writes under its current
/// key round trips through the decision lookup.
#[test]
fn restricted_mission_cache_rows_are_keyed_by_the_authenticated_route() {
    let jers_query: SarSearchQuery = serde_json::from_value(json!({
        "provider": "ASF",
        "dataset": "JERS-1",
        "start": "1993-01-01T00:00:00Z",
        "end": "1998-10-01T23:59:59Z",
        "max_results": 250,
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[
                [-147.82, 64.78], [-147.52, 64.78], [-147.52, 64.88],
                [-147.82, 64.88], [-147.82, 64.78]
            ]]
        }
    }))
    .unwrap();
    let authenticated_key = sar_search_cache_key(&jers_query).unwrap();

    // The route key change itself (generation 1 vs 2 for the identical
    // typed query) is asserted by the in-core route-generation tests; the
    // store contract this test pins is that the authenticated route's own
    // cache row — written under the current key — round trips through the
    // decision lookup, while any row the pre-fix route wrote under its
    // different key can never satisfy a current lookup.
    let database = TempDatabase::new("legacy-cmr-route");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        let mut batch_request = request("jers-authenticated-batch", &jers_query);
        batch_request.batch.candidates = vec![item("JERS1_ITEM", "1995-06-12T16:44:31Z")];
        store
            .save_search_batch_with_sar_cache(&batch_request, &authenticated_key, 1_000, true)
            .unwrap();
        drop(store);
    }

    let store = TaskStore::open(database.path()).unwrap();
    match store
        .load_sar_cache_decision(&jers_query, 1_001, 60)
        .unwrap()
    {
        StoreSarCacheDecision::Reuse { batch } => {
            assert_eq!(batch.candidates.len(), 1);
            assert_eq!(batch.candidates[0].item_id, "JERS1_ITEM");
        }
        other => panic!("the authenticated route's own cache row must be reused, got {other:?}"),
    }
}
