//! Authenticated CMR search for the restricted legacy SAR missions
//! (ERS-1/ERS-2, RADARSAT-1, JERS-1).
//!
//! These missions are published by ASF as restricted CMR collections: their
//! granules are invisible to unauthenticated queries on both the ASF
//! SearchAPI and CMR (verified live 2026-10-31; SEASAT — free and open —
//! returns its archive anonymously on the same endpoints, while all three
//! legacy missions return zero). ASF's own tooling searches them through CMR
//! `shortName` queries with an Earthdata bearer token, and this module mirrors
//! that mechanism: project the legacy dataset selector to its CMR collection
//! short names, query `granules.json` with pagination, and map each entry to
//! the legacy identity model so `map_sar_feature` keeps owning identity,
//! footprint and asset classification.
//!
//! Evidence and probe log: `the ASF CMR capability contract

use insar_core::{AsfRawFeature, BBox, SarSearchQuery};
use serde_json::{Map, Value};

use super::query::asf_datetime;

/// CMR collection short names per legacy dataset selector, mirroring
/// `asf_search.CMR.datasets.dataset_collections` (upstream asf-search 12.x).
/// The Polar Year RADARSAT-1 collections are intentionally excluded: they are
/// separate Antarctic/Greenland mosaics, not the standard L0/L1 archive the
/// Registry resource advertises.
const LEGACY_CMR_COLLECTIONS: &[(&str, &[&str])] = &[
    ("ERS", &["ERS-1_L0", "ERS-1_L1", "ERS-2_L0", "ERS-2_L1"]),
    ("JERS-1", &["JERS-1_L0", "JERS-1_L1"]),
    ("RADARSAT-1", &["RSAT-1_L0", "RSAT-1_L1"]),
];

pub const LEGACY_CMR_PAGE_SIZE: u64 = 2000;
pub const MAX_LEGACY_CMR_PAGES: u64 = 100;

/// The CMR provider that owns the legacy collections (ASF DAAC).
const CMR_PROVIDER: &str = "ASF";

pub fn is_legacy_cmr_dataset(dataset: &str) -> bool {
    legacy_cmr_collections(dataset).is_some()
}

fn legacy_cmr_collections(dataset: &str) -> Option<&'static [&'static str]> {
    let trimmed = dataset.trim();
    LEGACY_CMR_COLLECTIONS
        .iter()
        .find(|(selector, _)| selector.eq_ignore_ascii_case(trimmed))
        .map(|(_, collections)| *collections)
}

/// One page request against CMR `granules.json` for a legacy mission.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LegacyCmrPage {
    pub params: Vec<(String, String)>,
    pub page_number: u64,
}

/// Project a legacy SAR query to its CMR page parameters. `bbox` comes from
/// the reviewed AOI projection (the same window logic the `/param` path uses),
/// so the spatial authority stays with the existing query planner.
pub(crate) fn legacy_cmr_page(
    query: &SarSearchQuery,
    bbox: Option<&BBox>,
    page_number: u64,
) -> Option<LegacyCmrPage> {
    let collections = legacy_cmr_collections(&query.dataset)?;
    if page_number == 0 || page_number > MAX_LEGACY_CMR_PAGES {
        return None;
    }
    let mut params = vec![
        ("provider".to_string(), CMR_PROVIDER.to_string()),
        ("page_size".to_string(), LEGACY_CMR_PAGE_SIZE.to_string()),
        ("page_num".to_string(), page_number.to_string()),
        ("sort_key[]".to_string(), "-start_date".to_string()),
        ("sort_key[]".to_string(), "granule_ur".to_string()),
    ];
    params.extend(
        collections
            .iter()
            .map(|collection| ("shortName".to_string(), (*collection).to_string())),
    );
    let start = asf_datetime(query.start.as_deref().unwrap_or(""), false)
        .ok()
        .flatten();
    let end = asf_datetime(query.end.as_deref().unwrap_or(""), true)
        .ok()
        .flatten();
    if start.is_some() || end.is_some() {
        params.push((
            "temporal".to_string(),
            format!(
                "{},{}",
                start.as_deref().unwrap_or(""),
                end.as_deref().unwrap_or("")
            ),
        ));
    }
    if let Some(bbox) = bbox {
        if !bbox_is_global(bbox) {
            params.push((
                "bounding_box".to_string(),
                format!("{},{},{},{}", bbox.west, bbox.south, bbox.east, bbox.north),
            ));
        }
    }
    Some(LegacyCmrPage {
        params,
        page_number,
    })
}

/// A whole-earth bounding box would match every granule; skipping it keeps the
/// query unfiltered, matching the unbounded archive-window searches the
/// legacy sources already perform.
fn bbox_is_global(bbox: &BBox) -> bool {
    bbox.west <= -180.0 && bbox.east >= 180.0 && bbox.south <= -90.0 && bbox.north >= 90.0
}

/// Count-only page: identical projection with a minimal page size. The count
/// itself comes from the `CMR-Hits` response header of this request, the
/// same signal CMR exposes for the open collections.
pub(crate) fn legacy_cmr_count_page(
    query: &SarSearchQuery,
    bbox: Option<&BBox>,
) -> Option<Vec<(String, String)>> {
    let page = legacy_cmr_page(query, bbox, 1)?;
    let mut params = page.params;
    params.retain(|(name, _)| name != "page_size" && name != "page_num");
    params.push(("page_size".to_string(), "1".to_string()));
    Some(params)
}

/// Map one CMR `feed.entry` granule record (ECHO granule JSON) to the legacy
/// `AsfRawFeature` shape. Identity stays with the legacy collection profiles:
/// `sceneName`/`granuleName` remain display-approved names and the mapper's
/// `productID`/`granuleUR` preference decides identity.
pub(crate) fn legacy_cmr_entry_to_feature(entry: &Map<String, Value>) -> Option<AsfRawFeature> {
    let mut properties = Map::new();
    let insert_text = |properties: &mut Map<String, Value>, key: &str, value: Option<String>| {
        if let Some(value) = value {
            properties.insert(key.to_string(), Value::String(value));
        }
    };
    let title = text(entry.get("title"))?;
    let producer_granule_id = text(entry.get("producer_granule_id"));
    let granule_ur = text(entry.get("producer_granule_id")).or_else(|| text(entry.get("id")));
    let display_name = producer_granule_id.as_deref().unwrap_or(title.as_str());
    insert_text(&mut properties, "sceneName", Some(display_name.to_string()));
    insert_text(&mut properties, "granuleName", granule_ur.clone());
    insert_text(&mut properties, "productID", granule_ur.clone());
    if let Some(size_mb) = entry.get("granule_size").and_then(Value::as_f64) {
        if size_mb.is_finite() && size_mb >= 0.0 {
            let bytes = (size_mb * 1024.0 * 1024.0).round() as u64;
            properties.insert(
                "bytes".to_string(),
                Value::Number(serde_json::Number::from(bytes)),
            );
        }
    }
    if let Some(link) = data_link(entry) {
        insert_text(&mut properties, "downloadUrl", Some(link));
    }
    let attributes = legacy_attribute_map(entry);
    let attribute_text = |names: &[&str]| -> Option<String> {
        names
            .iter()
            .find_map(|name| attributes.get(*name))
            .and_then(|value| match value {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
    };
    // The dataset selector owns the mission; the exact platform is only
    // informative for these single-instrument missions.
    insert_text(
        &mut properties,
        "platform",
        attribute_text(&["asfplatform", "platform"]),
    );
    insert_text(
        &mut properties,
        "polarization",
        attribute_text(&["polarization"]),
    );
    insert_text(
        &mut properties,
        "flightDirection",
        attribute_text(&["ascendingdescending", "flightdirection", "orbitdirection"]),
    );
    insert_text(
        &mut properties,
        "beamModeType",
        attribute_text(&["beammodetype"]),
    );
    insert_text(&mut properties, "beamMode", attribute_text(&["beammode"]));
    insert_text(
        &mut properties,
        "processingLevel",
        attribute_text(&["processinglevel", "processingtype", "level"]),
    );
    if let Some(path) = attributes.get("pathnumber").and_then(numeric_u64) {
        properties.insert(
            "pathNumber".to_string(),
            Value::Number(serde_json::Number::from(path)),
        );
    }
    if let Some(frame) = attributes.get("framenumber").and_then(numeric_u64) {
        properties.insert(
            "frameNumber".to_string(),
            Value::Number(serde_json::Number::from(frame)),
        );
    }
    if let Some(time) = text(entry.get("time_start")) {
        insert_text(&mut properties, "startTime", Some(time));
    }
    if let Some(time) = text(entry.get("time_end")) {
        insert_text(&mut properties, "stopTime", Some(time));
    }
    Some(AsfRawFeature {
        r#type: "Feature".to_string(),
        properties,
        geometry: legacy_polygon_geometry(entry),
    })
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn numeric_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn data_link(entry: &Map<String, Value>) -> Option<String> {
    entry
        .get("links")?
        .as_array()?
        .iter()
        .filter_map(Value::as_object)
        .find_map(|link| {
            let href = link.get("href")?.as_str()?.trim();
            if !href.starts_with("https://") {
                return None;
            }
            let relation = link.get("rel").and_then(Value::as_str).unwrap_or("");
            let title = link
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            (relation.contains("data#")
                || title.contains("direct download")
                || href.to_ascii_lowercase().ends_with(".zip"))
            .then(|| href.to_string())
        })
}

fn legacy_attribute_map(entry: &Map<String, Value>) -> Map<String, Value> {
    let mut attributes = Map::new();
    let entries = entry
        .get("additional_attributes")
        .or_else(|| entry.get("AdditionalAttributes"))
        .and_then(Value::as_array);
    let Some(entries) = entries else {
        return attributes;
    };
    for item in entries.iter().filter_map(Value::as_object) {
        let Some(name) = item
            .get("name")
            .or_else(|| item.get("Name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let key: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        let value = item
            .get("values")
            .or_else(|| item.get("Values"))
            .or_else(|| item.get("value"))
            .or_else(|| item.get("Value"))
            .and_then(first_non_empty);
        if let Some(value) = value {
            attributes.entry(key).or_insert(value.clone());
        }
    }
    attributes
}

fn first_non_empty(value: &Value) -> Option<&Value> {
    match value {
        Value::Array(items) => items.iter().find(|item| match item {
            Value::Null => false,
            Value::String(text) => !text.is_empty(),
            _ => true,
        }),
        Value::Null => None,
        Value::String(text) if text.is_empty() => None,
        value => Some(value),
    }
}

fn legacy_polygon_geometry(entry: &Map<String, Value>) -> Option<Map<String, Value>> {
    let polygons = entry.get("polygons")?.as_array()?;
    let mut rings = Vec::new();
    for polygon in polygons {
        let raw = match polygon {
            Value::Array(items) => items.first().and_then(Value::as_str),
            Value::String(text) => Some(text.as_str()),
            _ => None,
        };
        let Some(raw) = raw else {
            continue;
        };
        let numbers = raw
            .replace(',', " ")
            .split_whitespace()
            .filter_map(|text| text.parse::<f64>().ok())
            .collect::<Vec<_>>();
        let mut ring = numbers
            .chunks_exact(2)
            .filter_map(|pair| {
                let (latitude, longitude) = (pair[0], pair[1]);
                ((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude))
                    .then(|| vec![Value::from(longitude), Value::from(latitude)])
            })
            .collect::<Vec<_>>();
        if ring.len() >= 3 {
            if ring.first() != ring.last() {
                ring.push(ring[0].clone());
            }
            rings.push(Value::Array(ring.into_iter().map(Value::Array).collect()));
        }
    }
    if rings.is_empty() {
        return None;
    }
    let (kind, coordinates) = if rings.len() == 1 {
        ("Polygon", Value::Array(rings))
    } else {
        (
            "MultiPolygon",
            Value::Array(
                rings
                    .into_iter()
                    .map(|ring| Value::Array(vec![ring]))
                    .collect(),
            ),
        )
    };
    Some(Map::from_iter([
        ("type".to_string(), Value::String(kind.to_string())),
        ("coordinates".to_string(), coordinates),
    ]))
}
