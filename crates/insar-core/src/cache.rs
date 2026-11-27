use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{
    canonical_geojson_hash, normalize_bbox, normalize_date, normalize_filter, normalize_ranges,
    AsfSearchQuery, CanonicalAsfQuery, ContractError, InclusiveRange, ProviderResult,
};

#[derive(Serialize)]
struct AsfSearchCacheIdentity {
    version: u64,
    dataset: String,
    bbox: Option<crate::CanonicalBbox>,
    aoi: String,
    start: String,
    end: String,
    product_type: String,
    beam_mode: String,
    polarization: String,
    orbit_direction: String,
    relative_orbit: String,
    frame: String,
}

fn normalized_range_text(value: Option<&str>) -> String {
    let mut ranges = normalize_ranges(value.unwrap_or_default());
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges
        .into_iter()
        .map(|range| {
            if range.start == range.end {
                range.start.to_string()
            } else {
                format!("{}-{}", range.start, range.end)
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub fn asf_search_cache_key(query: &AsfSearchQuery) -> ProviderResult<String> {
    query.validate().map_err(|reason| {
        ContractError::new(
            "ASF002",
            "ASF 缓存检索条件无效",
            Map::from_iter([("reason".to_string(), Value::String(reason))]),
        )
        .expect("bounded ASF cache-key context is safe")
    })?;
    let aoi = query
        .aoi_geojson
        .as_ref()
        .map(|geometry| canonical_geojson_hash(&Value::Object(geometry.clone())))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| query.aoi.trim().to_string());
    let identity = AsfSearchCacheIdentity {
        version: query.version,
        dataset: normalize_filter(&query.dataset),
        bbox: query
            .bbox
            .as_ref()
            .map(|bbox| normalize_bbox(bbox.west, bbox.east, bbox.south, bbox.north, &bbox.crs)),
        aoi,
        start: normalize_date(&query.start),
        end: normalize_date(&query.end),
        product_type: normalize_filter(&query.product_type),
        beam_mode: normalize_filter(&query.beam_mode),
        polarization: normalize_filter(&query.polarization),
        orbit_direction: normalize_filter(&query.orbit_direction),
        relative_orbit: normalized_range_text(query.relative_orbit.as_deref()),
        frame: normalized_range_text(query.frame.as_deref()),
    };
    let raw = serde_json::to_vec(&identity).expect("ASF cache identity always serializes");
    Ok(format!("{:x}", Sha256::digest(raw)))
}

fn filter_covers(cached: &str, requested: &str) -> bool {
    let cached = normalize_filter(cached)
        .split(',')
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let requested = normalize_filter(requested)
        .split(',')
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    if requested.is_empty() {
        return cached.is_empty();
    }
    cached.is_empty() || requested.is_subset(&cached)
}

fn range_list_covers(cached: &[InclusiveRange], requested: &[InclusiveRange]) -> bool {
    if requested.is_empty() {
        return cached.is_empty();
    }
    if cached.is_empty() {
        return true;
    }
    requested.iter().all(|requested| {
        cached
            .iter()
            .any(|cached| cached.start <= requested.start && cached.end >= requested.end)
    })
}

pub fn ranges_cover(cached: &str, requested: &str) -> bool {
    range_list_covers(&normalize_ranges(cached), &normalize_ranges(requested))
}

fn dates_cover(
    cached_start: &str,
    cached_end: &str,
    requested_start: &str,
    requested_end: &str,
) -> bool {
    if requested_start.is_empty() && !cached_start.is_empty() {
        return false;
    }
    if requested_end.is_empty() && !cached_end.is_empty() {
        return false;
    }
    if !cached_start.is_empty() && !requested_start.is_empty() && cached_start > requested_start {
        return false;
    }
    if !cached_end.is_empty() && !requested_end.is_empty() && cached_end < requested_end {
        return false;
    }
    true
}

pub fn query_covers(cached: &CanonicalAsfQuery, requested: &CanonicalAsfQuery) -> bool {
    cached.version == requested.version
        && cached.dataset == requested.dataset
        && cached.bbox == requested.bbox
        && cached.aoi == requested.aoi
        && cached.product_type == requested.product_type
        && dates_cover(&cached.start, &cached.end, &requested.start, &requested.end)
        && filter_covers(&cached.beam_mode, &requested.beam_mode)
        && filter_covers(&cached.polarization, &requested.polarization)
        && filter_covers(&cached.orbit_direction, &requested.orbit_direction)
        && ranges_cover(&cached.relative_orbit, &requested.relative_orbit)
        && ranges_cover(&cached.frame, &requested.frame)
}

pub fn cache_satisfies_limit(available: usize, complete: bool, requested: Option<usize>) -> bool {
    match requested {
        Some(limit) => available >= limit,
        None => complete && available > 0,
    }
}
