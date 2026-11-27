use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use geo::{BoundingRect, LineString, MultiPolygon, Polygon};
use insar_core::{
    asf_catalog_archive_window, asf_catalog_capabilities,
    validated_authoritative_polygonal_geojson, wrapped_aoi_envelope, AsfSearchQuery, BBox,
    CatalogSearchQuery, ContractError, DatasetCapabilities, FilterDescriptor, FilterValueType,
    ProviderResult, SarFilterRange, SarSearchQuery,
};
use serde_json::{Map, Value};

use crate::catalog::{
    alos2_expected_polarizations,
    spatial::footprint_intersects_exact_catalogue_aoi as shared_exact_catalogue_intersection,
    CollectionProfile, DatasetRoutePolicyRegistry, ALOS2_COLLECTION_SHORT_NAME,
    ALOS2_FILENAME_RULE_ID, ALOS2_IDENTITY_RULE_ID, ALOS2_LOGICAL_DATASET,
    ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID, ALOS2_RESOLVER_POLICY_ID,
    ALOS2_SCAN_SAR_BEAM_MODES,
};

const MAX_CATALOG_RESULTS: u64 = 10_000;
const SPLIT_WINDOW_DAYS: i64 = 180;
const ARCHIVE_START: &str = "2014-10-01T00:00:00Z";
const CMR_MAX_TRANSPORT_VERTICES: usize = 96;
const CMR_MAX_TRANSPORT_PARTS: usize = 8;
const CMR_TARGET_TILE_LONGITUDE_SPAN: f64 = 30.0;
const CMR_TARGET_TILE_LATITUDE_SPAN: f64 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AoiPlanMode {
    None,
    Exact,
    Simplified,
    Bbox,
}

impl AoiPlanMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Exact => "exact",
            Self::Simplified => "simplified",
            Self::Bbox => "bbox",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsfQueryPlan {
    pub params: BTreeMap<String, String>,
    pub requested_limit: u64,
    pub query_limit: u64,
    pub should_count: bool,
    pub aoi_mode: AoiPlanMode,
    pub requested_was_unbounded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryWindow {
    pub start: String,
    pub end: String,
}

pub const ASF_CMR_SINGLE_PART_PAGE_SIZE: u64 = 250;
pub const ASF_CMR_MULTI_PART_PAGE_SIZE: u64 = 50;

pub(crate) fn catalogue_transport_page_size(
    result_limit: Option<u64>,
    is_multi_polygon: bool,
) -> u64 {
    let adapter_maximum = if is_multi_polygon {
        ASF_CMR_MULTI_PART_PAGE_SIZE
    } else {
        ASF_CMR_SINGLE_PART_PAGE_SIZE
    };
    result_limit.unwrap_or(adapter_maximum).min(adapter_maximum)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AsfCmrQuery {
    pub common_params: Vec<(String, String)>,
    pub canonical_geometry: Option<MultiPolygon<f64>>,
    pub polygons: Vec<String>,
    pub result_limit: Option<u64>,
    pub provider_page_size: u64,
    pub is_multi_polygon: bool,
    pub complete_batch_local_filters: Vec<(String, String)>,
    pub spatial_transport_mode: &'static str,
    pub requires_exact_spatial_post_filter: bool,
    pub canonical_polygon_count: usize,
    pub canonical_ring_count: usize,
    pub canonical_vertex_count: usize,
    pub transport_part_count: usize,
    pub transport_vertex_counts: Vec<usize>,
}

impl AsfCmrQuery {
    pub fn page_params(
        &self,
        polygon: &str,
        page_number: u64,
        provider_page_size: u64,
    ) -> Vec<(String, String)> {
        let mut params = self.common_params.clone();
        if let Some((_, value)) = params.iter_mut().find(|(key, _)| key == "page_size") {
            *value = provider_page_size.to_string();
        } else {
            params.push(("page_size".to_owned(), provider_page_size.to_string()));
        }
        // Stable sort keys plus a fixed page size make page_num deterministic.
        // The current HTTP seam does not expose Search-After response headers;
        // bounded round-robin paging does not require expanding that seam.
        params.push(("page_num".to_owned(), page_number.to_string()));
        if !polygon.is_empty() {
            params.push(("polygon".to_owned(), polygon.to_owned()));
        }
        params
    }
}

/// Translate the closed ALOS-2 catalogue query into typed CMR parameters.
pub fn build_alos2_cmr_query(
    query: &CatalogSearchQuery,
    profile: &CollectionProfile,
) -> ProviderResult<AsfCmrQuery> {
    let query = &insar_core::clamp_catalogue_search_query(query);
    query
        .validate()
        .map_err(|reason| catalogue_query_error("CATALOGUE_QUERY_INVALID", "query", reason))?;
    if query.cursor.is_some() {
        return Err(catalogue_query_error(
            "CATALOGUE_CURSOR_UNSUPPORTED",
            "cursor",
            "external_continuation",
        ));
    }
    if query.logical_dataset != ALOS2_LOGICAL_DATASET
        || query.provider_id != ALOS2_PROVIDER_ID
        || query.native_collection_id != ALOS2_NATIVE_COLLECTION_ID
        || profile.profile_id != ALOS2_PROFILE_ID
        || profile.logical_dataset != ALOS2_LOGICAL_DATASET
        || profile.provider_id != ALOS2_PROVIDER_ID
        || profile.native_collection_id != ALOS2_NATIVE_COLLECTION_ID
        || profile.required_product_type != ALOS2_COLLECTION_SHORT_NAME
        || profile.identity_rule_id != ALOS2_IDENTITY_RULE_ID
        || profile.filename_rule_id != ALOS2_FILENAME_RULE_ID
        || profile.resolver_policy_id != ALOS2_RESOLVER_POLICY_ID
    {
        return Err(catalogue_query_error(
            "ASF_COLLECTION_IDENTITY_MISMATCH",
            "collection",
            "profile_or_query",
        ));
    }
    let filter_keys = query.filters.keys().map(String::as_str).collect::<Vec<_>>();
    DatasetRoutePolicyRegistry
        .validate_filter_keys(ALOS2_LOGICAL_DATASET, &filter_keys)
        .map_err(|_| {
            catalogue_query_error(
                "CATALOGUE_FILTER_UNSUPPORTED",
                "filters",
                "unsupported_filter",
            )
        })?;

    let spatial_plan = match query.aoi_geojson.as_ref() {
        Some(aoi_geojson) => catalogue_spatial_plan(aoi_geojson)?,
        None => CatalogueSpatialPlan::empty(),
    };
    let polygons = spatial_plan.polygons.clone();
    let is_multi_polygon = polygons.len() > 1;
    let provider_page_size = catalogue_transport_page_size(query.result_limit, is_multi_polygon);
    if query.aoi_geojson.is_some() && polygons.is_empty() {
        return Err(catalogue_query_error(
            "CATALOGUE_QUERY_INVALID",
            "aoi_geojson",
            "empty_polygon_plan",
        ));
    }

    let mut params = vec![
        (
            "collection_concept_id".to_owned(),
            ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        ),
        ("page_size".to_owned(), provider_page_size.to_string()),
        ("sort_key[]".to_owned(), "start_date".to_owned()),
        ("sort_key[]".to_owned(), "granule_ur".to_owned()),
    ];
    if let Some(value) = query.filters.get("datetime") {
        params.push(("temporal".to_owned(), catalogue_temporal(value)?));
    }

    let beam_mode = optional_catalogue_string(&query.filters, "beam_mode")?
        .map(|value| value.to_ascii_uppercase());
    if let Some(beam_mode) = beam_mode.as_deref() {
        if !ALOS2_SCAN_SAR_BEAM_MODES.contains(&beam_mode) {
            return Err(catalogue_filter_error("beam_mode"));
        }
        push_cmr_attribute(&mut params, "BEAM_MODE", beam_mode);
    }

    let polarizations =
        optional_catalogue_strings(&query.filters, "polarizations")?.map(|values| {
            let mut values = values
                .into_iter()
                .map(|value| value.to_ascii_uppercase())
                .collect::<Vec<_>>();
            values.sort();
            values.dedup();
            values
        });
    if let Some(polarizations) = polarizations.as_ref() {
        let supported = matches!(polarizations.as_slice(), [value] if value == "HH")
            || polarizations.as_slice() == ["HH", "HV"];
        if !supported
            || beam_mode
                .as_deref()
                .and_then(alos2_expected_polarizations)
                .is_some_and(|expected| expected != polarizations.as_slice())
        {
            return Err(catalogue_filter_error("polarizations"));
        }
        for polarization in polarizations {
            push_cmr_attribute(&mut params, "POLARIZATION", polarization);
        }
    }

    for (key, attribute, allowed) in [
        (
            "flight_direction",
            "ASCENDING_DESCENDING",
            &["ASCENDING", "DESCENDING"][..],
        ),
        ("look_direction", "LOOK_DIRECTION", &["LEFT", "RIGHT"][..]),
    ] {
        if let Some(value) = optional_catalogue_string(&query.filters, key)? {
            let value = value.to_ascii_uppercase();
            if !allowed.contains(&value.as_str()) {
                return Err(catalogue_filter_error(key));
            }
            push_cmr_attribute(&mut params, attribute, &value);
        }
    }
    for (key, attribute) in [("path", "PATH_NUMBER"), ("frame", "FRAME_NUMBER")] {
        if let Some(value) = optional_catalogue_integer(&query.filters, key)? {
            if value == 0 {
                return Err(catalogue_filter_error(key));
            }
            push_cmr_integer_attribute(&mut params, attribute, value);
        }
    }

    Ok(AsfCmrQuery {
        common_params: params,
        canonical_geometry: spatial_plan.canonical_geometry,
        polygons,
        result_limit: query.result_limit,
        provider_page_size,
        is_multi_polygon,
        complete_batch_local_filters: Vec::new(),
        spatial_transport_mode: spatial_plan.mode,
        requires_exact_spatial_post_filter: spatial_plan.requires_exact_post_filter,
        canonical_polygon_count: spatial_plan.canonical_polygon_count,
        canonical_ring_count: spatial_plan.canonical_ring_count,
        canonical_vertex_count: spatial_plan.canonical_vertex_count,
        transport_part_count: spatial_plan.transport_part_count,
        transport_vertex_counts: spatial_plan.transport_vertex_counts,
    })
}

pub(crate) fn push_cmr_attribute(params: &mut Vec<(String, String)>, name: &str, value: &str) {
    params.push(("attribute[]".to_owned(), format!("string,{name},{value}")));
}

pub(crate) fn push_cmr_integer_attribute(
    params: &mut Vec<(String, String)>,
    name: &str,
    value: u64,
) {
    params.push(("attribute[]".to_owned(), format!("int,{name},{value}")));
}

pub(crate) fn optional_catalogue_string(
    filters: &Map<String, Value>,
    key: &'static str,
) -> ProviderResult<Option<String>> {
    let Some(value) = filters.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .filter(|value| !value.is_empty() && value == &value.trim() && value.len() <= 128)
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| catalogue_filter_error(key))
}

fn optional_catalogue_strings(
    filters: &Map<String, Value>,
    key: &'static str,
) -> ProviderResult<Option<Vec<String>>> {
    let Some(value) = filters.get(key) else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| catalogue_filter_error(key))?;
    if values.is_empty() || values.len() > 8 {
        return Err(catalogue_filter_error(key));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty() && value == &value.trim() && value.len() <= 128)
                .map(str::to_owned)
                .ok_or_else(|| catalogue_filter_error(key))
        })
        .collect::<ProviderResult<Vec<_>>>()
        .map(Some)
}

pub(crate) fn optional_catalogue_integer(
    filters: &Map<String, Value>,
    key: &'static str,
) -> ProviderResult<Option<u64>> {
    let Some(value) = filters.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| catalogue_filter_error(key))
}

pub(crate) fn catalogue_temporal(value: &Value) -> ProviderResult<String> {
    let value = value
        .as_object()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 2
                && value.keys().all(|key| key == "start" || key == "end")
        })
        .ok_or_else(|| catalogue_filter_error("datetime"))?;
    let parse = |key: &'static str| -> ProviderResult<Option<DateTime<Utc>>> {
        let Some(raw) = value.get(key) else {
            return Ok(None);
        };
        let raw = raw
            .as_str()
            .filter(|raw| raw.len() <= 64 && raw.ends_with('Z'))
            .ok_or_else(|| catalogue_filter_error("datetime"))?;
        DateTime::parse_from_rfc3339(raw)
            .map(|value| value.with_timezone(&Utc))
            .map(Some)
            .map_err(|_| catalogue_filter_error("datetime"))
    };
    let start = parse("start")?;
    let end = parse("end")?;
    if matches!((&start, &end), (Some(start), Some(end)) if start > end) {
        return Err(catalogue_filter_error("datetime"));
    }
    Ok(format!(
        "{},{}",
        start
            .map(|value| value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
            .unwrap_or_default(),
        end.map(|value| value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
            .unwrap_or_default()
    ))
}

#[cfg(test)]
mod catalogue_temporal_tests {
    use super::catalogue_temporal;
    use serde_json::json;

    #[test]
    fn accepts_one_sided_catalogue_intervals() {
        assert_eq!(
            catalogue_temporal(&json!({"start": "2026-01-01T00:00:00Z"})).unwrap(),
            "2026-01-01T00:00:00Z,"
        );
        assert_eq!(
            catalogue_temporal(&json!({"end": "2026-01-31T23:59:59Z"})).unwrap(),
            ",2026-01-31T23:59:59Z"
        );
    }

    #[test]
    fn rejects_empty_malformed_and_reversed_catalogue_intervals() {
        assert!(catalogue_temporal(&json!({})).is_err());
        assert!(catalogue_temporal(&json!({"start": "not-a-date"})).is_err());
        assert!(catalogue_temporal(&json!({
            "start": "2026-02-02T00:00:00Z",
            "end": "2026-02-01T23:59:59Z"
        }))
        .is_err());
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CatalogueSpatialPlan {
    pub canonical_geometry: Option<MultiPolygon<f64>>,
    pub polygons: Vec<String>,
    pub mode: &'static str,
    pub requires_exact_post_filter: bool,
    pub transport_part_count: usize,
    pub canonical_polygon_count: usize,
    pub canonical_ring_count: usize,
    pub canonical_vertex_count: usize,
    pub transport_vertex_counts: Vec<usize>,
}

impl CatalogueSpatialPlan {
    pub(crate) fn empty() -> Self {
        Self {
            canonical_geometry: None,
            polygons: Vec::new(),
            mode: "none",
            requires_exact_post_filter: false,
            transport_part_count: 0,
            canonical_polygon_count: 0,
            canonical_ring_count: 0,
            canonical_vertex_count: 0,
            transport_vertex_counts: Vec::new(),
        }
    }
}

/// Keep the canonical AOI immutable while producing a provider-safe transport.
/// A simple safe polygon is sent exactly. Holes, MultiPolygons, high-vertex and
/// national-scale AOIs use a small conservative envelope plan; every candidate
/// is then checked against the canonical AOI by the catalogue client.
pub(crate) fn catalogue_spatial_plan(
    aoi: &Map<String, Value>,
) -> ProviderResult<CatalogueSpatialPlan> {
    let geometry = catalogue_authoritative_geometry(aoi).ok_or_else(|| {
        catalogue_query_error(
            "CATALOGUE_FILTER_INVALID",
            "aoi_geojson",
            "authoritative_polygon_invalid",
        )
    })?;
    let canonical_polygon_count = geometry.0.len();
    let canonical_ring_count = geometry
        .0
        .iter()
        .map(|polygon| 1 + polygon.interiors().len())
        .sum();
    let canonical_vertex_count = geometry.0.iter().map(polygon_vertex_count).sum();
    let wrapped_envelope = wrapped_aoi_envelope(&geometry);
    let request_windows = wrapped_envelope
        .as_ref()
        .map(|envelope| envelope.request_windows.clone())
        .or_else(|| native_catalogue_bbox(&geometry).map(|bbox| vec![bbox]))
        .ok_or_else(|| {
            catalogue_query_error(
                "CATALOGUE_FILTER_INVALID",
                "aoi_geojson",
                "authoritative_envelope_invalid",
            )
        })?;
    let is_simple_safe_polygon = geometry.0.len() == 1
        && geometry.0[0].interiors().is_empty()
        && geometry.0[0].exterior().0.len() <= CMR_MAX_TRANSPORT_VERTICES
        && wrapped_envelope
            .as_ref()
            .is_some_and(|envelope| envelope.request_windows.len() == 1);

    let (parts, mode, requires_exact_post_filter) = if is_simple_safe_polygon {
        (vec![geometry.0[0].clone()], "cmr_exact_polygon", false)
    } else {
        let parts = conservative_transport_tiles(request_windows)?;
        let mode = if parts.len() == 1 {
            "cmr_conservative_bbox_exact_post_filter"
        } else {
            "cmr_tiled_envelope_exact_post_filter"
        };
        (parts, mode, true)
    };
    if parts.is_empty() || parts.len() > CMR_MAX_TRANSPORT_PARTS {
        return Err(catalogue_query_error(
            "CATALOGUE_SPATIAL_PLAN_EXCEEDED",
            "aoi_geojson",
            "transport_part_limit",
        ));
    }
    let mut encoded = Vec::with_capacity(parts.len());
    let mut transport_vertex_counts = Vec::with_capacity(parts.len());
    for polygon in parts {
        let values = polygon
            .exterior()
            .0
            .iter()
            .map(|coordinate| (coordinate.x, coordinate.y))
            .collect::<Vec<_>>();
        transport_vertex_counts.push(values.len());
        encoded.push(canonical_cmr_exterior_ring(values)?);
    }
    let mut paired = encoded
        .into_iter()
        .zip(transport_vertex_counts)
        .collect::<Vec<_>>();
    paired.dedup_by(|left, right| left.0 == right.0);
    let (polygons, transport_vertex_counts): (Vec<_>, Vec<_>) = paired.into_iter().unzip();

    Ok(CatalogueSpatialPlan {
        canonical_geometry: Some(geometry),
        transport_part_count: polygons.len(),
        polygons,
        mode,
        requires_exact_post_filter,
        canonical_polygon_count,
        canonical_ring_count,
        canonical_vertex_count,
        transport_vertex_counts,
    })
}

/// Applies the canonical AOI to a mapped provider footprint after a conservative
/// CMR transport plan. Both catalogue routes use this single longitude-aware
/// implementation so an approximate transport cannot admit a false positive.
pub(crate) fn footprint_intersects_exact_catalogue_aoi(
    footprint_geojson: &Map<String, Value>,
    aoi: &MultiPolygon<f64>,
) -> bool {
    shared_exact_catalogue_intersection(footprint_geojson, aoi)
}

fn native_catalogue_bbox(geometry: &MultiPolygon<f64>) -> Option<BBox> {
    let rect = geometry.bounding_rect()?;
    let bbox = BBox {
        west: rect.min().x,
        east: rect.max().x,
        south: rect.min().y,
        north: rect.max().y,
        crs: "EPSG:4326".to_owned(),
    };
    (bbox.east - bbox.west <= 180.0 && bbox.validate().is_ok()).then_some(bbox)
}

fn catalogue_authoritative_geometry(aoi: &Map<String, Value>) -> Option<MultiPolygon<f64>> {
    match aoi.get("type").and_then(Value::as_str)? {
        "Polygon" | "MultiPolygon" => {
            validated_authoritative_polygonal_geojson(aoi).map(|validated| validated.geometry)
        }
        "Feature" => aoi
            .get("geometry")
            .and_then(Value::as_object)
            .and_then(catalogue_authoritative_geometry),
        "FeatureCollection" => {
            let mut polygons = Vec::new();
            for feature in aoi.get("features")?.as_array()? {
                let feature = feature.as_object()?;
                polygons.extend(catalogue_authoritative_geometry(feature)?.0);
            }
            (!polygons.is_empty()).then_some(MultiPolygon(polygons))
        }
        _ => None,
    }
}

fn polygon_vertex_count(polygon: &Polygon<f64>) -> usize {
    polygon.exterior().0.len()
        + polygon
            .interiors()
            .iter()
            .map(|ring| ring.0.len())
            .sum::<usize>()
}

fn conservative_transport_tiles(windows: Vec<BBox>) -> ProviderResult<Vec<Polygon<f64>>> {
    let mut tiles = windows;
    if tiles.is_empty() || tiles.len() > CMR_MAX_TRANSPORT_PARTS {
        return Err(catalogue_query_error(
            "CATALOGUE_SPATIAL_PLAN_EXCEEDED",
            "aoi_geojson",
            "transport_part_limit",
        ));
    }
    while tiles.len() < CMR_MAX_TRANSPORT_PARTS {
        let Some((index, _)) = tiles
            .iter()
            .enumerate()
            .filter_map(|(index, tile)| {
                let longitude_ratio = (tile.east - tile.west) / CMR_TARGET_TILE_LONGITUDE_SPAN;
                let latitude_ratio = (tile.north - tile.south) / CMR_TARGET_TILE_LATITUDE_SPAN;
                let score = longitude_ratio.max(latitude_ratio);
                (score > 1.0).then_some((index, score))
            })
            .max_by(|left, right| left.1.total_cmp(&right.1))
        else {
            break;
        };
        let tile = tiles.remove(index);
        let width = tile.east - tile.west;
        let height = tile.north - tile.south;
        let (first, second) =
            if width / CMR_TARGET_TILE_LONGITUDE_SPAN >= height / CMR_TARGET_TILE_LATITUDE_SPAN {
                let middle = tile.west + width / 2.0;
                (
                    BBox {
                        east: middle,
                        ..tile.clone()
                    },
                    BBox {
                        west: middle,
                        ..tile
                    },
                )
            } else {
                let middle = tile.south + height / 2.0;
                (
                    BBox {
                        north: middle,
                        ..tile.clone()
                    },
                    BBox {
                        south: middle,
                        ..tile
                    },
                )
            };
        tiles.extend([first, second]);
    }
    tiles.sort_by(|left, right| {
        left.west
            .total_cmp(&right.west)
            .then_with(|| left.south.total_cmp(&right.south))
            .then_with(|| left.east.total_cmp(&right.east))
            .then_with(|| left.north.total_cmp(&right.north))
    });
    Ok(tiles
        .into_iter()
        .map(|tile| rectangle_polygon(tile.west, tile.south, tile.east, tile.north))
        .collect())
}

fn rectangle_polygon(west: f64, south: f64, east: f64, north: f64) -> Polygon<f64> {
    Polygon::new(
        LineString::from(vec![
            (west, south),
            (east, south),
            (east, north),
            (west, north),
            (west, south),
        ]),
        Vec::new(),
    )
}

#[cfg(test)]
mod catalogue_spatial_plan_tests {
    use super::catalogue_spatial_plan;
    use serde_json::{json, Map, Value};

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().expect("GeoJSON object").clone()
    }

    #[test]
    fn simple_safe_polygon_uses_exact_cmr_transport() {
        let aoi = object(json!({
            "type": "Polygon",
            "coordinates": [[[106.0, 29.0], [107.0, 29.0], [107.0, 30.0], [106.0, 30.0], [106.0, 29.0]]]
        }));

        let plan = catalogue_spatial_plan(&aoi).expect("simple exact plan");

        assert_eq!(plan.mode, "cmr_exact_polygon");
        assert!(!plan.requires_exact_post_filter);
        assert_eq!(plan.transport_part_count, 1);
        assert_eq!(plan.transport_vertex_counts, vec![5]);
        assert_eq!(plan.canonical_polygon_count, 1);
        assert_eq!(plan.canonical_ring_count, 1);
        assert_eq!(plan.canonical_vertex_count, 5);
        assert!(plan.canonical_geometry.is_some());
    }

    #[test]
    fn polygon_with_hole_uses_one_conservative_bbox_and_exact_post_filter() {
        let aoi = object(json!({
            "type": "Polygon",
            "coordinates": [
                [[100.0, 20.0], [110.0, 20.0], [110.0, 30.0], [100.0, 30.0], [100.0, 20.0]],
                [[103.0, 23.0], [107.0, 23.0], [107.0, 27.0], [103.0, 27.0], [103.0, 23.0]]
            ]
        }));

        let plan = catalogue_spatial_plan(&aoi).expect("hole plan");

        assert_eq!(plan.mode, "cmr_conservative_bbox_exact_post_filter");
        assert!(plan.requires_exact_post_filter);
        assert_eq!(plan.transport_part_count, 1);
        assert_eq!(plan.transport_vertex_counts, vec![5]);
        assert_eq!(plan.canonical_ring_count, 2);
    }

    #[test]
    fn multipolygon_uses_bounded_conservative_transport() {
        let aoi = object(json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[100.0, 20.0], [101.0, 20.0], [101.0, 21.0], [100.0, 21.0], [100.0, 20.0]]],
                [[[120.0, 40.0], [121.0, 40.0], [121.0, 41.0], [120.0, 41.0], [120.0, 40.0]]]
            ]
        }));

        let plan = catalogue_spatial_plan(&aoi).expect("multipolygon plan");

        assert!(plan.requires_exact_post_filter);
        assert!((1..=8).contains(&plan.transport_part_count));
        assert_eq!(plan.transport_part_count, plan.polygons.len());
        assert!(plan.transport_vertex_counts.iter().all(|count| *count == 5));
        assert_eq!(plan.canonical_polygon_count, 2);
    }

    #[test]
    fn high_detail_administrative_multipolygon_falls_back_to_native_conservative_bbox() {
        let coordinates = (0..6)
            .map(|component| {
                let center_x = 114.0 + f64::from(component % 3) * 1.5;
                let center_y = 37.0 + f64::from(component / 3) * 1.5;
                let mut ring = (0..280)
                    .map(|index| {
                        let angle = std::f64::consts::TAU * f64::from(index) / 280.0;
                        json!([center_x + 0.3 * angle.cos(), center_y + 0.3 * angle.sin()])
                    })
                    .collect::<Vec<_>>();
                ring.push(ring[0].clone());
                json!([ring])
            })
            .collect::<Vec<_>>();
        let aoi = object(json!({"type": "MultiPolygon", "coordinates": coordinates}));

        let plan = catalogue_spatial_plan(&aoi).expect("ordinary-domain detail remains queryable");

        assert_eq!(plan.mode, "cmr_conservative_bbox_exact_post_filter");
        assert!(plan.requires_exact_post_filter);
        assert_eq!(plan.transport_part_count, 1);
        assert_eq!(plan.transport_vertex_counts, vec![5]);
        assert_eq!(plan.canonical_polygon_count, 6);
        assert_eq!(plan.canonical_vertex_count, 1_686);
    }

    #[test]
    fn national_scale_high_vertex_polygon_is_tiled_without_fragment_explosion() {
        let mut ring = (0..160)
            .map(|index| {
                let angle = std::f64::consts::TAU * index as f64 / 160.0;
                json!([104.0 + 31.0 * angle.cos(), 36.0 + 18.0 * angle.sin()])
            })
            .collect::<Vec<_>>();
        ring.push(ring[0].clone());
        let aoi = object(json!({"type": "Polygon", "coordinates": [ring]}));

        let plan = catalogue_spatial_plan(&aoi).expect("national-scale plan");

        assert_eq!(plan.mode, "cmr_tiled_envelope_exact_post_filter");
        assert!(plan.requires_exact_post_filter);
        assert!((2..=8).contains(&plan.transport_part_count));
        assert_eq!(plan.transport_part_count, plan.polygons.len());
        assert!(plan.transport_vertex_counts.iter().all(|count| *count == 5));
        assert_eq!(plan.canonical_vertex_count, 161);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CmrPoint {
    longitude: f64,
    latitude: f64,
}

fn canonical_cmr_exterior_ring(values: Vec<(f64, f64)>) -> ProviderResult<String> {
    let mut points = values
        .into_iter()
        .map(|(longitude, latitude)| CmrPoint {
            longitude,
            latitude,
        })
        .collect::<Vec<_>>();
    if points.is_empty() {
        return Err(catalogue_query_error(
            "CATALOGUE_FILTER_INVALID",
            "aoi_geojson",
            "polygon_empty_ring",
        ));
    }

    points.dedup();
    while points.len() > 1 && points.last() == points.first() {
        points.pop();
    }
    if points.len() < 3 {
        return Err(catalogue_query_error(
            "CATALOGUE_FILTER_INVALID",
            "aoi_geojson",
            "polygon_too_few_vertices",
        ));
    }
    points.push(points[0]);

    let distinct = points[..points.len() - 1]
        .iter()
        .enumerate()
        .filter(|(index, point)| points[..*index].iter().all(|prior| prior != *point))
        .count();
    if distinct < 3 {
        return Err(catalogue_query_error(
            "CATALOGUE_FILTER_INVALID",
            "aoi_geojson",
            "polygon_too_few_vertices",
        ));
    }

    let signed_area = ring_signed_area(&points);
    if !signed_area.is_finite() || signed_area == 0.0 {
        return Err(catalogue_query_error(
            "CATALOGUE_FILTER_INVALID",
            "aoi_geojson",
            "polygon_zero_area",
        ));
    }
    if signed_area < 0.0 {
        let first = points[0];
        let mut interior = points[1..points.len() - 1].to_vec();
        interior.reverse();
        points.clear();
        points.push(first);
        points.extend(interior);
        points.push(first);
    }

    Ok(points
        .into_iter()
        .flat_map(|point| {
            [
                format_cmr_number(point.longitude),
                format_cmr_number(point.latitude),
            ]
        })
        .collect::<Vec<_>>()
        .join(","))
}

fn ring_signed_area(points: &[CmrPoint]) -> f64 {
    points
        .windows(2)
        .map(|pair| pair[0].longitude * pair[1].latitude - pair[1].longitude * pair[0].latitude)
        .sum::<f64>()
        / 2.0
}

fn format_cmr_number(value: f64) -> String {
    if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    }
}

pub(crate) fn catalogue_filter_error(field: &'static str) -> ContractError {
    catalogue_query_error("CATALOGUE_FILTER_INVALID", field, "invalid_value")
}

pub(crate) fn catalogue_query_error(
    code: &'static str,
    field: &'static str,
    reason: impl Into<String>,
) -> ContractError {
    ContractError::new(
        code,
        "ASF 目录检索条件无效",
        Map::from_iter([
            ("field".to_owned(), Value::String(field.to_owned())),
            ("reason".to_owned(), Value::String(reason.into())),
        ]),
    )
    .expect("bounded catalogue query error is safe")
}

fn sar_query_error(code: &str, message: &str, field: &str) -> ContractError {
    sar_query_error_with_context(code, message, field, &[])
}

fn sar_query_error_with_context(
    code: &str,
    message: &str,
    field: &str,
    extra: &[(&str, Value)],
) -> ContractError {
    let mut map = Map::from_iter([("field".to_string(), Value::String(field.to_string()))]);
    for (key, value) in extra {
        map.insert((*key).to_string(), value.clone());
    }
    ContractError::new(code, message, map).expect("bounded SAR query context is safe")
}

fn canonical_values(
    requested: &[String],
    allowed: &[String],
    field: &str,
) -> ProviderResult<Vec<String>> {
    requested
        .iter()
        .map(|value| {
            allowed
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(value.trim()))
                .cloned()
                .ok_or_else(|| {
                    sar_query_error(
                        "ASF_QUERY_UNSUPPORTED",
                        "ASF 检索条件不受当前数据集支持",
                        field,
                    )
                })
        })
        .collect()
}

fn dataset_capabilities(value: &str) -> ProviderResult<DatasetCapabilities> {
    asf_catalog_capabilities()
        .datasets
        .into_iter()
        .find(|item| item.dataset.eq_ignore_ascii_case(value.trim()))
        .ok_or_else(|| {
            if super::legacy_cmr::is_legacy_cmr_dataset(value) {
                return sar_query_error(
                    "ASF_DATASET_REMOVED",
                    "ASF 已从检索接口下线该数据集",
                    "dataset",
                );
            }
            sar_query_error(
                "ASF_QUERY_UNSUPPORTED",
                "ASF Provider 不支持该数据集",
                "dataset",
            )
        })
}

fn optional_canonical(
    requested: Option<&str>,
    allowed: &[String],
    field: &str,
) -> ProviderResult<Option<String>> {
    let Some(value) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    canonical_values(&[value.to_string()], allowed, field).map(|values| values.into_iter().next())
}

fn validate_filter_pattern(
    descriptor: &FilterDescriptor,
    value: &str,
    field: &str,
) -> ProviderResult<()> {
    match descriptor.pattern.as_deref() {
        None => Ok(()),
        Some("^[0-9]+$") if value.chars().all(|character| character.is_ascii_digit()) => Ok(()),
        Some("^[0-9]+$") => Err(sar_query_error(
            "ASF_QUERY_INVALID",
            "Filter value does not match the dataset capability pattern",
            field,
        )),
        Some(_) => Err(sar_query_error(
            "ASF_QUERY_INVALID",
            "Filter capability uses an unsupported validation pattern",
            field,
        )),
    }
}

fn effective_filter_bounds(
    descriptor: &FilterDescriptor,
    platforms: &[String],
    field: &str,
) -> ProviderResult<(Option<u64>, Option<u64>)> {
    let mut minimum = descriptor.minimum;
    let mut maximum = descriptor.maximum;
    for platform in platforms {
        let Some(limit) = descriptor
            .platform_limits
            .iter()
            .find(|limit| limit.platform.eq_ignore_ascii_case(platform))
        else {
            continue;
        };
        minimum = Some(minimum.map_or(limit.minimum, |value| value.max(limit.minimum)));
        maximum = Some(maximum.map_or(limit.maximum, |value| value.min(limit.maximum)));
    }
    if minimum.zip(maximum).is_some_and(|(low, high)| low > high) {
        return Err(sar_query_error(
            "ASF_QUERY_INVALID",
            "Dataset filter capability has contradictory bounds",
            field,
        ));
    }
    Ok((minimum, maximum))
}

fn validate_filter_value(
    descriptor: &FilterDescriptor,
    value: &str,
    platforms: &[String],
    field: &str,
) -> ProviderResult<u64> {
    validate_filter_pattern(descriptor, value, field)?;
    if let Some(width) = descriptor
        .exact_width
        .filter(|width| value.len() != *width as usize)
    {
        return Err(sar_query_error_with_context(
            "ASF_QUERY_INVALID",
            "Filter value does not use the required fixed width",
            field,
            &[
                ("reason", Value::String("exact_width".to_string())),
                ("exact_width", Value::from(width)),
            ],
        ));
    }
    let numeric = value.parse::<u64>().map_err(|_| {
        sar_query_error(
            "ASF_QUERY_INVALID",
            "Filter value is outside the supported integer range",
            field,
        )
    })?;
    let (minimum, maximum) = effective_filter_bounds(descriptor, platforms, field)?;
    if minimum.is_some_and(|minimum| numeric < minimum)
        || maximum.is_some_and(|maximum| numeric > maximum)
    {
        let mut extra = vec![("reason", Value::String("out_of_range".to_string()))];
        if let Some(low) = minimum {
            extra.push(("min", Value::from(low)));
        }
        if let Some(high) = maximum {
            extra.push(("max", Value::from(high)));
        }
        return Err(sar_query_error_with_context(
            "ASF_QUERY_INVALID",
            "Filter value is outside the dataset capability range",
            field,
            &extra,
        ));
    }
    Ok(numeric)
}

fn validate_filter_expression(
    descriptor: &FilterDescriptor,
    value: &str,
    platforms: &[String],
    field: &str,
) -> ProviderResult<()> {
    for part in value.split(',') {
        if let Some((start, end)) = part.split_once('-') {
            validate_filter_value(descriptor, start, platforms, field)?;
            validate_filter_value(descriptor, end, platforms, field)?;
        } else {
            validate_filter_value(descriptor, part, platforms, field)?;
        }
    }
    Ok(())
}

fn insert_capability_filter(
    params: &mut BTreeMap<String, String>,
    descriptor: Option<&FilterDescriptor>,
    value: Option<&str>,
    platforms: &[String],
    field: &str,
) -> ProviderResult<()> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let descriptor = descriptor.filter(|item| item.supported).ok_or_else(|| {
        sar_query_error("ASF_QUERY_UNSUPPORTED", "当前数据集不支持该筛选条件", field)
    })?;
    let parameter = descriptor
        .parameter
        .as_deref()
        .filter(|parameter| !parameter.is_empty())
        .ok_or_else(|| sar_query_error("ASF_QUERY_INVALID", "数据集筛选能力缺少参数名", field))?;
    let numeric = validate_filter_value(descriptor, value, platforms, field)?;
    let translated = match descriptor.value_type {
        FilterValueType::Integer => numeric.to_string(),
        FilterValueType::PaddedString => value.to_string(),
    };
    params.insert(parameter.to_string(), translated);
    Ok(())
}

fn range_endpoint<'a>(value: Option<&'a str>, field: &str) -> ProviderResult<&'a str> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            sar_query_error(
                "ASF_QUERY_INVALID",
                "Filter range requires complete start and end values",
                field,
            )
        })
}

fn translate_range_endpoint(
    descriptor: &FilterDescriptor,
    value: &str,
    platforms: &[String],
    field: &str,
) -> ProviderResult<String> {
    let numeric = validate_filter_value(descriptor, value, platforms, field)?;
    match descriptor.value_type {
        FilterValueType::Integer => Ok(numeric.to_string()),
        FilterValueType::PaddedString => Ok(value.to_string()),
    }
}

fn insert_capability_filter_range(
    params: &mut BTreeMap<String, String>,
    descriptor: Option<&FilterDescriptor>,
    range: &SarFilterRange,
    platforms: &[String],
    field: &str,
) -> ProviderResult<()> {
    let descriptor = descriptor.filter(|item| item.supported).ok_or_else(|| {
        sar_query_error(
            "ASF_QUERY_UNSUPPORTED",
            "The selected dataset does not support this filter range",
            field,
        )
    })?;
    let parameter = descriptor
        .parameter
        .as_deref()
        .filter(|parameter| !parameter.is_empty())
        .ok_or_else(|| {
            sar_query_error(
                "ASF_QUERY_INVALID",
                "Dataset filter capability is missing a parameter name",
                field,
            )
        })?;
    let start = translate_range_endpoint(
        descriptor,
        range_endpoint(range.start.as_deref(), field)?,
        platforms,
        field,
    )?;
    let end = translate_range_endpoint(
        descriptor,
        range_endpoint(range.end.as_deref(), field)?,
        platforms,
        field,
    )?;
    if numeric_string_greater(&start, &end) {
        return Err(sar_query_error(
            "ASF_QUERY_INVALID",
            "筛选范围起始值不能大于结束值",
            field,
        ));
    }
    params.insert(parameter.to_string(), format!("{start}-{end}"));
    Ok(())
}

fn numeric_string_greater(left: &str, right: &str) -> bool {
    let left = left.trim_start_matches('0');
    let right = right.trim_start_matches('0');
    let left = if left.is_empty() { "0" } else { left };
    let right = if right.is_empty() { "0" } else { right };
    left.len() > right.len() || (left.len() == right.len() && left > right)
}

fn validate_sentinel_grd_polarizations(
    dataset: &str,
    levels: &[String],
    polarizations: &[String],
) -> ProviderResult<()> {
    if !dataset.eq_ignore_ascii_case("SENTINEL-1") || polarizations.is_empty() {
        return Ok(());
    }
    let requires_dual = levels
        .iter()
        .any(|level| matches!(level.as_str(), "GRD_HD" | "GRD_MD" | "GRD_FD"));
    let requires_single = levels
        .iter()
        .any(|level| matches!(level.as_str(), "GRD_HS" | "GRD_MS"));
    if !requires_dual && !requires_single {
        return Ok(());
    }
    let has_dual = polarizations
        .iter()
        .any(|value| matches!(value.as_str(), "DV" | "DH") || value.contains('+'));
    let has_single = polarizations
        .iter()
        .any(|value| matches!(value.as_str(), "SV" | "SH" | "VV" | "HH"));
    let valid = match (requires_dual, requires_single) {
        (true, true) => has_dual && has_single,
        (true, false) => has_dual && !has_single,
        (false, true) => has_single && !has_dual,
        (false, false) => true,
    };
    if valid {
        Ok(())
    } else {
        Err(sar_query_error(
            "ASF_QUERY_INVALID",
            "Sentinel-1 精细 GRD 处理级别与极化组合不匹配",
            "polarizations",
        ))
    }
}

/// Translate the generic SAR query into the reviewed ASF SearchAPI vocabulary.
pub fn plan_sar_query(query: &SarSearchQuery) -> ProviderResult<AsfQueryPlan> {
    query
        .validate()
        .map_err(|reason| sar_query_error("ASF_QUERY_INVALID", "ASF 检索条件无效", &reason))?;
    if !query.provider.eq_ignore_ascii_case("ASF") {
        return Err(sar_query_error(
            "ASF_QUERY_UNSUPPORTED",
            "ASF Provider 不接受其他数据提供方查询",
            "provider",
        ));
    }
    let capabilities = dataset_capabilities(&query.dataset)?;
    if query.burst.is_some() && !capabilities.supports_burst {
        return Err(sar_query_error(
            "ASF_QUERY_UNSUPPORTED",
            "Burst filters are only supported by the SLC-BURST dataset",
            "burst",
        ));
    }
    if let Some(burst) = query.burst.as_ref() {
        if burst.burst_id.is_some() || burst.subswath.is_some() || burst.burst_index.is_some() {
            return Err(sar_query_error(
                "ASF_QUERY_UNSUPPORTED",
                "Only absoluteBurstID, relativeBurstID, and fullBurstID are remote filters",
                "burst",
            ));
        }
    }
    let platforms = canonical_values(&query.platforms, &capabilities.platforms, "platforms")?;
    let instrument = optional_canonical(
        query.instrument.as_deref(),
        &capabilities.instruments,
        "instrument",
    )?;
    let mut levels = canonical_values(
        &query.processing_levels,
        &capabilities.processing_levels,
        "processing_levels",
    )?;
    if capabilities.supports_burst && levels.is_empty() {
        levels.push("BURST".to_string());
    }
    canonical_values(
        &query.product_families,
        &capabilities.product_families,
        "product_families",
    )?;
    let beams = expand_sentinel_beam_modes(
        &capabilities.dataset,
        canonical_values(&query.beam_modes, &capabilities.beam_modes, "beam_modes")?,
    );
    let polarizations = canonical_values(
        &query.polarizations,
        &capabilities.polarizations,
        "polarizations",
    )?;
    validate_sentinel_grd_polarizations(&capabilities.dataset, &levels, &polarizations)?;

    if query.look_direction.is_some() && !capabilities.supports_look_direction {
        return Err(sar_query_error(
            "ASF_QUERY_UNSUPPORTED",
            "当前数据集不支持 Look Direction 筛选",
            "look_direction",
        ));
    }
    let look_direction = match query
        .look_direction
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) if value.eq_ignore_ascii_case("R") || value.eq_ignore_ascii_case("RIGHT") => {
            Some("RIGHT")
        }
        Some(value) if value.eq_ignore_ascii_case("L") || value.eq_ignore_ascii_case("LEFT") => {
            Some("LEFT")
        }
        Some(_) => {
            return Err(sar_query_error(
                "ASF_QUERY_UNSUPPORTED",
                "Look Direction 仅支持 LEFT 或 RIGHT",
                "look_direction",
            ));
        }
        None => None,
    };
    if query.track_range.is_none()
        && query.relative_orbit.is_some()
        && !capabilities.supports_relative_orbit
    {
        return Err(sar_query_error(
            "ASF_QUERY_UNSUPPORTED",
            "当前数据集不支持 Relative Orbit 筛选",
            "relative_orbit",
        ));
    }
    if query.frame_range.is_none()
        && (query.frame.is_some() || query.asf_frame.is_some())
        && !capabilities
            .frame_filter
            .as_ref()
            .is_some_and(|descriptor| descriptor.supported)
    {
        return Err(sar_query_error(
            "ASF_QUERY_UNSUPPORTED",
            "当前数据集不支持 Frame 筛选",
            "frame",
        ));
    }
    if query.track_range.is_none()
        && query.path.is_some()
        && !capabilities
            .track_filter
            .as_ref()
            .is_some_and(|descriptor| descriptor.supported)
    {
        return Err(sar_query_error(
            "ASF_QUERY_UNSUPPORTED",
            "当前数据集不支持 Path 筛选",
            "path",
        ));
    }
    if query.track_range.is_none()
        && query.relative_orbit.is_some()
        && query.path.is_some()
        && query.relative_orbit != query.path
    {
        return Err(sar_query_error(
            "ASF_QUERY_INVALID",
            "Path 与 Relative Orbit 不能同时指定不同值",
            "path",
        ));
    }
    if query.frame_range.is_none() {
        if let (Some(frame), Some(asf_frame)) = (query.frame, query.asf_frame.as_deref()) {
            if frame.to_string() != asf_frame.trim() {
                return Err(sar_query_error(
                    "ASF_QUERY_INVALID",
                    "Frame 与 ASF Frame 不能同时设置为不同值",
                    "frame",
                ));
            }
        }
    }

    let requested_was_unbounded = query.max_results.is_none();
    let requested_limit = query
        .max_results
        .unwrap_or(MAX_CATALOG_RESULTS)
        .clamp(1, insar_core::SEARCH_SAFETY_CAP);
    let raw_aoi_geometry = match query.aoi_geojson.as_ref() {
        Some(raw) => Some(
            validated_authoritative_polygonal_geojson(raw)
                .ok_or_else(|| sar_query_error("ASF_QUERY_INVALID", "AOI 几何无效", "aoi_geojson"))?
                .geometry,
        ),
        None => None,
    };
    let bbox = query
        .aoi_geojson
        .is_none()
        .then_some(query.bbox.as_ref())
        .flatten();
    let has_aoi = query.aoi_wkt.is_some() || query.aoi_geojson.is_some() || bbox.is_some();
    let query_limit = search_query_limit(requested_limit, has_aoi);
    let filter_platforms = if platforms.is_empty() {
        capabilities.platforms.clone()
    } else {
        platforms.clone()
    };
    let mut params = BTreeMap::from([
        ("dataset".to_string(), capabilities.dataset.clone()),
        ("output".to_string(), "geojson".to_string()),
        ("maxResults".to_string(), query_limit.to_string()),
    ]);
    for (name, values) in [
        ("platform", platforms),
        ("processingLevel", levels),
        ("beamMode", beams),
        ("polarization", polarizations),
    ] {
        if !values.is_empty() {
            params.insert(name.to_string(), values.join(","));
        }
    }
    if let Some(value) = instrument {
        params.insert("instrument".to_string(), value);
    }
    if let Some(value) = query.flight_direction.as_deref() {
        let value = value.trim().to_ascii_uppercase();
        if !matches!(value.as_str(), "ASCENDING" | "DESCENDING")
            || !capabilities.supports_flight_direction
        {
            return Err(sar_query_error(
                "ASF_QUERY_UNSUPPORTED",
                "当前数据集不支持该轨道方向筛选",
                "flight_direction",
            ));
        }
        params.insert("flightDirection".to_string(), value);
    }
    if let Some(value) = look_direction {
        params.insert("lookDirection".to_string(), value.to_string());
    }
    if let Some(burst) = query.burst.as_ref() {
        if let Some(value) = burst.full_burst_id.as_deref() {
            let value = value.trim().to_ascii_uppercase();
            let parts = value.split('_').collect::<Vec<_>>();
            let valid_swath = parts.get(2).is_some_and(|swath| {
                matches!(
                    *swath,
                    "IW1" | "IW2" | "IW3" | "EW1" | "EW2" | "EW3" | "EW4" | "EW5"
                )
            });
            if parts.len() != 3
                || parts[0].len() != 3
                || parts[1].len() != 6
                || !parts[0].chars().all(|character| character.is_ascii_digit())
                || !parts[1].chars().all(|character| character.is_ascii_digit())
                || !valid_swath
            {
                return Err(sar_query_error(
                    "ASF_QUERY_INVALID",
                    "fullBurstID must use the official NNN_NNNNNN_IWn/EWn format",
                    "burst.full_burst_id",
                ));
            }
            params.insert("fullBurstID".to_string(), value);
        }
        for (parameter, value, field) in [
            (
                "relativeBurstID",
                burst.relative_burst_id,
                "burst.relative_burst_id",
            ),
            (
                "absoluteBurstID",
                burst.absolute_burst_id,
                "burst.absolute_burst_id",
            ),
        ] {
            if let Some(value) = value {
                if value <= 0 {
                    return Err(sar_query_error(
                        "ASF_QUERY_INVALID",
                        "Burst identifiers must be positive integers",
                        field,
                    ));
                }
                params.insert(parameter.to_string(), value.to_string());
            }
        }
    }
    if let Some(range) = query.track_range.as_ref() {
        insert_capability_filter_range(
            &mut params,
            capabilities.track_filter.as_ref(),
            range,
            &filter_platforms,
            "track_range",
        )?;
    } else {
        let track_value = query
            .relative_orbit
            .or(query.path)
            .map(|value| value.to_string());
        insert_capability_filter(
            &mut params,
            capabilities.track_filter.as_ref(),
            track_value.as_deref(),
            &filter_platforms,
            "relative_orbit",
        )?;
    }
    if let Some(range) = query.frame_range.as_ref() {
        insert_capability_filter_range(
            &mut params,
            capabilities.frame_filter.as_ref(),
            range,
            &filter_platforms,
            "frame_range",
        )?;
    } else {
        let frame_value = query.frame.map(|value| value.to_string());
        let requested_frame = query.asf_frame.as_deref().or(frame_value.as_deref());
        insert_capability_filter(
            &mut params,
            capabilities.frame_filter.as_ref(),
            requested_frame,
            &filter_platforms,
            "frame",
        )?;
    }
    if let Some(value) = query.start.as_deref() {
        if let Some(value) = asf_datetime(value, false)
            .map_err(|_| sar_query_error("ASF_QUERY_INVALID", "ASF 日期筛选格式无效", "start"))?
        {
            params.insert("start".to_string(), value);
        }
    }
    if let Some(value) = query.end.as_deref() {
        if let Some(value) = asf_datetime(value, true)
            .map_err(|_| sar_query_error("ASF_QUERY_INVALID", "ASF 日期筛选格式无效", "end"))?
        {
            params.insert("end".to_string(), value);
        }
    }
    let (planned_wkt, aoi_mode) = if query.aoi_geojson.is_some() {
        let authoritative_bbox = raw_aoi_geometry.as_ref().and_then(bbox_from_geometry);
        plan_aoi_wkt(raw_aoi_geometry.as_ref(), authoritative_bbox.as_ref())
    } else if let Some(value) = query
        .aoi_wkt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        (Some(value.to_string()), AoiPlanMode::Exact)
    } else {
        plan_aoi_wkt(None, bbox)
    };
    if let Some(value) = planned_wkt {
        params.insert("intersectsWith".to_string(), value);
    }
    Ok(AsfQueryPlan {
        params,
        requested_limit,
        query_limit,
        // Every finite search freezes a bounded working set first. Archive-wide
        // counts are independent reporting work and must not hold the foreground
        // metadata path open for any mission.
        should_count: requested_was_unbounded,
        aoi_mode,
        requested_was_unbounded,
    })
}

pub fn plan_asf_query(query: &AsfSearchQuery) -> ProviderResult<AsfQueryPlan> {
    query
        .validate()
        .map_err(|reason| invalid_query("ASF 检索条件无效", reason))?;
    if !query.dataset.trim().eq_ignore_ascii_case("SENTINEL-1") {
        return Err(invalid_query(
            "ASF Provider 只接受 Sentinel-1 检索",
            "unsupported_dataset",
        ));
    }
    let capabilities = dataset_capabilities("SENTINEL-1")?;
    let level = query.product_type.trim().to_ascii_uppercase();
    if !matches!(
        level.as_str(),
        "SLC" | "GRD" | "GRD_HD" | "GRD_HS" | "GRD_MD" | "GRD_MS" | "GRD_FD" | "RAW" | "OCN"
    ) {
        return Err(invalid_query("不支持的 Sentinel-1 产品类型", level));
    }

    let requested_was_unbounded = query.max_results.is_none();
    let requested_limit = query
        .max_results
        .unwrap_or(MAX_CATALOG_RESULTS)
        .clamp(1, insar_core::SEARCH_SAFETY_CAP);
    let raw_aoi_geometry = match query.aoi_geojson.as_ref() {
        Some(raw) => Some(
            validated_authoritative_polygonal_geojson(raw)
                .ok_or_else(|| invalid_query("ASF AOI geometry is invalid", "aoi_geojson"))?
                .geometry,
        ),
        None => None,
    };
    let bbox = query
        .aoi_geojson
        .is_none()
        .then_some(query.bbox.as_ref())
        .flatten();
    let has_aoi = query.aoi_geojson.is_some() || bbox.is_some();
    let query_limit = search_query_limit(requested_limit, has_aoi);

    let mut params = BTreeMap::from([
        ("dataset".to_string(), "SENTINEL-1".to_string()),
        ("processingLevel".to_string(), level.clone()),
        ("output".to_string(), "geojson".to_string()),
        ("maxResults".to_string(), query_limit.to_string()),
    ]);

    let requested_beams = split_filter_values(&query.beam_mode);
    let includes_sm = requested_beams
        .iter()
        .any(|beam| beam.eq_ignore_ascii_case("SM"));
    let beams = expand_sentinel_beam_modes("SENTINEL-1", requested_beams);
    if beams.len() == 1 || includes_sm {
        params.insert("beamMode".to_string(), beams.join(","));
    }
    let polarizations = split_filter_values(&query.polarization);
    validate_sentinel_grd_polarizations(
        "SENTINEL-1",
        std::slice::from_ref(&level),
        &polarizations,
    )?;
    if polarizations.len() == 1 {
        params.insert(
            "polarization".to_string(),
            polarization_param(&polarizations[0]),
        );
    }
    if let Some(start) = asf_datetime(&query.start, false)? {
        params.insert("start".to_string(), start);
    }
    if let Some(end) = asf_datetime(&query.end, true)? {
        params.insert("end".to_string(), end);
    }

    let authoritative_bbox = raw_aoi_geometry.as_ref().and_then(bbox_from_geometry);
    let (aoi_wkt, aoi_mode) = plan_aoi_wkt(
        raw_aoi_geometry.as_ref(),
        authoritative_bbox.as_ref().or(bbox),
    );
    if let Some(wkt) = aoi_wkt {
        params.insert("intersectsWith".to_string(), wkt);
    }

    let directions = split_filter_values(&query.orbit_direction)
        .into_iter()
        .filter(|value| matches!(value.as_str(), "ASCENDING" | "DESCENDING"))
        .collect::<Vec<_>>();
    if directions.len() == 1 {
        params.insert("flightDirection".to_string(), directions[0].clone());
    }
    if let Some(value) = bounded_int_filter(
        query.relative_orbit.as_deref(),
        "Path",
        capabilities.track_filter.as_ref(),
        &capabilities.platforms,
    )? {
        params.insert("relativeOrbit".to_string(), value);
    }
    if let Some(value) = bounded_int_filter(
        query.frame.as_deref(),
        "Frame",
        capabilities.frame_filter.as_ref(),
        &capabilities.platforms,
    )? {
        params.insert("frame".to_string(), value);
    }

    Ok(AsfQueryPlan {
        params,
        requested_limit,
        query_limit,
        should_count: requested_was_unbounded || requested_limit > 1,
        aoi_mode,
        requested_was_unbounded,
    })
}

pub fn apply_remote_count(plan: &mut AsfQueryPlan, total_count: Option<u64>) {
    if plan.requested_was_unbounded {
        plan.requested_limit = total_count
            .map(|count| count.clamp(1, MAX_CATALOG_RESULTS))
            .unwrap_or(MAX_CATALOG_RESULTS);
        // Keep the remote count as reporting truth while bounding the actual
        // import target. A blank quantity must not turn a broad historical
        // query into a single request for tens of thousands of products.
        plan.query_limit = plan.requested_limit;
    } else if let Some(count) = total_count.filter(|count| *count > 0) {
        plan.query_limit = plan.query_limit.min(count.max(plan.requested_limit));
    }
    plan.params
        .insert("maxResults".to_string(), plan.query_limit.to_string());
}

pub fn next_refill_limit(current: u64, total_count: Option<u64>) -> Option<u64> {
    if current >= MAX_CATALOG_RESULTS || total_count.is_some_and(|total| current >= total) {
        return None;
    }
    let mut next = current
        .saturating_mul(2)
        .max(current.saturating_add(100))
        .min(MAX_CATALOG_RESULTS);
    if let Some(total) = total_count {
        next = next.min(total.max(current));
    }
    (next > current).then_some(next)
}

pub fn split_query_windows(
    start: Option<&str>,
    end: Option<&str>,
    now: &str,
) -> ProviderResult<Vec<QueryWindow>> {
    let start = parse_query_seconds(start.unwrap_or(""), false)?
        .unwrap_or(parse_rfc3339_seconds(ARCHIVE_START)?);
    let end = parse_query_seconds(end.unwrap_or(""), true)?.unwrap_or(parse_rfc3339_seconds(now)?);
    if start > end {
        return Err(invalid_query(
            "ASF 检索开始日期不能晚于结束日期",
            "start_after_end",
        ));
    }
    let mut windows = Vec::new();
    let mut cursor_end = end;
    let step = SPLIT_WINDOW_DAYS * 86_400;
    while cursor_end >= start {
        let cursor_start = start.max(cursor_end - step + 1);
        windows.push(QueryWindow {
            start: format_utc_seconds(cursor_start),
            end: format_utc_seconds(cursor_end),
        });
        cursor_end = cursor_start - 1;
    }
    Ok(windows)
}

pub fn split_query_windows_for_dataset(
    dataset: &str,
    start: Option<&str>,
    end: Option<&str>,
    now: &str,
) -> ProviderResult<Vec<QueryWindow>> {
    let archive = asf_catalog_archive_window(dataset).ok_or_else(|| {
        sar_query_error(
            "ASF_QUERY_UNSUPPORTED",
            "ASF Provider 不支持该数据集",
            "dataset",
        )
    })?;
    let default_end = match archive.end.as_deref() {
        Some(archive_end) => {
            let archive_seconds = parse_rfc3339_seconds(archive_end)?;
            let now_seconds = parse_rfc3339_seconds(now)?;
            format_utc_seconds(archive_seconds.min(now_seconds))
        }
        None => now.to_string(),
    };
    let effective_start = start
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(archive.start.as_str());
    let effective_end = end
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&default_end);
    split_query_windows(Some(effective_start), Some(effective_end), now)
}

pub fn bisect_window(window: &QueryWindow) -> ProviderResult<Vec<QueryWindow>> {
    let start = parse_rfc3339_seconds(&window.start)?;
    let end = parse_rfc3339_seconds(&window.end)?;
    if end - start <= 86_400 {
        return Ok(Vec::new());
    }
    let midpoint = start + (end - start) / 2;
    Ok(vec![
        QueryWindow {
            start: format_utc_seconds(midpoint + 1),
            end: format_utc_seconds(end),
        },
        QueryWindow {
            start: format_utc_seconds(start),
            end: format_utc_seconds(midpoint),
        },
    ])
}

fn invalid_query(message: &str, reason: impl Into<String>) -> ContractError {
    ContractError::new(
        "ASF002",
        message,
        Map::from_iter([("reason".to_string(), Value::String(reason.into()))]),
    )
    .expect("bounded ASF query context is safe")
}

fn search_query_limit(requested: u64, has_aoi: bool) -> u64 {
    if !has_aoi {
        return requested;
    }
    if requested == 1 {
        return 51;
    }
    if requested <= 200 {
        requested
            .saturating_mul(4)
            .max(requested.saturating_add(50))
    } else if requested <= 1_000 {
        requested
            .saturating_mul(3)
            .max(requested.saturating_add(250))
    } else {
        requested.saturating_add(1_000)
    }
}

fn split_filter_values(value: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    value
        .replace([';', '|'], ",")
        .split(',')
        .filter_map(|item| {
            let item = item.trim().to_ascii_uppercase();
            (!item.is_empty() && seen.insert(item.clone())).then_some(item)
        })
        .collect()
}

fn expand_sentinel_beam_modes(dataset: &str, beams: Vec<String>) -> Vec<String> {
    if !dataset.eq_ignore_ascii_case("SENTINEL-1") {
        return beams;
    }
    let mut seen = HashSet::new();
    let mut expanded = Vec::new();
    for beam in beams {
        if beam.eq_ignore_ascii_case("SM") {
            for submode in ["S1", "S2", "S3", "S4", "S5", "S6"] {
                if seen.insert(submode.to_string()) {
                    expanded.push(submode.to_string());
                }
            }
        } else if seen.insert(beam.clone()) {
            expanded.push(beam);
        }
    }
    expanded
}

fn polarization_param(value: &str) -> String {
    let normalized = value.trim().to_ascii_uppercase().replace(['-', '+'], "_");
    match normalized.as_str() {
        "DV" | "VV_VH" => "VV+VH".to_string(),
        "DH" | "HH_HV" => "HH+HV".to_string(),
        _ => normalized,
    }
}

fn int_filter(value: Option<&str>, name: &str) -> ProviderResult<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let compact = value.replace(' ', "");
    let valid = compact.split(',').all(|part| {
        let mut bounds = part.split('-');
        matches!(bounds.next(), Some(value) if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
            && bounds
                .next()
                .is_none_or(|value| !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
            && bounds.next().is_none()
    });
    if !valid {
        return Err(invalid_query(
            &format!("ASF {name} 参数必须是整数、整数区间或逗号列表"),
            "invalid_integer_filter",
        ));
    }
    for part in compact.split(',') {
        if let Some((start, end)) = part.split_once('-') {
            if numeric_string_greater(start, end) {
                return Err(invalid_query(
                    &format!("ASF {name} 参数范围起始值不能大于结束值"),
                    "range_start_after_end",
                ));
            }
        }
    }
    Ok(Some(compact))
}

fn bounded_int_filter(
    value: Option<&str>,
    name: &str,
    descriptor: Option<&FilterDescriptor>,
    platforms: &[String],
) -> ProviderResult<Option<String>> {
    let Some(compact) = int_filter(value, name)? else {
        return Ok(None);
    };
    let descriptor = descriptor
        .filter(|descriptor| descriptor.supported)
        .ok_or_else(|| {
            invalid_query(
                &format!("ASF {name} filter is not supported by this dataset"),
                "unsupported_filter",
            )
        })?;
    validate_filter_expression(descriptor, &compact, platforms, name).map_err(|_| {
        invalid_query(
            &format!("ASF {name} filter is outside the supported dataset range"),
            "filter_out_of_range",
        )
    })?;
    Ok(Some(compact))
}

pub(crate) fn asf_datetime(value: &str, end: bool) -> ProviderResult<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.contains('T') {
        return Ok(Some(value.to_string()));
    }
    let clean = value.replace('/', "-");
    let (year, month, day) = parse_date(&clean)?;
    let suffix = if end { "23:59:59Z" } else { "00:00:00Z" };
    Ok(Some(format!("{year:04}-{month:02}-{day:02}T{suffix}")))
}

pub(crate) fn effective_bbox_windows(query: &AsfSearchQuery) -> Option<Vec<BBox>> {
    match query.aoi_geojson.as_ref() {
        Some(raw_aoi) => {
            let geometry = validated_authoritative_polygonal_geojson(raw_aoi)?.geometry;
            Some(wrapped_aoi_envelope(&geometry)?.request_windows)
        }
        None => query.bbox.clone().map(|bbox| vec![bbox]),
    }
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

fn plan_aoi_wkt(
    geometry: Option<&MultiPolygon<f64>>,
    bbox: Option<&BBox>,
) -> (Option<String>, AoiPlanMode) {
    if let Some(geometry) = geometry {
        let wkt = multipolygon_wkt(geometry);
        if !wkt.is_empty() {
            return (Some(wkt), AoiPlanMode::Exact);
        }
        return (None, AoiPlanMode::None);
    }
    if let Some(bbox) = bbox {
        return (Some(bbox_wkt(bbox)), AoiPlanMode::Bbox);
    }
    (None, AoiPlanMode::None)
}

fn multipolygon_wkt(geometry: &MultiPolygon<f64>) -> String {
    if geometry.0.len() == 1 {
        return format!("POLYGON ({})", polygon_rings_wkt(&geometry.0[0]));
    }
    let polygons = geometry
        .0
        .iter()
        .map(|polygon| format!("({})", polygon_rings_wkt(polygon)))
        .collect::<Vec<_>>()
        .join(", ");
    if polygons.is_empty() {
        String::new()
    } else {
        format!("MULTIPOLYGON ({polygons})")
    }
}

fn polygon_rings_wkt(polygon: &Polygon<f64>) -> String {
    std::iter::once(polygon.exterior())
        .chain(polygon.interiors())
        .map(|ring| {
            let coordinates = ring
                .points()
                .map(|point| format!("{} {}", compact_float(point.x()), compact_float(point.y())))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({coordinates})")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn compact_float(value: f64) -> String {
    if value == 0.0 {
        "0".to_string()
    } else {
        value.to_string()
    }
}

fn bbox_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

fn bbox_wkt(bbox: &BBox) -> String {
    let west = bbox_float(bbox.west);
    let east = bbox_float(bbox.east);
    let south = bbox_float(bbox.south);
    let north = bbox_float(bbox.north);
    format!("POLYGON(({west} {south},{east} {south},{east} {north},{west} {north},{west} {south}))")
}

fn parse_query_seconds(value: &str, end: bool) -> ProviderResult<Option<i64>> {
    let Some(value) = asf_datetime(value, end)? else {
        return Ok(None);
    };
    parse_rfc3339_seconds(&value).map(Some)
}

fn parse_rfc3339_seconds(value: &str) -> ProviderResult<i64> {
    let value = value.trim();
    if value.len() < 19 || value.as_bytes().get(10) != Some(&b'T') {
        return Err(invalid_query("ASF 日期格式无效", "invalid_datetime"));
    }
    let (year, month, day) = parse_date(&value[..10])?;
    let hour = parse_two(&value[11..13])?;
    let minute = parse_two(&value[14..16])?;
    let second = parse_two(&value[17..19])?;
    if hour > 23 || minute > 59 || second > 59 {
        return Err(invalid_query("ASF 日期格式无效", "invalid_time"));
    }
    let suffix = &value[19..];
    let offset = if suffix == "Z" || suffix == "+00:00" || suffix.is_empty() {
        0
    } else if suffix.len() == 6 && matches!(suffix.as_bytes()[0], b'+' | b'-') {
        let hours = parse_two(&suffix[1..3])? as i64;
        let minutes = parse_two(&suffix[4..6])? as i64;
        if hours > 23 || minutes > 59 || suffix.as_bytes()[3] != b':' {
            return Err(invalid_query("ASF 日期格式无效", "invalid_offset"));
        }
        let seconds = hours * 3_600 + minutes * 60;
        if suffix.starts_with('+') {
            seconds
        } else {
            -seconds
        }
    } else {
        return Err(invalid_query("ASF 日期格式无效", "invalid_offset"));
    };
    Ok(days_from_civil(year, month, day) * 86_400
        + hour as i64 * 3_600
        + minute as i64 * 60
        + second as i64
        - offset)
}

fn parse_date(value: &str) -> ProviderResult<(i32, u32, u32)> {
    if value.len() != 10 || &value[4..5] != "-" || &value[7..8] != "-" {
        return Err(invalid_query("ASF 日期格式应为 YYYY-MM-DD", "invalid_date"));
    }
    let year = value[..4]
        .parse::<i32>()
        .map_err(|_| invalid_query("ASF 日期格式应为 YYYY-MM-DD", "invalid_year"))?;
    let month = parse_two(&value[5..7])? as u32;
    let day = parse_two(&value[8..10])? as u32;
    let max_day = days_in_month(year, month);
    if max_day == 0 || day == 0 || day > max_day {
        return Err(invalid_query("ASF 日期格式应为 YYYY-MM-DD", "invalid_date"));
    }
    Ok((year, month, day))
}

fn parse_two(value: &str) -> ProviderResult<u8> {
    value
        .parse::<u8>()
        .map_err(|_| invalid_query("ASF 日期格式无效", "invalid_datetime"))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let adjusted_month = month as i32 + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day as i32 - 1;
    let day_of_era = yoe * 365 + yoe / 4 - yoe / 100 + day_of_year;
    (era * 146_097 + day_of_era - 719_468) as i64
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era as i32 + era as i32 * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_piece + 2) / 5 + 1;
    let month = month_piece + if month_piece < 10 { 3 } else { -9 };
    year += i32::from(month <= 2);
    (year, month as u32, day as u32)
}

pub(crate) fn format_utc_seconds(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}
