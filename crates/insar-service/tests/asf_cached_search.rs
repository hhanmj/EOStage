use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    asf_search_cache_key, AsfCacheDecisionKind, AsfCacheEntry, AsfCachedSearchRequest,
    AsfCachedSearchResponse, AsfNormalizeError, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    AsfSceneMetadata, AsfSearchQuery, AsfSearchStats, ContractError, ProviderResult,
    SidecarOperation, SidecarRequest,
};
use insar_service::{Dispatcher, LiveAsfSearch};
use insar_store::TaskStore;
use serde_json::Map;

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-service-asf-cache-{label}-{}-{nonce}.sqlite3",
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

#[derive(Default)]
struct CountingSearch {
    calls: Mutex<Vec<AsfRemoteSearchRequest>>,
    responses: Mutex<VecDeque<ProviderResult<AsfRemoteSearchResponse>>>,
}

impl CountingSearch {
    fn with_responses(responses: Vec<AsfRemoteSearchResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            ..Self::default()
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl LiveAsfSearch for CountingSearch {
    fn search(
        &self,
        request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        self.calls.lock().unwrap().push(request.clone());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted response")
    }
}

fn query(limit: Option<u64>) -> AsfSearchQuery {
    AsfSearchQuery {
        version: 1,
        dataset: "SENTINEL-1".to_string(),
        bbox: None,
        aoi: String::new(),
        aoi_geojson: None,
        start: "2024-01-01".to_string(),
        end: "2024-12-31".to_string(),
        product_type: "SLC".to_string(),
        beam_mode: "IW".to_string(),
        polarization: String::new(),
        orbit_direction: String::new(),
        relative_orbit: None,
        frame: None,
        max_results: limit,
    }
}

fn scene(id: &str, acquired: &str, direction: &str) -> AsfSceneMetadata {
    AsfSceneMetadata {
        scene_id: id.to_string(),
        url: None,
        platform: "S1A".to_string(),
        product_type: "SLC".to_string(),
        beam_mode: "IW".to_string(),
        polarization: "DV".to_string(),
        acquisition_datetime: acquired.to_string(),
        absolute_orbit: Some(52_000),
        path: Some(42),
        frame: Some(456),
        orbit_direction: Some(direction.to_string()),
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
    }
}

fn remote_response(scenes: Vec<AsfSceneMetadata>) -> AsfRemoteSearchResponse {
    AsfRemoteSearchResponse {
        stats: AsfSearchStats {
            requested_limit: scenes.len().max(1) as u64,
            query_limit: scenes.len().max(1) as u64,
            metadata_request_count: None,
            total_count: Some(scenes.len() as u64),
            candidate_count: scenes.len() as u64,
            returned_count: scenes.len() as u64,
            source: "ASF".to_string(),
            cmr_enriched: None,
            matched_count: None,
            fetched_record_count: None,
            replayed_record_count: None,
            mapped_record_count: None,
            mapping_error_count: None,
            aggregated_asset_record_count: None,
            unique_provider_product_count: None,
            asset_count: None,
            identity_conflict_count: None,
            page_size: None,
            truncated: None,
            item_errors: Vec::new(),
            warnings: Vec::new(),
        },
        scenes,
        errors: Vec::<AsfNormalizeError>::new(),
    }
}

fn request(query: AsfSearchQuery) -> SidecarRequest {
    let request_id = format!(
        "cached-search-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    request_with_id(query, &request_id)
}

fn request_with_id(query: AsfSearchQuery, request_id: &str) -> SidecarRequest {
    let payload = serde_json::to_value(AsfCachedSearchRequest {
        query,
        allow_cmr_fallback: true,
        max_cache_age_seconds: 86_400,
    })
    .unwrap()
    .as_object()
    .unwrap()
    .clone();
    SidecarRequest {
        protocol_major: 1,
        protocol_minor: 8,
        request_id: request_id.to_string(),
        operation: SidecarOperation::AsfSearchCached,
        payload,
    }
}

struct WaitingSearch {
    started: Arc<AtomicBool>,
}

impl LiveAsfSearch for WaitingSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        self.started.store(true, Ordering::SeqCst);
        while !cancelled() {
            thread::yield_now();
        }
        Err(ContractError::new("DL001", "ASF 检索已停止", Map::new()).unwrap())
    }
}

fn dispatch(dispatcher: &Dispatcher, query: AsfSearchQuery) -> AsfCachedSearchResponse {
    let response = dispatcher.dispatch(request(query));
    assert!(response.ok, "{:?}", response.error);
    serde_json::from_value(serde_json::Value::Object(response.result)).unwrap()
}

#[test]
fn exact_second_search_reuses_sqlite_without_network() {
    let database = TempDatabase::new("exact");
    let remote = Arc::new(CountingSearch::with_responses(vec![remote_response(vec![
        scene("scene-1", "2024-03-01T00:00:00Z", "ASCENDING"),
    ])]));
    let dispatcher =
        Dispatcher::with_asf_search_and_store_path(remote.clone(), database.path().to_path_buf());

    let first = dispatch(&dispatcher, query(Some(1)));
    let second = dispatch(&dispatcher, query(Some(1)));

    assert_eq!(first.cache_kind, AsfCacheDecisionKind::Miss);
    assert_eq!(second.cache_kind, AsfCacheDecisionKind::Reuse);
    assert_eq!(second.result.stats.source, "RUST_ASF_CACHE");
    assert_eq!(remote.call_count(), 1);
}

#[test]
fn broad_cache_filters_narrow_request_without_network() {
    let database = TempDatabase::new("broad");
    let broad = query(None);
    let broad_key = asf_search_cache_key(&broad).unwrap();
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .upsert_asf_cache_entry(
            &AsfCacheEntry {
                cache_key: broad_key.clone(),
                query: broad,
                scenes: vec![
                    scene("ascending", "2024-03-01T00:00:00Z", "ASCENDING"),
                    scene("descending", "2024-02-01T00:00:00Z", "DESCENDING"),
                ],
                complete: true,
                total_count: Some(2),
            },
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();
    drop(store);
    let remote = Arc::new(CountingSearch::default());
    let dispatcher =
        Dispatcher::with_asf_search_and_store_path(remote.clone(), database.path().to_path_buf());
    let mut narrow = query(Some(1));
    narrow.orbit_direction = "DESCENDING".to_string();

    let response = dispatch(&dispatcher, narrow);

    assert_eq!(response.cache_kind, AsfCacheDecisionKind::Reuse);
    assert_eq!(response.cache_key, broad_key);
    assert_eq!(response.result.scenes[0].scene_id, "descending");
    assert_eq!(remote.call_count(), 0);
}

#[test]
fn increased_limit_fetches_only_missing_older_scenes_and_persists_merge() {
    let database = TempDatabase::new("incremental");
    let cached_query = query(Some(1));
    let cache_key = asf_search_cache_key(&cached_query).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .upsert_asf_cache_entry(
            &AsfCacheEntry {
                cache_key,
                query: cached_query,
                scenes: vec![scene("newer", "2024-03-01T00:00:00Z", "ASCENDING")],
                complete: false,
                total_count: Some(5),
            },
            now,
        )
        .unwrap();
    drop(store);
    let remote = Arc::new(CountingSearch::with_responses(vec![remote_response(vec![
        scene("older", "2024-02-01T00:00:00Z", "ASCENDING"),
    ])]));
    let dispatcher =
        Dispatcher::with_asf_search_and_store_path(remote.clone(), database.path().to_path_buf());

    let response = dispatch(&dispatcher, query(Some(2)));

    assert_eq!(response.cache_kind, AsfCacheDecisionKind::Incremental);
    assert_eq!(response.result.scenes.len(), 2);
    assert_eq!(remote.call_count(), 1);
    let calls = remote.calls.lock().unwrap();
    assert_eq!(calls[0].query.max_results, Some(1));
    assert_eq!(calls[0].query.end, "2024-03-01T00:00:00Z");
    drop(calls);
    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        store.load_fresh_asf_cache_entries(now, 86_400).unwrap()[0]
            .scenes
            .len(),
        2
    );
}

#[test]
fn protocol_six_cannot_invoke_cached_search() {
    let database = TempDatabase::new("protocol");
    let dispatcher = Dispatcher::with_asf_search_and_store_path(
        Arc::new(CountingSearch::default()),
        database.path().to_path_buf(),
    );
    let mut old = request(query(Some(1)));
    old.protocol_minor = 6;

    let response = dispatcher.dispatch(old);

    assert!(!response.ok);
    assert_eq!(
        response.error.unwrap().code,
        "V3_SIDECAR_CAPABILITY_UNAVAILABLE"
    );
}

#[test]
fn preview_metadata_is_only_emitted_on_the_reviewed_protocol_minor() {
    let database = TempDatabase::new("preview-protocol");
    let mut preview_scene = scene("preview", "2024-03-01T00:00:00Z", "ASCENDING");
    preview_scene.provider_product_id = Some("provider-preview".to_string());
    preview_scene.browse_url =
        Some("https://datapool.asf.alaska.edu/BROWSE/provider-preview.jpg".to_string());
    let remote = Arc::new(CountingSearch::with_responses(vec![remote_response(vec![
        preview_scene,
    ])]));
    let dispatcher =
        Dispatcher::with_asf_search_and_store_path(remote, database.path().to_path_buf());

    let mut current = request_with_id(query(Some(1)), "preview-current");
    current.protocol_minor = 15;
    let current_response = dispatcher.dispatch(current);
    assert!(current_response.ok, "{:?}", current_response.error);
    assert_eq!(
        current_response.result["result"]["scenes"][0]["browse_url"],
        "https://datapool.asf.alaska.edu/BROWSE/provider-preview.jpg"
    );

    let mut legacy = request_with_id(query(Some(1)), "preview-legacy");
    legacy.protocol_minor = 14;
    let legacy_response = dispatcher.dispatch(legacy);
    assert!(legacy_response.ok, "{:?}", legacy_response.error);
    let legacy_scene = legacy_response.result["result"]["scenes"][0]
        .as_object()
        .unwrap();
    assert!(!legacy_scene.contains_key("browse_url"));
    assert!(!legacy_scene.contains_key("provider_product_id"));
}

#[test]
fn cancellation_does_not_persist_a_partial_cache_entry() {
    let database = TempDatabase::new("cancel");
    let started = Arc::new(AtomicBool::new(false));
    let dispatcher = Arc::new(Dispatcher::with_asf_search_and_store_path(
        Arc::new(WaitingSearch {
            started: Arc::clone(&started),
        }),
        database.path().to_path_buf(),
    ));
    let worker = {
        let dispatcher = Arc::clone(&dispatcher);
        thread::spawn(move || {
            dispatcher.dispatch(request_with_id(query(Some(1)), "cached-cancel-target"))
        })
    };
    while !started.load(Ordering::SeqCst) {
        thread::yield_now();
    }
    let cancel = dispatcher.dispatch(SidecarRequest {
        protocol_major: 1,
        protocol_minor: 8,
        request_id: "cancel-command".to_string(),
        operation: SidecarOperation::Cancel,
        payload: Map::from_iter([(
            "request_id".to_string(),
            serde_json::Value::String("cached-cancel-target".to_string()),
        )]),
    });
    assert!(cancel.ok);
    let response = worker.join().unwrap();
    assert!(!response.ok);
    assert_eq!(response.error.unwrap().code, "DL001");

    let store = TaskStore::open(database.path()).unwrap();
    assert!(store
        .load_fresh_asf_cache_entries(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            86_400,
        )
        .unwrap()
        .is_empty());
}
