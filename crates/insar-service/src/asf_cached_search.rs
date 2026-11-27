use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    asf_search_cache_key, decide_asf_cache, AsfCacheDecisionKind, AsfCacheDecisionRequest,
    AsfCacheEntry, AsfCachedSearchRequest, AsfCachedSearchResponse, AsfRemoteSearchRequest,
    AsfRemoteSearchResponse, AsfSceneMetadata, AsfSearchStats, ContractError, ProviderResult,
};
use insar_store::StoreError;
use serde_json::{Map, Value};

use crate::dispatcher::LiveAsfSearch;
use crate::store_dispatch::LazyTaskStore;

pub(crate) fn execute_cached_asf_search(
    provider: &dyn LiveAsfSearch,
    lazy_store: &Mutex<LazyTaskStore>,
    request: &AsfCachedSearchRequest,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ProviderResult<AsfCachedSearchResponse> {
    request.validate().map_err(|reason| {
        ContractError::new(
            "ASF002",
            "ASF 缓存检索请求无效",
            Map::from_iter([("reason".to_string(), Value::String(reason))]),
        )
        .expect("bounded cached-search context is safe")
    })?;
    check_cancelled(cancelled)?;
    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entries = {
        let mut guard = lazy_store.lock().map_err(|_| store_lock_error())?;
        guard
            .get()
            .and_then(|store| {
                store.load_fresh_asf_cache_entries(now_epoch, request.max_cache_age_seconds)
            })
            .map_err(store_error)?
    };
    let cache_key = asf_search_cache_key(&request.query)?;
    let decision = decide_asf_cache(&AsfCacheDecisionRequest {
        query: request.query.clone(),
        entries: entries.clone(),
    })?;
    check_cancelled(cancelled)?;

    match decision.kind {
        AsfCacheDecisionKind::Reuse => {
            let selected_key = decision.cache_key.clone().unwrap_or(cache_key);
            let selected = entries.iter().find(|entry| entry.cache_key == selected_key);
            let total_count = selected.and_then(|entry| entry.total_count);
            let result = cached_result(
                &request.query,
                decision.scenes,
                total_count,
                "RUST_ASF_CACHE",
            );
            Ok(AsfCachedSearchResponse {
                cache_count: selected.map_or(result.scenes.len() as u64, |entry| {
                    entry.scenes.len() as u64
                }),
                cache_kind: AsfCacheDecisionKind::Reuse,
                cache_key: selected_key,
                result,
            })
        }
        AsfCacheDecisionKind::Incremental => {
            let mut query = request.query.clone();
            query.max_results = Some(decision.missing_count.max(1));
            if let Some(start) = decision.incremental_start {
                query.start = start;
            }
            if let Some(end) = decision.incremental_end {
                query.end = end;
            }
            let remote = provider.search(
                &AsfRemoteSearchRequest {
                    query,
                    allow_cmr_fallback: request.allow_cmr_fallback,
                },
                cancelled,
            )?;
            check_cancelled(cancelled)?;
            let scenes = merge_scenes(decision.scenes, remote.scenes, request.query.max_results);
            let result = AsfRemoteSearchResponse {
                stats: AsfSearchStats {
                    requested_limit: request
                        .query
                        .max_results
                        .unwrap_or(scenes.len().max(1) as u64),
                    query_limit: scenes.len().max(1) as u64,
                    metadata_request_count: remote.stats.metadata_request_count,
                    total_count: None,
                    candidate_count: scenes.len() as u64,
                    returned_count: scenes.len() as u64,
                    source: "RUST_ASF_INCREMENTAL".to_string(),
                    cmr_enriched: remote.stats.cmr_enriched,
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
                errors: remote.errors,
            };
            persist(
                lazy_store,
                AsfCacheEntry {
                    cache_key: cache_key.clone(),
                    query: request.query.clone(),
                    scenes: result.scenes.clone(),
                    complete: false,
                    total_count: None,
                },
                now_epoch,
            )?;
            Ok(AsfCachedSearchResponse {
                cache_count: result.scenes.len() as u64,
                cache_kind: AsfCacheDecisionKind::Incremental,
                cache_key,
                result,
            })
        }
        AsfCacheDecisionKind::Miss => {
            let result = provider.search(
                &AsfRemoteSearchRequest {
                    query: request.query.clone(),
                    allow_cmr_fallback: request.allow_cmr_fallback,
                },
                cancelled,
            )?;
            check_cancelled(cancelled)?;
            let complete = result
                .stats
                .total_count
                .is_some_and(|total| result.scenes.len() as u64 >= total);
            persist(
                lazy_store,
                AsfCacheEntry {
                    cache_key: cache_key.clone(),
                    query: request.query.clone(),
                    scenes: result.scenes.clone(),
                    complete,
                    total_count: result.stats.total_count,
                },
                now_epoch,
            )?;
            Ok(AsfCachedSearchResponse {
                cache_count: result.scenes.len() as u64,
                cache_kind: AsfCacheDecisionKind::Miss,
                cache_key,
                result,
            })
        }
    }
}

fn cached_result(
    query: &insar_core::AsfSearchQuery,
    scenes: Vec<AsfSceneMetadata>,
    total_count: Option<u64>,
    source: &str,
) -> AsfRemoteSearchResponse {
    AsfRemoteSearchResponse {
        stats: AsfSearchStats {
            requested_limit: query.max_results.unwrap_or(scenes.len().max(1) as u64),
            query_limit: scenes.len().max(1) as u64,
            metadata_request_count: None,
            total_count,
            candidate_count: total_count.unwrap_or(scenes.len() as u64),
            returned_count: scenes.len() as u64,
            source: source.to_string(),
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
        errors: Vec::new(),
    }
}

fn merge_scenes(
    cached: Vec<AsfSceneMetadata>,
    remote: Vec<AsfSceneMetadata>,
    limit: Option<u64>,
) -> Vec<AsfSceneMetadata> {
    let mut scenes = cached
        .into_iter()
        .map(|scene| (scene.scene_id.clone(), scene))
        .collect::<BTreeMap<_, _>>();
    for scene in remote {
        scenes.insert(scene.scene_id.clone(), scene);
    }
    let mut scenes = scenes.into_values().collect::<Vec<_>>();
    scenes.sort_by(|left, right| {
        right
            .acquisition_datetime
            .cmp(&left.acquisition_datetime)
            .then_with(|| left.scene_id.cmp(&right.scene_id))
    });
    if let Some(limit) = limit.and_then(|value| usize::try_from(value).ok()) {
        scenes.truncate(limit);
    }
    scenes
}

fn persist(
    lazy_store: &Mutex<LazyTaskStore>,
    entry: AsfCacheEntry,
    now_epoch: u64,
) -> ProviderResult<()> {
    let mut guard = lazy_store.lock().map_err(|_| store_lock_error())?;
    guard
        .get()
        .and_then(|store| store.upsert_asf_cache_entry(&entry, now_epoch))
        .map_err(store_error)
}

fn check_cancelled(cancelled: &(dyn Fn() -> bool + Sync)) -> ProviderResult<()> {
    if cancelled() {
        Err(ContractError::new("DL001", "ASF 检索已停止", Map::new())
            .expect("empty cancellation context is safe"))
    } else {
        Ok(())
    }
}

fn store_lock_error() -> ContractError {
    ContractError::new(
        "V3_STORE_LOCK_POISONED",
        "Rust ASF 缓存存储锁异常",
        Map::new(),
    )
    .expect("empty store-lock context is safe")
}

fn store_error(error: StoreError) -> ContractError {
    ContractError::new(error.code(), "Rust ASF 缓存存储操作失败", Map::new())
        .expect("empty store-error context is safe")
}
