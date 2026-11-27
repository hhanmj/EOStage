use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

use geo::{Area, BoundingRect, Intersects, MultiPolygon};
use serde_json::{Map, Value};

use crate::longitude_envelope::align_polygonal_to_longitude_reference_for_intersection;
use crate::{
    align_polygonal_to_longitude_domain, align_polygonal_to_longitude_reference, normalize_filter,
    normalize_ranges, parse_sentinel1_identifier, validated_authoritative_polygonal_geojson,
    validated_polygonal_geojson, wrapped_aoi_envelope, AlignedPolygonalCopies,
    AsfCacheDecisionKind, AsfCacheDecisionRequest, AsfCacheDecisionResponse, AsfNormalizeError,
    AsfNormalizeRequest, AsfNormalizeResponse, AsfRawFeature, AsfSceneMetadata, AsfSearchQuery,
    BBox, ContractError, InclusiveRange, ProviderResult,
};

struct Candidate {
    source_index: usize,
    scene: AsfSceneMetadata,
    geometry: Option<MultiPolygon<f64>>,
}

struct AlignedAoi {
    geometry: MultiPolygon<f64>,
    display_west: f64,
    display_east: f64,
    area: f64,
}

fn contract_error(code: &str, message: &str, index: usize) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("index".to_string(), Value::from(index as u64))]),
    )
    .expect("bounded ASF error context is safe")
}

fn text_property(properties: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        properties.get(*name).and_then(|value| match value {
            Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
    })
}

fn text_or_first_array_property(properties: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        properties.get(*name).and_then(|value| match value {
            Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
            Value::Array(values) => values.iter().find_map(|item| {
                item.as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
            }),
            _ => None,
        })
    })
}

fn number_property(properties: &Map<String, Value>, names: &[&str]) -> Option<f64> {
    names.iter().find_map(|name| {
        properties.get(*name).and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
                .filter(|number| number.is_finite())
        })
    })
}

fn checksum_property(properties: &Map<String, Value>) -> (Option<String>, Option<String>) {
    if let Some(value) = text_property(properties, &["sha256", "sha256sum"]) {
        return (Some("sha256".to_string()), Some(value));
    }
    if let Some(value) = text_property(properties, &["md5sum", "md5"]) {
        return (Some("md5".to_string()), Some(value));
    }
    let algorithm = text_property(properties, &["checksumAlgorithm", "checksum_algorithm"])
        .map(|value| value.to_ascii_lowercase());
    let value = text_property(properties, &["checksumValue", "checksum_value", "checksum"]);
    if algorithm.is_some() == value.is_some() {
        (algorithm, value)
    } else {
        (None, None)
    }
}

fn unsigned_property(properties: &Map<String, Value>, names: &[&str]) -> Option<u64> {
    text_property(properties, names)
        .and_then(|value| value.parse::<f64>().ok())
        .and_then(|value| {
            if value.is_finite() && value >= 0.0 && value.fract() == 0.0 {
                Some(value as u64)
            } else {
                None
            }
        })
}

fn direction_property(properties: &Map<String, Value>) -> Option<String> {
    let value = text_property(
        properties,
        &["flightDirection", "orbitDirection", "ascendingDescending"],
    )?
    .to_ascii_uppercase();
    if value.starts_with("ASC") {
        Some("ASCENDING".to_string())
    } else if value.starts_with("DESC") {
        Some("DESCENDING".to_string())
    } else {
        None
    }
}

fn scene_source(properties: &Map<String, Value>) -> Option<String> {
    text_property(
        properties,
        &["sceneName", "fileName", "fileID", "granuleName", "url"],
    )
}

fn bbox_from_geometry(geometry: &MultiPolygon<f64>) -> Option<BBox> {
    let rect = geometry.bounding_rect()?;
    let bbox = BBox {
        west: rect.min().x,
        east: rect.max().x,
        south: rect.min().y,
        north: rect.max().y,
        crs: "EPSG:4326".to_string(),
    };
    bbox.validate().ok().map(|()| bbox)
}

fn normalize_feature(
    feature: &AsfRawFeature,
    index: usize,
) -> Result<Candidate, AsfNormalizeError> {
    let source = scene_source(&feature.properties).ok_or_else(|| AsfNormalizeError {
        index: index as u64,
        error: contract_error("ASF001", "ASF 结果缺少可识别的影像名称", index),
    })?;
    let parsed = parse_sentinel1_identifier(&source).map_err(|error| AsfNormalizeError {
        index: index as u64,
        error,
    })?;
    let geometry = feature
        .geometry
        .as_ref()
        .and_then(validated_polygonal_geojson);
    let footprint_bbox = geometry.as_ref().and_then(bbox_from_geometry);
    let url = text_property(&feature.properties, &["url"]).or_else(|| parsed.source_url.clone());
    let file_name = text_property(&feature.properties, &["fileName", "filename"]);
    let (checksum_algorithm, checksum_value) = checksum_property(&feature.properties);
    let acquisition_datetime = format!(
        "{}-{}-{}T{}:{}:{}+00:00",
        &parsed.acquisition_start[0..4],
        &parsed.acquisition_start[4..6],
        &parsed.acquisition_start[6..8],
        &parsed.acquisition_start[9..11],
        &parsed.acquisition_start[11..13],
        &parsed.acquisition_start[13..15]
    );
    Ok(Candidate {
        source_index: index,
        scene: AsfSceneMetadata {
            scene_id: parsed.scene_id,
            url,
            platform: parsed.platform,
            product_type: parsed.product_type,
            beam_mode: parsed.beam_mode,
            polarization: parsed.polarization,
            acquisition_datetime,
            absolute_orbit: unsigned_property(
                &feature.properties,
                &["orbit", "absoluteOrbit", "absoluteOrbitNumber"],
            )
            .or(Some(parsed.absolute_orbit)),
            path: unsigned_property(
                &feature.properties,
                &["pathNumber", "path", "relativeOrbit", "relativeOrbitNumber"],
            ),
            frame: unsigned_property(&feature.properties, &["frameNumber", "frame"]),
            orbit_direction: direction_property(&feature.properties),
            file_size_remote: unsigned_property(&feature.properties, &["bytes", "fileSize"]),
            footprint_bbox,
            footprint_geojson: feature.geometry.clone(),
            provider_product_id: text_property(
                &feature.properties,
                &["fileID", "productID", "granuleUR", "granuleName"],
            ),
            file_name,
            processing_level: text_property(
                &feature.properties,
                &["processingLevel", "processingType"],
            ),
            acquisition_stop: text_property(
                &feature.properties,
                &["stopTime", "acquisitionStop", "endTime"],
            ),
            browse_url: text_or_first_array_property(
                &feature.properties,
                &["browse", "browseUrl", "browse_url"],
            ),
            thumbnail_url: text_or_first_array_property(
                &feature.properties,
                &["thumbnail", "thumbnailUrl", "thumbnail_url"],
            ),
            checksum_algorithm,
            checksum_value,
            center_latitude: number_property(
                &feature.properties,
                &["centerLat", "centerLatitude", "center_latitude"],
            )
            .filter(|value| (-90.0..=90.0).contains(value)),
            center_longitude: number_property(
                &feature.properties,
                &["centerLon", "centerLongitude", "center_longitude"],
            )
            .filter(|value| (-180.0..=180.0).contains(value)),
        },
        geometry,
    })
}

fn filter_values(value: &str) -> HashSet<String> {
    value
        .split([',', ';', '|'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_ascii_uppercase)
        .collect()
}

fn matches_filters(candidate: &Candidate, request: &AsfNormalizeRequest) -> bool {
    let query = &request.query;
    let beam = filter_values(&query.beam_mode);
    let polarization = filter_values(&query.polarization);
    let direction = filter_values(&query.orbit_direction);
    (beam.is_empty() || beam.contains(&candidate.scene.beam_mode))
        && (polarization.is_empty() || polarization.contains(&candidate.scene.polarization))
        && (direction.is_empty()
            || candidate
                .scene
                .orbit_direction
                .as_ref()
                .is_some_and(|value| direction.contains(value)))
}

fn aoi_geometry(request: &AsfNormalizeRequest) -> ProviderResult<Option<AlignedAoi>> {
    let Some(value) = &request.query.aoi_geojson else {
        return Ok(None);
    };
    let invalid = || {
        ContractError::new("AOI001", "AOI 几何无效", Map::new())
            .expect("empty AOI error context is safe")
    };
    let geometry = validated_authoritative_polygonal_geojson(value)
        .map(|value| value.geometry)
        .ok_or_else(invalid)?;
    let Some(envelope) = wrapped_aoi_envelope(&geometry) else {
        return Ok(None);
    };
    let Some(geometry) = align_polygonal_to_longitude_domain(
        &geometry,
        envelope.display_west,
        envelope.display_east,
    ) else {
        return Ok(None);
    };
    let area = geometry.unsigned_area();
    if !area.is_finite() || area <= 0.0 {
        return Ok(None);
    }
    Ok(Some(AlignedAoi {
        geometry,
        display_west: envelope.display_west,
        display_east: envelope.display_east,
        area,
    }))
}

fn overlap_score(geometry: Option<&AlignedPolygonalCopies>, aoi: &AlignedAoi) -> (f64, f64) {
    let Some(geometry) = geometry else {
        return (0.0, 0.0);
    };
    let Some(overlap) = geometry.overlap_area(&aoi.geometry) else {
        return (0.0, 0.0);
    };
    if !overlap.is_finite() || overlap <= 0.0 {
        return (0.0, 0.0);
    }
    (overlap / aoi.area, overlap)
}

pub fn normalize_asf_features(
    request: &AsfNormalizeRequest,
) -> ProviderResult<AsfNormalizeResponse> {
    request.validate().map_err(|message| {
        ContractError::new("ASF002", message, Map::new())
            .expect("generated validation message is safe")
    })?;
    let aoi = aoi_geometry(request)?;
    let mut errors = Vec::new();
    let mut candidates = Vec::new();
    for (index, feature) in request.features.iter().enumerate() {
        match normalize_feature(feature, index) {
            Ok(candidate) => {
                if feature.geometry.is_some() && candidate.geometry.is_none() {
                    errors.push(AsfNormalizeError {
                        index: index as u64,
                        error: contract_error(
                            "ASF_GEOMETRY_INVALID",
                            "ASF 结果的足迹几何无效，已保留影像但不使用该足迹筛选",
                            index,
                        ),
                    });
                }
                if matches_filters(&candidate, request) {
                    candidates.push(candidate);
                }
            }
            Err(error) => errors.push(error),
        }
    }

    let mut seen = HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.scene.scene_id.clone()));
    if let Some(aoi) = &aoi {
        let mut aligned = candidates
            .into_iter()
            .map(|candidate| {
                let aligned_geometry = candidate.geometry.as_ref().and_then(|geometry| {
                    align_polygonal_to_longitude_reference(
                        geometry,
                        aoi.display_west,
                        aoi.display_east,
                    )
                });
                let filtering_copies = candidate.geometry.as_ref().and_then(|geometry| {
                    align_polygonal_to_longitude_reference_for_intersection(
                        geometry,
                        aoi.display_west,
                        aoi.display_east,
                    )
                });
                let projection_failed = candidate.geometry.is_some()
                    && (aligned_geometry.is_none() || filtering_copies.is_none());
                (
                    candidate,
                    aligned_geometry,
                    filtering_copies,
                    projection_failed,
                )
            })
            .collect::<Vec<_>>();
        if !aligned.iter().any(|entry| entry.3) {
            aligned.retain(|entry| {
                entry.0.geometry.as_ref().is_none_or(|_| {
                    entry.2.as_ref().is_some_and(|copies| {
                        catch_unwind(AssertUnwindSafe(|| {
                            copies
                                .iter()
                                .any(|polygon| polygon.intersects(&aoi.geometry))
                        }))
                        .unwrap_or(false)
                    })
                })
            });
            aligned.sort_by(|left, right| {
                let left_score = overlap_score(left.1.as_ref(), aoi);
                let right_score = overlap_score(right.1.as_ref(), aoi);
                right_score
                    .0
                    .total_cmp(&left_score.0)
                    .then_with(|| right_score.1.total_cmp(&left_score.1))
                    .then_with(|| left.0.source_index.cmp(&right.0.source_index))
            });
        }
        candidates = aligned
            .into_iter()
            .map(|(candidate, _, _, _)| candidate)
            .collect();
    }
    if let Some(limit) = request.query.max_results {
        candidates.truncate(limit as usize);
    }

    Ok(AsfNormalizeResponse {
        scenes: candidates
            .into_iter()
            .map(|candidate| candidate.scene)
            .collect(),
        errors,
    })
}

fn value_set(value: &str) -> HashSet<String> {
    filter_values(&normalize_filter(value))
}

fn filter_covers(cached: &str, requested: &str) -> bool {
    let cached = value_set(cached);
    let requested = value_set(requested);
    if requested.is_empty() {
        cached.is_empty()
    } else {
        cached.is_empty() || requested.is_subset(&cached)
    }
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

fn optional_ranges(value: &Option<String>) -> Vec<InclusiveRange> {
    normalize_ranges(value.as_deref().unwrap_or_default())
}

fn dates_cover(cached: &AsfSearchQuery, requested: &AsfSearchQuery) -> bool {
    if requested.start.is_empty() && !cached.start.is_empty() {
        return false;
    }
    if requested.end.is_empty() && !cached.end.is_empty() {
        return false;
    }
    if !cached.start.is_empty() && !requested.start.is_empty() && cached.start > requested.start {
        return false;
    }
    if !cached.end.is_empty() && !requested.end.is_empty() && cached.end < requested.end {
        return false;
    }
    true
}

fn cache_query_covers(cached: &AsfSearchQuery, requested: &AsfSearchQuery) -> bool {
    cached.version == requested.version
        && cached.dataset == requested.dataset
        && cached.bbox == requested.bbox
        && cached.aoi == requested.aoi
        && cached.product_type == requested.product_type
        && dates_cover(cached, requested)
        && filter_covers(&cached.beam_mode, &requested.beam_mode)
        && filter_covers(&cached.polarization, &requested.polarization)
        && filter_covers(&cached.orbit_direction, &requested.orbit_direction)
        && range_list_covers(
            &optional_ranges(&cached.relative_orbit),
            &optional_ranges(&requested.relative_orbit),
        )
        && range_list_covers(
            &optional_ranges(&cached.frame),
            &optional_ranges(&requested.frame),
        )
}

fn value_in_ranges(value: Option<u64>, ranges: &[InclusiveRange]) -> bool {
    if ranges.is_empty() {
        return true;
    }
    value.is_some_and(|value| {
        ranges
            .iter()
            .any(|range| range.start <= value && value <= range.end)
    })
}

fn scene_matches_cache_query(scene: &AsfSceneMetadata, query: &AsfSearchQuery) -> bool {
    let acquired = scene.acquisition_datetime.get(..10).unwrap_or_default();
    let start = query.start.get(..10).unwrap_or(query.start.as_str());
    let end = query.end.get(..10).unwrap_or(query.end.as_str());
    if !start.is_empty() && !acquired.is_empty() && acquired < start {
        return false;
    }
    if !end.is_empty() && !acquired.is_empty() && acquired > end {
        return false;
    }
    let beams = value_set(&query.beam_mode);
    let polarizations = value_set(&query.polarization);
    let directions = value_set(&query.orbit_direction);
    query.product_type.eq_ignore_ascii_case(&scene.product_type)
        && (beams.is_empty() || beams.contains(&scene.beam_mode.to_ascii_uppercase()))
        && (polarizations.is_empty()
            || polarizations.contains(&scene.polarization.to_ascii_uppercase()))
        && (directions.is_empty()
            || scene
                .orbit_direction
                .as_ref()
                .is_some_and(|value| directions.contains(&value.to_ascii_uppercase())))
        && value_in_ranges(scene.path, &optional_ranges(&query.relative_orbit))
        && value_in_ranges(scene.frame, &optional_ranges(&query.frame))
}

fn filtered_scenes(scenes: &[AsfSceneMetadata], query: &AsfSearchQuery) -> Vec<AsfSceneMetadata> {
    let mut scenes = scenes
        .iter()
        .filter(|scene| scene_matches_cache_query(scene, query))
        .cloned()
        .collect::<Vec<_>>();
    scenes.sort_by(|left, right| right.acquisition_datetime.cmp(&left.acquisition_datetime));
    scenes
}

pub fn decide_asf_cache(
    request: &AsfCacheDecisionRequest,
) -> ProviderResult<AsfCacheDecisionResponse> {
    request.validate().map_err(|message| {
        ContractError::new("ASF002", message, Map::new())
            .expect("generated validation message is safe")
    })?;
    let selected = request
        .entries
        .iter()
        .filter(|entry| cache_query_covers(&entry.query, &request.query))
        .map(|entry| (entry, filtered_scenes(&entry.scenes, &request.query)))
        .filter(|(_, scenes)| !scenes.is_empty())
        .max_by(|(left_entry, left_scenes), (right_entry, right_scenes)| {
            left_entry
                .complete
                .cmp(&right_entry.complete)
                .then_with(|| left_scenes.len().cmp(&right_scenes.len()))
                .then_with(|| left_entry.cache_key.cmp(&right_entry.cache_key))
        });

    let Some((entry, mut scenes)) = selected else {
        return Ok(AsfCacheDecisionResponse {
            kind: AsfCacheDecisionKind::Miss,
            cache_key: None,
            scenes: Vec::new(),
            missing_count: 0,
            incremental_start: None,
            incremental_end: None,
        });
    };

    let can_reuse = match request.query.max_results {
        Some(limit) => scenes.len() >= limit as usize,
        None => entry.complete,
    };
    if can_reuse {
        if let Some(limit) = request.query.max_results {
            scenes.truncate(limit as usize);
        }
        return Ok(AsfCacheDecisionResponse {
            kind: AsfCacheDecisionKind::Reuse,
            cache_key: Some(entry.cache_key.clone()),
            scenes,
            missing_count: 0,
            incremental_start: None,
            incremental_end: None,
        });
    }

    if let Some(limit) = request.query.max_results {
        let oldest = scenes
            .iter()
            .map(|scene| scene.acquisition_datetime.as_str())
            .min()
            .map(str::to_string);
        if let Some(incremental_end) = oldest {
            return Ok(AsfCacheDecisionResponse {
                kind: AsfCacheDecisionKind::Incremental,
                cache_key: Some(entry.cache_key.clone()),
                missing_count: limit.saturating_sub(scenes.len() as u64),
                scenes,
                incremental_start: Some(request.query.start.clone()),
                incremental_end: Some(incremental_end),
            });
        }
    }

    Ok(AsfCacheDecisionResponse {
        kind: AsfCacheDecisionKind::Miss,
        cache_key: None,
        scenes: Vec::new(),
        missing_count: 0,
        incremental_start: None,
        incremental_end: None,
    })
}
