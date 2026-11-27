use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InclusiveRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalBbox {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
    pub crs: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalAsfQuery {
    pub version: u8,
    pub dataset: String,
    pub bbox: Option<CanonicalBbox>,
    pub aoi: String,
    pub start: String,
    pub end: String,
    pub product_type: String,
    pub beam_mode: String,
    pub polarization: String,
    pub orbit_direction: String,
    pub relative_orbit: String,
    pub frame: String,
}

pub fn normalize_filter(value: &str) -> String {
    value
        .split([',', ';'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_uppercase)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",")
}

pub fn normalize_date(value: &str) -> String {
    value.trim().replace('/', "-")
}

pub fn normalize_ranges(value: &str) -> Vec<InclusiveRange> {
    value
        .split([',', ';'])
        .filter_map(|part| {
            let item = part.trim();
            if item.is_empty() {
                return None;
            }
            let delimiter = if item.contains('-') {
                '-'
            } else if item.contains(':') {
                ':'
            } else {
                let value = item.parse::<u64>().ok()?;
                return Some(InclusiveRange {
                    start: value,
                    end: value,
                });
            };
            let mut pieces = item.split(delimiter);
            let first = pieces.next()?.trim().parse::<u64>().ok()?;
            let second = pieces.next()?.trim().parse::<u64>().ok()?;
            if pieces.next().is_some() {
                return None;
            }
            Some(InclusiveRange {
                start: first.min(second),
                end: first.max(second),
            })
        })
        .collect()
}

fn round_seven(value: f64) -> f64 {
    (value * 10_000_000.0).round() / 10_000_000.0
}

pub fn normalize_bbox(west: f64, east: f64, south: f64, north: f64, crs: &str) -> CanonicalBbox {
    CanonicalBbox {
        west: round_seven(west),
        east: round_seven(east),
        south: round_seven(south),
        north: round_seven(north),
        crs: if crs.is_empty() { "EPSG:4326" } else { crs }.to_string(),
    }
}

fn normalized_number(number: &Number) -> Value {
    let Some(value) = number.as_f64() else {
        return Value::Number(number.clone());
    };
    if number.is_i64() || number.is_u64() {
        return Value::Number(number.clone());
    }
    Number::from_f64(round_seven(value))
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn normalize_geojson_value(value: &Value) -> Value {
    match value {
        Value::Number(number) => normalized_number(number),
        Value::Array(items) => Value::Array(items.iter().map(normalize_geojson_value).collect()),
        Value::Object(object) => {
            let object_type = object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match object_type {
                "Feature" => object
                    .get("geometry")
                    .map(normalize_geojson_value)
                    .unwrap_or(Value::Null),
                "FeatureCollection" => {
                    let features = object
                        .get("features")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_object)
                        .map(|feature| {
                            if feature.get("type").and_then(Value::as_str) == Some("Feature") {
                                feature
                                    .get("geometry")
                                    .map(normalize_geojson_value)
                                    .unwrap_or(Value::Null)
                            } else {
                                normalize_geojson_value(&Value::Object(feature.clone()))
                            }
                        })
                        .collect();
                    Value::Object(Map::from_iter([
                        ("type".to_string(), Value::String(object_type.to_string())),
                        ("features".to_string(), Value::Array(features)),
                    ]))
                }
                "GeometryCollection" => {
                    let geometries = object
                        .get("geometries")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter(|item| item.is_object())
                        .map(normalize_geojson_value)
                        .collect();
                    Value::Object(Map::from_iter([
                        ("type".to_string(), Value::String(object_type.to_string())),
                        ("geometries".to_string(), Value::Array(geometries)),
                    ]))
                }
                "" => Value::Object(Map::from_iter(
                    object
                        .iter()
                        .map(|(key, item)| (key.clone(), normalize_geojson_value(item))),
                )),
                _ => Value::Object(Map::from_iter([
                    ("type".to_string(), Value::String(object_type.to_string())),
                    (
                        "coordinates".to_string(),
                        object
                            .get("coordinates")
                            .map(normalize_geojson_value)
                            .unwrap_or(Value::Null),
                    ),
                ])),
            }
        }
        _ => value.clone(),
    }
}

fn sha256_json(value: &Value) -> String {
    let raw = serde_json::to_vec(value).expect("JSON values always serialize");
    format!("{:x}", Sha256::digest(raw))
}

pub fn canonical_geojson_hash(value: &Value) -> String {
    if !value.is_object() {
        return String::new();
    }
    sha256_json(&normalize_geojson_value(value))
}

pub fn canonical_query_key(query: &CanonicalAsfQuery) -> String {
    let value = serde_json::to_value(query).expect("canonical queries always serialize");
    sha256_json(&value)
}
