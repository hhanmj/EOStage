use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{
    canonical_geojson_hash, canonicalized_authoritative_polygonal_geojson, normalize_bbox,
    normalize_date, normalize_filter, CatalogSearchQuery, ContractError, FrozenSearchBatch,
    ProviderResult, SarBurstSelection, SarFilterRange, SarSearchQuery,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SarSearchCacheDecision {
    Reuse,
    Incremental {
        older_than: String,
        missing_results: u64,
    },
    Miss,
}

#[derive(Serialize)]
struct SarSearchCacheIdentity {
    version: u64,
    /// Search-route generation. The restricted legacy missions (ERS-1/2,
    /// RADARSAT-1, JERS-1) moved from the anonymous ASF `/param` endpoint to
    /// authenticated CMR; caches frozen by the older route (which could only
    /// ever hold an anonymous zero-row answer) must not be reused by the new
    /// route. Bumping this per-dataset discriminator changes only those
    /// datasets' cache keys; all other datasets keep their existing keys.
    route: u64,
    provider: String,
    dataset: String,
    platforms: Vec<String>,
    instrument: String,
    processing_levels: Vec<String>,
    product_families: Vec<String>,
    beam_modes: Vec<String>,
    polarizations: Vec<String>,
    flight_direction: String,
    look_direction: String,
    relative_orbit: Option<i64>,
    frame: Option<i64>,
    asf_frame: String,
    path: Option<i64>,
    start: String,
    end: String,
    track_range: Option<(String, String)>,
    frame_range: Option<(String, String)>,
    aoi_geojson_hash: String,
    aoi_wkt: String,
    bbox: Option<crate::CanonicalBbox>,
    target_when_aoi: Option<u64>,
    burst: Option<SarBurstCacheIdentity>,
}

#[derive(Serialize)]
struct SarBurstCacheIdentity {
    burst_id: String,
    full_burst_id: String,
    relative_burst_id: Option<i64>,
    absolute_burst_id: Option<i64>,
    subswath: String,
    burst_index: Option<i64>,
}

#[derive(Serialize)]
struct CatalogSearchCacheIdentity {
    family: &'static str,
    version: u64,
    logical_dataset: String,
    provider_id: String,
    native_collection_id: String,
    aoi_geojson_hash: String,
    cursor: String,
    result_limit: Option<u64>,
    filters: Value,
}

fn cache_error(
    code: &'static str,
    message: &'static str,
    reason: impl Into<String>,
) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.into()))]),
    )
    .expect("bounded SAR cache error context is safe")
}

fn normalized_set(values: &[String]) -> Vec<String> {
    let mut values = values
        .iter()
        .map(|value| normalize_filter(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn normalized_range(value: Option<&SarFilterRange>) -> Option<(String, String)> {
    value.map(|range| {
        (
            range
                .start
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .to_owned(),
            range
                .end
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .to_owned(),
        )
    })
}

fn normalized_burst(value: Option<&SarBurstSelection>) -> Option<SarBurstCacheIdentity> {
    value.map(|burst| SarBurstCacheIdentity {
        burst_id: normalize_filter(burst.burst_id.as_deref().unwrap_or_default()),
        full_burst_id: normalize_filter(burst.full_burst_id.as_deref().unwrap_or_default()),
        relative_burst_id: burst.relative_burst_id,
        absolute_burst_id: burst.absolute_burst_id,
        subswath: normalize_filter(burst.subswath.as_deref().unwrap_or_default()),
        burst_index: burst.burst_index,
    })
}

fn has_aoi(query: &SarSearchQuery) -> bool {
    query.aoi_geojson.is_some() || query.aoi_wkt.is_some() || query.bbox.is_some()
}

pub fn catalog_search_cache_key(query: &CatalogSearchQuery) -> ProviderResult<String> {
    query.validate().map_err(|reason| {
        cache_error(
            "CATALOG_CACHE_QUERY_INVALID",
            "目录缓存检索条件无效",
            reason,
        )
    })?;
    let aoi_geojson_hash = match query.aoi_geojson.as_ref() {
        Some(geometry) => canonicalized_authoritative_polygonal_geojson(geometry)
            .map(|geometry| canonical_geojson_hash(&Value::Object(geometry.geojson)))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                cache_error(
                    "CATALOG_CACHE_QUERY_INVALID",
                    "目录缓存 AOI 无效",
                    "aoi_geojson",
                )
            })?,
        None => String::new(),
    };
    let identity = CatalogSearchCacheIdentity {
        family: "typed_catalogue",
        // 2: reviewed STAC sources sort dated searches newest-first.
        version: 2,
        logical_dataset: normalize_filter(&query.logical_dataset),
        provider_id: normalize_filter(&query.provider_id),
        native_collection_id: normalize_filter(&query.native_collection_id),
        aoi_geojson_hash,
        cursor: query
            .cursor
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned(),
        result_limit: query.result_limit,
        filters: Value::Object(query.filters.clone()),
    };
    let raw = serde_json::to_vec(&identity).expect("catalogue cache identity always serializes");
    Ok(format!("{:x}", Sha256::digest(raw)))
}

/// Cache-route generation per dataset selector. The legacy restricted
/// missions (ERS, JERS-1, RADARSAT-1) moved to generation 2 with authenticated
/// CMR, and SENTINEL-1 moved to generation 2 when live 2026 CMR fallback was
/// expanded to Sentinel-1C/1D (`C3294057315-ASF` / `C3873123999-ASF`) with
/// attribute post-filtering so pre-fix zero-row caches are re-keyed; every
/// other dataset stays on generation 1 so existing cache rows remain valid.
fn legacy_cmr_route_generation(dataset: &str) -> u64 {
    let trimmed = dataset.trim();
    if ["ERS", "JERS-1", "RADARSAT-1", "SENTINEL-1"]
        .iter()
        .any(|selector| selector.eq_ignore_ascii_case(trimmed))
    {
        2
    } else {
        1
    }
}

pub fn sar_search_cache_key(query: &SarSearchQuery) -> ProviderResult<String> {
    query
        .validate()
        .map_err(|reason| cache_error("SAR_CACHE_QUERY_INVALID", "SAR 缓存检索条件无效", reason))?;
    Ok(cache_key_with_route(
        query,
        legacy_cmr_route_generation(&query.dataset),
    ))
}

/// The cache-key digest for one explicit route generation. `sar_search_cache_key`
/// derives the generation from the dataset; tests pass the pre-CMR generation
/// directly to prove the discriminator alone re-keys the restricted missions.
fn cache_key_with_route(query: &SarSearchQuery, route: u64) -> String {
    let aoi_geojson_hash = match query.aoi_geojson.as_ref() {
        Some(geometry) => canonicalized_authoritative_polygonal_geojson(geometry)
            .map(|geometry| canonical_geojson_hash(&Value::Object(geometry.geojson)))
            .filter(|value| !value.is_empty())
            .expect("validated query carries a canonical AOI hash"),
        None => String::new(),
    };
    let identity = SarSearchCacheIdentity {
        version: 1,
        route,
        provider: normalize_filter(&query.provider),
        dataset: normalize_filter(&query.dataset),
        platforms: normalized_set(&query.platforms),
        instrument: normalize_filter(query.instrument.as_deref().unwrap_or_default()),
        processing_levels: normalized_set(&query.processing_levels),
        product_families: normalized_set(&query.product_families),
        beam_modes: normalized_set(&query.beam_modes),
        polarizations: normalized_set(&query.polarizations),
        flight_direction: normalize_filter(query.flight_direction.as_deref().unwrap_or_default()),
        look_direction: normalize_filter(query.look_direction.as_deref().unwrap_or_default()),
        relative_orbit: query.relative_orbit,
        frame: query.frame,
        asf_frame: query
            .asf_frame
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned(),
        path: query.path,
        start: normalize_date(query.start.as_deref().unwrap_or_default()),
        end: normalize_date(query.end.as_deref().unwrap_or_default()),
        track_range: normalized_range(query.track_range.as_ref()),
        frame_range: normalized_range(query.frame_range.as_ref()),
        aoi_geojson_hash,
        aoi_wkt: query
            .aoi_wkt
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned(),
        bbox: query
            .bbox
            .as_ref()
            .map(|bbox| normalize_bbox(bbox.west, bbox.east, bbox.south, bbox.north, &bbox.crs)),
        target_when_aoi: has_aoi(query).then_some(query.max_results).flatten(),
        burst: normalized_burst(query.burst.as_ref()),
    };
    let raw = serde_json::to_vec(&identity).expect("SAR cache identity always serializes");
    format!("{:x}", Sha256::digest(raw))
}

fn canonical_acquisition(value: &str) -> Option<(DateTime<Utc>, String)> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .ok()?
        .with_timezone(&Utc);
    let canonical = parsed.to_rfc3339_opts(SecondsFormat::AutoSi, true);
    (canonical == value).then_some((parsed, canonical))
}

pub fn decide_sar_search_cache(
    requested: &SarSearchQuery,
    cached: &FrozenSearchBatch,
    complete: bool,
) -> ProviderResult<SarSearchCacheDecision> {
    cached
        .validate()
        .map_err(|reason| cache_error("SAR_CACHE_BATCH_INVALID", "SAR 缓存批次无效", reason))?;
    let cached_query: SarSearchQuery = serde_json::from_value(Value::Object(cached.query.clone()))
        .map_err(|_| cache_error("SAR_CACHE_BATCH_INVALID", "SAR 缓存批次查询无效", "query"))?;
    if sar_search_cache_key(requested)? != sar_search_cache_key(&cached_query)? {
        return Ok(SarSearchCacheDecision::Miss);
    }
    // Records that were fetched but could not be mapped are a provider error,
    // not an empty answer; an empty batch frozen that way is searched again.
    if cached.candidates.is_empty()
        && cached.search_stats.as_ref().is_some_and(|stats| {
            matches!(
                (stats.fetched_record_count, stats.mapped_record_count),
                (Some(fetched), Some(mapped)) if mapped < fetched
            )
        })
    {
        return Ok(SarSearchCacheDecision::Miss);
    }

    let Some(requested_target) = requested.max_results else {
        return Ok(if complete {
            SarSearchCacheDecision::Reuse
        } else {
            SarSearchCacheDecision::Miss
        });
    };
    let available = u64::try_from(cached.candidates.len()).unwrap_or(u64::MAX);
    if complete || available >= requested_target {
        return Ok(SarSearchCacheDecision::Reuse);
    }
    if has_aoi(requested) || cached.candidates.is_empty() {
        return Ok(SarSearchCacheDecision::Miss);
    }

    let mut previous: Option<DateTime<Utc>> = None;
    let mut oldest = None;
    for candidate in &cached.candidates {
        let Some(value) = candidate.acquisition_datetime.as_deref() else {
            return Ok(SarSearchCacheDecision::Miss);
        };
        let Some((acquisition, canonical)) = canonical_acquisition(value) else {
            return Ok(SarSearchCacheDecision::Miss);
        };
        if previous.is_some_and(|previous| previous <= acquisition) {
            return Ok(SarSearchCacheDecision::Miss);
        }
        previous = Some(acquisition);
        oldest = Some(canonical);
    }
    Ok(SarSearchCacheDecision::Incremental {
        older_than: oldest.expect("non-empty canonical candidate list"),
        missing_results: requested_target - available,
    })
}

#[cfg(test)]
mod route_generation_tests {
    use super::*;

    fn legacy_query(dataset: &str) -> SarSearchQuery {
        serde_json::from_value(serde_json::json!({
            "provider": "ASF",
            "dataset": dataset,
            "start": "1995-01-01T00:00:00Z",
            "end": "2000-12-31T23:59:59Z",
            "max_results": 250,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[
                    [-147.82, 64.78], [-147.52, 64.78], [-147.52, 64.88],
                    [-147.82, 64.88], [-147.82, 64.78]
                ]]
            }
        }))
        .expect("valid legacy SAR query")
    }

    /// The restricted missions moved from the anonymous `/param` route to
    /// authenticated CMR (2026-10-31). Their caches — which under the old
    /// route could only ever hold an anonymous zero-row answer frozen as
    /// complete — must be re-keyed, so a stored pre-fix row can never satisfy
    /// `decide_sar_search_cache` for the new route. All other datasets keep
    /// their generation-1 keys so unrelated caches stay valid.
    #[test]
    fn restricted_missions_are_rekeyed_and_other_datasets_keep_their_keys() {
        for dataset in [
            "ERS",
            "JERS-1",
            "RADARSAT-1",
            "SENTINEL-1",
            " jers-1 ",
            " sentinel-1 ",
        ] {
            let query = legacy_query(dataset);
            let current = sar_search_cache_key(&query).expect("current route-2 key");
            let pre_cmr = cache_key_with_route(&query, 1);
            assert_ne!(
                current, pre_cmr,
                "{dataset}: route-2 migration must change the cache key"
            );
        }

        for control in ["SEASAT", "ALOS PALSAR", "NISAR"] {
            let query = legacy_query(control);
            let current = sar_search_cache_key(&query).expect("control key");
            let generation_one = cache_key_with_route(&query, 1);
            assert_eq!(
                current, generation_one,
                "{control}: an unaffected dataset must keep its generation-1 key"
            );
        }
    }

    /// The store caches SAR batches by `cache_key` (see
    /// `TaskStore::load_sar_cache_decision`: `WHERE cache_key = ?1`). A row
    /// the pre-fix route wrote carries the route-1 key; the updated route
    /// computes the route-2 key, so the lookup misses and a fresh provider
    /// search runs. This test proves the two keys differ — the exact condition
    /// that makes the stored row invisible to the new route.
    #[test]
    fn a_pre_cmr_cache_row_key_never_matches_the_authenticated_route_key() {
        for dataset in ["ERS", "JERS-1", "RADARSAT-1", "SENTINEL-1"] {
            let query = legacy_query(dataset);
            let authenticated_route_key =
                sar_search_cache_key(&query).expect("authenticated route key");
            let pre_cmr_route_key = cache_key_with_route(&query, 1);
            assert_ne!(
                authenticated_route_key, pre_cmr_route_key,
                "{dataset}: a pre-fix route-1 cache row must not match"
            );
        }
    }
}
