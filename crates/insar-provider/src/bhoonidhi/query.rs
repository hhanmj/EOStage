use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, SecondsFormat, Utc};
use insar_core::{BBox, CatalogSearchQuery, ContractError, ProviderResult};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use url::Url;

use super::{BHOONIDHI_COLLECTION_ID, BHOONIDHI_PROVIDER_ID};

pub const BHOONIDHI_CURSOR_VERSION: &str = "bhoonidhi-cursor-v1";
pub const BHOONIDHI_MAX_CURSOR_BYTES: usize = 24 * 1024;
pub const BHOONIDHI_MAX_PAGE_SIZE: u64 = 500;
pub const BHOONIDHI_MAX_AOI_POSITIONS: usize = 16 * 1024;
pub const BHOONIDHI_MAX_AOI_DEPTH: usize = 64;
pub const BHOONIDHI_MAX_FILTER_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct BhoonidhiSearchRequest {
    body: Map<String, Value>,
    fingerprint: String,
    continuation: Option<BhoonidhiContinuation>,
}

impl BhoonidhiSearchRequest {
    pub fn body(&self) -> &Map<String, Value> {
        &self.body
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub(crate) fn continuation(&self) -> Option<&BhoonidhiContinuation> {
        self.continuation.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BhoonidhiContinuation {
    pub href: String,
    pub method: String,
    pub body: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CursorEnvelope {
    version: String,
    fingerprint: String,
    href: String,
    method: String,
    body: Map<String, Value>,
}

fn error(code: &str, message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded Bhoonidhi query error is safe")
}

fn cursor_error(reason: &str) -> ContractError {
    error(
        "BHOONIDHI_CURSOR_INVALID",
        "Bhoonidhi 续页标识无效或与原始查询不匹配",
        reason,
    )
}

fn query_error(reason: &str) -> ContractError {
    let message_zh = match reason {
        "aoi_too_complex" | "aoi_too_deep" => {
            "研究区多边形边界顶点过多（超过 Bhoonidhi 接口上限），请改用矩形或简化多边形框选"
        }
        _ => "Bhoonidhi S-RSLC 目录查询无效",
    };
    error("BHOONIDHI_QUERY_INVALID", message_zh, reason)
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "access_token",
        "refresh_token",
        "authorization",
        "cookie",
        "secret",
        "credential",
        "signed_url",
        "download_url",
    ]
    .iter()
    .any(|item| key == *item)
        || (key != "token" && key.ends_with("token"))
        || key.ends_with("_password")
        || key.contains("jwt")
        || (key.contains("signed") && key.contains("url"))
}

fn contains_sensitive_key(value: &Value) -> bool {
    match value {
        Value::Object(map) => map
            .iter()
            .any(|(key, value)| is_sensitive_key(key) || contains_sensitive_key(value)),
        Value::Array(values) => values.iter().any(contains_sensitive_key),
        _ => false,
    }
}

fn contains_unsafe_locator(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.values().any(contains_unsafe_locator),
        Value::Array(values) => values.iter().any(contains_unsafe_locator),
        Value::String(value) if value.starts_with("http://") || value.starts_with("https://") => {
            Url::parse(value)
                .map(|url| {
                    !url.username().is_empty()
                        || url.password().is_some()
                        || url.query().is_some()
                        || url.fragment().is_some()
                })
                .unwrap_or(true)
        }
        _ => false,
    }
}

fn authoritative_string(value: Option<&Value>, field: &str) -> ProviderResult<String> {
    let value = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && *value == value.trim())
        .filter(|value| !value.chars().any(char::is_control))
        .ok_or_else(|| query_error(field))?;
    Ok(value.to_owned())
}

fn parse_utc(value: &str, field: &str) -> ProviderResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| query_error(field))
}

fn normalize_utc(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn datetime_filter(value: &Value) -> ProviderResult<String> {
    match value {
        Value::Object(map) => {
            if map.len() != 2 || map.keys().any(|key| key != "start" && key != "end") {
                return Err(query_error("datetime_shape"));
            }
            let start = authoritative_string(map.get("start"), "datetime.start")?;
            let end = authoritative_string(map.get("end"), "datetime.end")?;
            let start_time = parse_utc(&start, "datetime.start")?;
            let end_time = parse_utc(&end, "datetime.end")?;
            if start_time >= end_time {
                return Err(query_error("datetime_order"));
            }
            Ok(format!(
                "{}/{}",
                normalize_utc(start_time),
                normalize_utc(end_time)
            ))
        }
        Value::String(value) => {
            let (start, end) = value
                .split_once('/')
                .ok_or_else(|| query_error("datetime_range"))?;
            let start_time = parse_utc(start, "datetime.start")?;
            let end_time = parse_utc(end, "datetime.end")?;
            if start_time >= end_time {
                return Err(query_error("datetime_order"));
            }
            Ok(format!(
                "{}/{}",
                normalize_utc(start_time),
                normalize_utc(end_time)
            ))
        }
        _ => Err(query_error("datetime_type")),
    }
}

fn property_for_filter(key: &str) -> Option<&'static str> {
    match key {
        "mode" | "Mode" => Some("Mode"),
        "polarization" | "Polarization" => Some("Polarization"),
        "acquisition_mode" | "AcquisitionMode" | "AcquistionMode" => Some("AcquistionMode"),
        "product_code" | "ProductCode" => Some("ProductCode"),
        "software_version" | "SoftwareVersion" => Some("SoftwareVersion"),
        "producer" | "Producer" => Some("Producer"),
        "platform" | "Platform" => Some("Platform"),
        "instrument" | "Instrument" => Some("Instrument"),
        "scene_identifier" | "Scene_Identifier" => Some("Scene_Identifier"),
        _ => None,
    }
}

fn exact_filter_clause(property: &str, value: &Value, key: &str) -> ProviderResult<Value> {
    let value = match value {
        Value::String(value)
            if !value.is_empty()
                && value == value.trim()
                && !value.chars().any(char::is_control) =>
        {
            Value::String(value.clone())
        }
        _ => return Err(query_error(key)),
    };
    Ok(serde_json::json!({
        "args": [{"property": property}, value],
        "op": "eq"
    }))
}

fn online_clause() -> Value {
    serde_json::json!({
        "args": [{"property": "Online"}, "Y"],
        "op": "eq"
    })
}

fn build_filter(filters: &Map<String, Value>) -> ProviderResult<Value> {
    let mut clauses = vec![online_clause()];
    let mut seen_properties = BTreeSet::new();
    for (key, value) in filters {
        if key == "datetime" {
            continue;
        }
        if key == "online" || key == "Online" {
            let online = authoritative_string(Some(value), key)?;
            if online != "Y" {
                return Err(query_error("online_must_be_y"));
            }
            continue;
        }
        let property = property_for_filter(key).ok_or_else(|| query_error("unknown_filter"))?;
        if !seen_properties.insert(property) {
            return Err(query_error("duplicate_filter_alias"));
        }
        clauses.push(exact_filter_clause(property, value, key)?);
    }
    if clauses.len() == 1 {
        Ok(clauses.remove(0))
    } else {
        Ok(serde_json::json!({"args": clauses, "op": "and"}))
    }
}

fn number(value: f64) -> Value {
    Value::Number(Number::from_f64(value).expect("validated finite coordinate"))
}

fn coordinate_pair(value: &Value, field: &str) -> ProviderResult<(f64, f64)> {
    let values = value
        .as_array()
        .filter(|values| values.len() >= 2)
        .ok_or_else(|| query_error(field))?;
    let longitude = values[0]
        .as_f64()
        .filter(|value| value.is_finite() && (-180.0..=180.0).contains(value))
        .ok_or_else(|| query_error("longitude"))?;
    let latitude = values[1]
        .as_f64()
        .filter(|value| value.is_finite() && (-90.0..=90.0).contains(value))
        .ok_or_else(|| query_error("latitude"))?;
    Ok((longitude, latitude))
}

fn collect_coordinates(
    value: &Value,
    positions: &mut Vec<(f64, f64)>,
    field: &str,
    depth: usize,
) -> ProviderResult<()> {
    if depth > BHOONIDHI_MAX_AOI_DEPTH {
        return Err(query_error("aoi_too_deep"));
    }
    let values = value.as_array().ok_or_else(|| query_error(field))?;
    if values.len() >= 2 && values[0].is_number() && values[1].is_number() {
        positions.push(coordinate_pair(value, field)?);
        if positions.len() > BHOONIDHI_MAX_AOI_POSITIONS {
            return Err(query_error("aoi_too_complex"));
        }
        return Ok(());
    }
    for value in values {
        collect_coordinates(value, positions, field, depth + 1)?;
    }
    Ok(())
}

fn collect_geometry_positions(
    value: &Value,
    positions: &mut Vec<(f64, f64)>,
    depth: usize,
) -> ProviderResult<()> {
    if depth > BHOONIDHI_MAX_AOI_DEPTH {
        return Err(query_error("aoi_too_deep"));
    }
    let object = value.as_object().ok_or_else(|| query_error("geometry"))?;
    match object.get("type").and_then(Value::as_str) {
        Some("Polygon") | Some("MultiPolygon") => collect_coordinates(
            object
                .get("coordinates")
                .ok_or_else(|| query_error("coordinates"))?,
            positions,
            "coordinates",
            depth + 1,
        ),
        Some("Feature") => collect_geometry_positions(
            object
                .get("geometry")
                .ok_or_else(|| query_error("feature_geometry"))?,
            positions,
            depth + 1,
        ),
        Some("FeatureCollection") => {
            let features = object
                .get("features")
                .and_then(Value::as_array)
                .ok_or_else(|| query_error("features"))?;
            if features.is_empty() {
                return Err(query_error("features_empty"));
            }
            for feature in features {
                collect_geometry_positions(feature, positions, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(query_error("geometry_type")),
    }
}

/// Derive canonical EPSG:4326 bounds from polygonal GeoJSON coordinates.
///
/// Provider or caller supplied bbox arrays are intentionally ignored. Bhoonidhi
/// has returned a bbox ordering that is not safe to treat as canonical STAC
/// ordering, while the geometry is authoritative for this adapter.
pub fn derive_bbox_from_geojson(value: &Value) -> ProviderResult<BBox> {
    let mut positions = Vec::new();
    collect_geometry_positions(value, &mut positions, 0)?;
    let Some((first_longitude, first_latitude)) = positions.first().copied() else {
        return Err(query_error("coordinates_empty"));
    };
    let (west, east, south, north) = positions.iter().skip(1).fold(
        (
            first_longitude,
            first_longitude,
            first_latitude,
            first_latitude,
        ),
        |(west, east, south, north), (longitude, latitude)| {
            (
                west.min(*longitude),
                east.max(*longitude),
                south.min(*latitude),
                north.max(*latitude),
            )
        },
    );
    let bbox = BBox {
        west,
        east,
        south,
        north,
        crs: "EPSG:4326".to_owned(),
    };
    bbox.validate()
        .map_err(|_| query_error("bbox_degenerate"))?;
    Ok(bbox)
}

fn scope_fingerprint(scope: &Map<String, Value>) -> ProviderResult<String> {
    let bytes = serde_json::to_vec(scope).map_err(|_| query_error("scope_serialize"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn scope_body(query: &CatalogSearchQuery) -> ProviderResult<Map<String, Value>> {
    query.validate().map_err(|_| query_error("generic_query"))?;
    if query.provider_id != BHOONIDHI_PROVIDER_ID {
        return Err(query_error("provider_identity"));
    }
    if query.native_collection_id != BHOONIDHI_COLLECTION_ID {
        return Err(query_error("collection_identity"));
    }
    if query.logical_dataset != super::BHOONIDHI_LOGICAL_DATASET {
        return Err(query_error("dataset_identity"));
    }

    let mut body = Map::new();
    body.insert(
        "collections".to_owned(),
        Value::Array(vec![Value::String(BHOONIDHI_COLLECTION_ID.to_owned())]),
    );
    if let Some(aoi) = query.aoi_geojson.as_ref() {
        let bbox = derive_bbox_from_geojson(&Value::Object(aoi.clone()))?;
        body.insert(
            "bbox".to_owned(),
            Value::Array(
                [bbox.west, bbox.south, bbox.east, bbox.north]
                    .into_iter()
                    .map(number)
                    .collect(),
            ),
        );
    }
    if let Some(datetime) = query.filters.get("datetime") {
        body.insert(
            "datetime".to_owned(),
            Value::String(datetime_filter(datetime)?),
        );
    }
    body.insert("filter".to_owned(), build_filter(&query.filters)?);
    body.insert(
        "filter-lang".to_owned(),
        Value::String("cql2-json".to_owned()),
    );
    body.insert(
        "limit".to_owned(),
        Value::Number(serde_json::Number::from(
            query
                .result_limit
                .unwrap_or(BHOONIDHI_MAX_PAGE_SIZE)
                .min(BHOONIDHI_MAX_PAGE_SIZE),
        )),
    );
    Ok(body)
}

pub fn build_search_request(query: &CatalogSearchQuery) -> ProviderResult<BhoonidhiSearchRequest> {
    let query = &insar_core::clamp_catalogue_search_query(query);
    let scope = scope_body(query)?;
    let fingerprint = scope_fingerprint(&scope)?;
    let (body, continuation) = match query.cursor.as_deref() {
        None => (scope.clone(), None),
        Some(cursor) => {
            let continuation = decode_cursor(cursor, &fingerprint)?;
            validate_continuation_scope(&continuation.body, &scope)?;
            (continuation.body.clone(), Some(continuation))
        }
    };
    Ok(BhoonidhiSearchRequest {
        body,
        fingerprint,
        continuation,
    })
}

fn filter_terms(
    value: &Value,
    terms: &mut Vec<(String, Value)>,
    depth: usize,
) -> ProviderResult<()> {
    if depth > BHOONIDHI_MAX_FILTER_DEPTH {
        return Err(cursor_error("filter_too_deep"));
    }
    let object = value
        .as_object()
        .ok_or_else(|| cursor_error("filter_shape"))?;
    let op = object
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| cursor_error("filter_operator"))?;
    let args = object
        .get("args")
        .and_then(Value::as_array)
        .ok_or_else(|| cursor_error("filter_args"))?;
    match op {
        "and" => {
            if args.is_empty() {
                return Err(cursor_error("filter_args"));
            }
            for arg in args {
                filter_terms(arg, terms, depth + 1)?;
            }
        }
        "=" | "eq" => {
            if args.len() != 2 {
                return Err(cursor_error("filter_args"));
            }
            let property = args[0]
                .as_object()
                .and_then(|object| object.get("property"))
                .and_then(Value::as_str)
                .filter(|property| !property.is_empty() && !property.chars().any(char::is_control))
                .ok_or_else(|| cursor_error("filter_property"))?;
            if contains_sensitive_key(&args[1]) {
                return Err(cursor_error("sensitive_filter"));
            }
            terms.push((property.to_owned(), args[1].clone()));
        }
        _ => return Err(cursor_error("filter_operator")),
    }
    Ok(())
}

fn filter_counts(value: &Value) -> ProviderResult<BTreeMap<String, usize>> {
    let mut terms = Vec::new();
    filter_terms(value, &mut terms, 0)?;
    let mut counts = BTreeMap::new();
    for (property, value) in terms {
        let key = format!(
            "{property}:{}",
            serde_json::to_string(&value).map_err(|_| cursor_error("filter_value"))?
        );
        *counts.entry(key).or_insert(0) += 1;
    }
    Ok(counts)
}

fn validate_continuation_fields(value: Option<&Value>) -> ProviderResult<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let object = value
        .as_object()
        .ok_or_else(|| cursor_error("fields_shape"))?;
    if object.len() != 1 {
        return Err(cursor_error("fields_shape"));
    }
    let exclude = object
        .get("exclude")
        .and_then(Value::as_array)
        .ok_or_else(|| cursor_error("fields_exclude"))?;
    let allowed = ["assets.data", "properties.MetaData", "properties.IsDeleted"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut values = BTreeSet::new();
    for item in exclude {
        let item = item
            .as_str()
            .filter(|item| !item.is_empty() && !item.chars().any(char::is_control))
            .ok_or_else(|| cursor_error("fields_exclude"))?;
        if !values.insert(item) || !allowed.contains(item) {
            return Err(cursor_error("fields_exclude"));
        }
    }
    if values != allowed {
        return Err(cursor_error("fields_exclude"));
    }
    Ok(())
}

fn validate_bbox_scope(
    body: &Map<String, Value>,
    scope: &Map<String, Value>,
) -> ProviderResult<()> {
    match (body.get("bbox"), scope.get("bbox")) {
        (None, None) => Ok(()),
        (Some(actual), Some(expected)) => {
            let actual = actual
                .as_array()
                .filter(|values| values.len() == 4)
                .ok_or_else(|| cursor_error("scope_bbox"))?;
            let expected = expected
                .as_array()
                .filter(|values| values.len() == 4)
                .ok_or_else(|| cursor_error("scope_bbox"))?;
            if actual.iter().zip(expected).all(|(actual, expected)| {
                actual
                    .as_f64()
                    .zip(expected.as_f64())
                    .is_some_and(|(actual, expected)| (actual - expected).abs() <= 1e-6)
            }) {
                Ok(())
            } else {
                Err(cursor_error("scope_bbox"))
            }
        }
        _ => Err(cursor_error("scope_bbox")),
    }
}

fn datetime_matches_scope(actual: Option<&Value>, expected: Option<&Value>) -> bool {
    if actual == expected {
        return true;
    }
    let (Some(Value::String(actual_str)), Some(Value::String(expected_str))) = (actual, expected)
    else {
        return false;
    };
    let normalize = |raw: &str| -> Option<String> {
        let (start, end) = raw.split_once('/')?;
        datetime_filter(&serde_json::json!({"start": start, "end": end})).ok()
    };
    match (normalize(actual_str), normalize(expected_str)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn validate_continuation_scope(
    body: &Map<String, Value>,
    scope: &Map<String, Value>,
) -> ProviderResult<()> {
    for key in body.keys() {
        if !scope.contains_key(key) && !matches!(key.as_str(), "token" | "fields") {
            return Err(cursor_error("unexpected_paging_field"));
        }
    }
    for key in ["collections", "filter-lang", "limit"] {
        if body.get(key) != scope.get(key) {
            return Err(cursor_error("scope_mismatch"));
        }
    }
    if !datetime_matches_scope(body.get("datetime"), scope.get("datetime")) {
        return Err(cursor_error("scope_mismatch"));
    }
    validate_bbox_scope(body, scope)?;
    let expected_filter = scope
        .get("filter")
        .ok_or_else(|| cursor_error("scope_filter"))?;
    let actual_filter = body
        .get("filter")
        .ok_or_else(|| cursor_error("scope_filter"))?;
    let expected_counts = filter_counts(expected_filter)?;
    let actual_counts = filter_counts(actual_filter)?;
    let is_deleted_key = format!(
        "IsDeleted:{}",
        serde_json::to_string("N").map_err(|_| cursor_error("filter_value"))?
    );
    for (key, expected_count) in &expected_counts {
        if actual_counts.get(key).copied().unwrap_or_default() < *expected_count {
            return Err(cursor_error("scope_filter"));
        }
    }
    for (key, actual_count) in &actual_counts {
        let expected_count = expected_counts.get(key).copied().unwrap_or_default();
        if *actual_count > expected_count
            && !(key == &is_deleted_key && *actual_count >= 1 && expected_count == 0)
        {
            return Err(cursor_error("scope_filter"));
        }
    }
    if let Some(token) = body.get("token") {
        if !matches!(token, Value::String(value) if !value.is_empty() && value.len() <= 16 * 1024) {
            return Err(cursor_error("paging_token"));
        }
    }
    validate_continuation_fields(body.get("fields"))
}

pub(crate) fn decode_cursor(
    value: &str,
    expected_fingerprint: &str,
) -> ProviderResult<BhoonidhiContinuation> {
    if value.len() > BHOONIDHI_MAX_CURSOR_BYTES {
        return Err(cursor_error("cursor_size"));
    }
    let mut parts = value.splitn(3, ':');
    let prefix = parts.next();
    let fingerprint = parts.next();
    let encoded = parts.next();
    if prefix != Some(BHOONIDHI_CURSOR_VERSION)
        || fingerprint != Some(expected_fingerprint)
        || encoded.is_none()
    {
        return Err(cursor_error("fingerprint"));
    }
    let encoded = encoded.expect("cursor payload was checked");
    let bytes = hex_decode(encoded).ok_or_else(|| cursor_error("cursor_encoding"))?;
    let envelope: CursorEnvelope =
        serde_json::from_slice(&bytes).map_err(|_| cursor_error("cursor_shape"))?;
    if envelope.version != BHOONIDHI_CURSOR_VERSION
        || envelope.fingerprint != expected_fingerprint
        || envelope.method != "POST"
        || envelope.href.is_empty()
        || contains_sensitive_key(&Value::Object(envelope.body.clone()))
        || contains_unsafe_locator(&Value::Object(envelope.body.clone()))
    {
        return Err(cursor_error("cursor_scope"));
    }
    Ok(BhoonidhiContinuation {
        href: envelope.href,
        method: envelope.method,
        body: envelope.body,
    })
}

pub(crate) fn wrap_next_cursor(
    fingerprint: &str,
    href: &str,
    method: &str,
    body: &Map<String, Value>,
) -> ProviderResult<String> {
    if method != "POST" || fingerprint.len() != 64 {
        return Err(cursor_error("next_link_scope"));
    }
    let parsed = Url::parse(href).map_err(|_| cursor_error("next_link_scope"))?;
    if !matches!(parsed.scheme(), "https" | "http")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path().is_empty()
    {
        return Err(cursor_error("next_link_scope"));
    }
    let body_value = Value::Object(body.clone());
    if contains_sensitive_key(&body_value) {
        return Err(cursor_error("sensitive_next_link"));
    }
    if contains_unsafe_locator(&body_value) {
        return Err(cursor_error("unsafe_next_link"));
    }
    let envelope = CursorEnvelope {
        version: BHOONIDHI_CURSOR_VERSION.to_owned(),
        fingerprint: fingerprint.to_owned(),
        href: href.to_owned(),
        method: method.to_owned(),
        body: body.clone(),
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|_| cursor_error("cursor_serialize"))?;
    let cursor = format!(
        "{BHOONIDHI_CURSOR_VERSION}:{fingerprint}:{}",
        hex_encode(&bytes)
    );
    if cursor.len() > BHOONIDHI_MAX_CURSOR_BYTES {
        return Err(cursor_error("cursor_size"));
    }
    Ok(cursor)
}

pub(crate) fn parse_next_link(value: &Value, fingerprint: &str) -> ProviderResult<Option<String>> {
    let links = value
        .get("links")
        .and_then(Value::as_array)
        .ok_or_else(|| error("BHOONIDHI_RESPONSE_INVALID", "Bhoonidhi 响应无效", "links"))?;
    let mut cursor = None;
    for link in links {
        let object = link
            .as_object()
            .ok_or_else(|| error("BHOONIDHI_RESPONSE_INVALID", "Bhoonidhi 响应无效", "link"))?;
        if object.get("rel").and_then(Value::as_str) != Some("next") {
            continue;
        }
        if cursor.is_some() {
            return Err(cursor_error("duplicate_next_link"));
        }
        let href = object
            .get("href")
            .and_then(Value::as_str)
            .ok_or_else(|| cursor_error("next_href"))?;
        let method = object
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| cursor_error("next_method"))?;
        let body = object
            .get("body")
            .and_then(Value::as_object)
            .ok_or_else(|| cursor_error("next_body"))?;
        cursor = Some(wrap_next_cursor(fingerprint, href, method, body)?);
    }
    if let Some(context) = value.get("context").and_then(Value::as_object) {
        if let (Some(returned), Some(limit)) = (
            context.get("returned").and_then(Value::as_u64),
            context.get("limit").and_then(Value::as_u64),
        ) {
            if returned < limit {
                return Ok(None);
            }
        }
    }
    Ok(cursor)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_nibble(chunk[0])?;
            let low = hex_nibble(chunk[1])?;
            Some((high << 4) | low)
        })
        .collect()
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

pub(crate) fn validate_next_href(href: &str, endpoint: &Url) -> ProviderResult<()> {
    let parsed = Url::parse(href).map_err(|_| cursor_error("next_href_parse"))?;
    if parsed.scheme() != endpoint.scheme()
        || parsed.host_str() != endpoint.host_str()
        || parsed.port_or_known_default() != endpoint.port_or_known_default()
        || parsed.path() != endpoint.path()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(cursor_error("next_href_origin"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn query() -> CatalogSearchQuery {
        serde_json::from_value(json!({
            "logical_dataset": super::super::BHOONIDHI_LOGICAL_DATASET,
            "provider_id": BHOONIDHI_PROVIDER_ID,
            "native_collection_id": BHOONIDHI_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[68.0, 6.0], [98.0, 6.0], [98.0, 38.0], [68.0, 38.0], [68.0, 6.0]]]
            },
            "cursor": null,
            "result_limit": 2,
            "filters": {
                "datetime": {"start":"2026-07-08T00:00:00Z", "end":"2026-08-30T23:59:59Z"},
                "mode": "W",
                "polarization": "CP"
            }
        }))
        .expect("query fixture")
    }

    #[test]
    fn request_uses_geometry_derived_bbox_and_exact_online_filter() {
        let request = build_search_request(&query()).expect("request");
        assert_eq!(request.body()["bbox"], json!([68.0, 6.0, 98.0, 38.0]));
        assert_eq!(
            request.body()["collections"],
            json!([BHOONIDHI_COLLECTION_ID])
        );
        assert_eq!(request.body()["filter"]["args"][0]["args"][1], "Y");
        assert_eq!(request.body()["filter"]["args"][1]["args"][1], "W");
        assert_eq!(request.body()["filter"]["args"][2]["args"][1], "CP");
        assert_eq!(request.body()["limit"], 2);
    }

    #[test]
    fn hunan_owner_p0_query_uses_official_cql2_eq() {
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": super::super::BHOONIDHI_LOGICAL_DATASET,
            "provider_id": BHOONIDHI_PROVIDER_ID,
            "native_collection_id": BHOONIDHI_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[
                    [108.78801079615435, 24.639537509750618],
                    [114.25488646849476, 24.639537509750618],
                    [114.25488646849476, 30.128649782609237],
                    [108.78801079615435, 30.128649782609237],
                    [108.78801079615435, 24.639537509750618]
                ]]
            },
            "result_limit": 250,
            "filters": {
                "datetime": {
                    "start": "2026-07-04T00:00:00Z",
                    "end": "2026-10-04T23:59:59Z"
                },
                "mode": "W",
                "software_version": "1.4.00"
            }
        }))
        .expect("Hunan Owner P0 query fixture");

        let request = build_search_request(&query).expect("Hunan request");
        assert_eq!(
            request.body()["bbox"],
            json!([
                108.78801079615435,
                24.639537509750618,
                114.25488646849476,
                30.128649782609237
            ])
        );
        assert_eq!(
            request.body()["collections"],
            json!([BHOONIDHI_COLLECTION_ID])
        );
        assert_eq!(
            request.body()["datetime"],
            "2026-07-04T00:00:00Z/2026-10-04T23:59:59Z"
        );
        assert_eq!(request.body()["limit"], 250);
        assert_eq!(
            request.body()["filter"],
            json!({
                "op": "and",
                "args": [
                    {"op": "eq", "args": [{"property": "Online"}, "Y"]},
                    {"op": "eq", "args": [{"property": "Mode"}, "W"]},
                    {"op": "eq", "args": [{"property": "SoftwareVersion"}, "1.4.00"]}
                ]
            })
        );
    }

    #[test]
    fn mode_and_acquisition_mode_use_distinct_provider_properties() {
        let mut query = query();
        query.filters.remove("polarization");
        query
            .filters
            .insert("acquisition_mode".to_owned(), json!("J"));

        let request = build_search_request(&query).expect("request with distinct modes");
        assert_eq!(
            request.body()["filter"],
            json!({
                "op": "and",
                "args": [
                    {"op": "eq", "args": [{"property": "Online"}, "Y"]},
                    {"op": "eq", "args": [{"property": "AcquistionMode"}, "J"]},
                    {"op": "eq", "args": [{"property": "Mode"}, "W"]}
                ]
            })
        );
    }

    #[test]
    fn raw_bbox_is_not_used_as_aoi_authority() {
        let geometry = json!({
            "type": "Polygon",
            "bbox": [999, 999, 1000, 1000],
            "coordinates": [[[10.0, 20.0], [12.0, 20.0], [12.0, 22.0], [10.0, 22.0], [10.0, 20.0]]]
        });
        let bbox = derive_bbox_from_geojson(&geometry).expect("derived bbox");
        assert_eq!(
            (bbox.west, bbox.east, bbox.south, bbox.north),
            (10.0, 12.0, 20.0, 22.0)
        );
    }

    #[test]
    fn wrong_route_and_online_filter_fail_before_http() {
        let mut wrong = query();
        wrong.provider_id = "asf.nasa.earthdata".to_owned();
        assert_eq!(
            build_search_request(&wrong).unwrap_err().code,
            "BHOONIDHI_QUERY_INVALID"
        );

        let mut offline = query();
        offline.filters.insert("online".to_owned(), json!("N"));
        assert_eq!(
            build_search_request(&offline).unwrap_err().debug_context["reason"],
            "online_must_be_y"
        );
    }

    #[test]
    fn cursor_is_opaque_and_bound_to_the_original_scope() {
        let request = build_search_request(&query()).expect("request");
        let mut mixed_operator_filter = request.body()["filter"].clone();
        mixed_operator_filter["args"][0]["op"] = json!("=");
        assert_eq!(mixed_operator_filter["args"][0]["op"], "=");
        assert_eq!(mixed_operator_filter["args"][1]["op"], "eq");
        let body = Map::from_iter([
            ("collections".to_owned(), json!([BHOONIDHI_COLLECTION_ID])),
            ("bbox".to_owned(), request.body()["bbox"].clone()),
            ("datetime".to_owned(), request.body()["datetime"].clone()),
            (
                "filter".to_owned(),
                json!({
                    "args": [
                        mixed_operator_filter,
                        {"args":[{"property":"IsDeleted"},"N"],"op":"eq"}
                    ],
                    "op":"and"
                }),
            ),
            ("filter-lang".to_owned(), json!("cql2-json")),
            ("limit".to_owned(), json!(2)),
            (
                "fields".to_owned(),
                json!({"exclude":["assets.data","properties.MetaData","properties.IsDeleted"]}),
            ),
            ("token".to_owned(), json!("opaque-page-token")),
        ]);
        let cursor = wrap_next_cursor(
            request.fingerprint(),
            "https://bhoonidhi-api.nrsc.gov.in/data/search",
            "POST",
            &body,
        )
        .expect("cursor");
        assert!(!cursor.contains("opaque-page-token"));
        let mut continued = query();
        continued.cursor = Some(cursor);
        let resumed = build_search_request(&continued).expect("bound continuation");
        assert_eq!(resumed.body()["token"], "opaque-page-token");

        continued.filters.insert("mode".to_owned(), json!("X"));
        assert_eq!(
            build_search_request(&continued).unwrap_err().code,
            "BHOONIDHI_CURSOR_INVALID"
        );
    }

    #[test]
    fn actual_post_next_body_fixture_is_bound_to_original_query() {
        let mut query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": super::super::BHOONIDHI_LOGICAL_DATASET,
            "provider_id": BHOONIDHI_PROVIDER_ID,
            "native_collection_id": BHOONIDHI_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[68.0, 6.0], [98.0, 6.0], [98.0, 38.0], [68.0, 38.0], [68.0, 6.0]]]
            },
            "result_limit": 1,
            "filters": {
                "datetime": {"start":"2026-07-08T00:00:00Z", "end":"2026-08-30T23:59:59Z"}
            }
        }))
        .expect("catalogue query");
        let request = build_search_request(&query).expect("request");
        let next_body: Map<String, Value> =
            serde_json::from_str(super::super::fixtures::NEXT_BODY_JSON)
                .expect("next body fixture");
        let cursor = wrap_next_cursor(
            request.fingerprint(),
            "https://bhoonidhi-api.nrsc.gov.in/data/search",
            "POST",
            &next_body,
        )
        .expect("cursor");
        query.cursor = Some(cursor);
        let resumed = build_search_request(&query).expect("bound continuation");
        assert_eq!(resumed.body(), &next_body);
    }

    #[test]
    fn multi_page_cursor_allows_accumulated_is_deleted_and_float_tolerance() {
        let request = build_search_request(&query()).expect("request");
        let page2_filter = json!({
            "op": "and",
            "args": [
                {
                    "op": "and",
                    "args": [
                        request.body()["filter"].clone(),
                        {"args": [{"property": "IsDeleted"}, "N"], "op": "eq"}
                    ]
                },
                {"args": [{"property": "IsDeleted"}, "N"], "op": "eq"}
            ]
        });
        let bbox = request.body()["bbox"]
            .as_array()
            .expect("bbox")
            .iter()
            .map(|v| json!(v.as_f64().expect("f64") + 1e-9))
            .collect::<Vec<_>>();
        let body = Map::from_iter([
            ("collections".to_owned(), json!([BHOONIDHI_COLLECTION_ID])),
            ("bbox".to_owned(), Value::Array(bbox)),
            (
                "datetime".to_owned(),
                json!("2026-07-08T00:00:00.000Z/2026-08-30T23:59:59.000Z"),
            ),
            ("filter".to_owned(), page2_filter),
            ("filter-lang".to_owned(), json!("cql2-json")),
            ("limit".to_owned(), json!(2)),
            ("token".to_owned(), json!("next:page-3-token")),
        ]);
        let cursor = wrap_next_cursor(
            request.fingerprint(),
            "https://bhoonidhi-api.nrsc.gov.in/Bhoonidhi-DataSearch-1.0/v1/search",
            "POST",
            &body,
        )
        .expect("page 3 cursor");
        let mut continued = query();
        continued.cursor = Some(cursor);
        let resumed = build_search_request(&continued).expect("page 3 continuation succeeds");
        assert_eq!(resumed.body()["token"], "next:page-3-token");
    }

    #[test]
    fn complex_polygon_aoi_returns_clear_chinese_guidance() {
        let mut q = query();
        let mut ring: Vec<Value> = (0..(BHOONIDHI_MAX_AOI_POSITIONS + 10))
            .map(|i| {
                let t = (i as f64) * 0.0001;
                json!([78.0 + t, 30.0 + (i % 5) as f64 * 0.001])
            })
            .collect();
        ring.push(ring[0].clone());
        q.aoi_geojson = json!({
            "type": "Polygon",
            "coordinates": [ring]
        })
        .as_object()
        .cloned();
        let err = build_search_request(&q).unwrap_err();
        assert_eq!(err.code, "BHOONIDHI_QUERY_INVALID");
        assert_eq!(err.debug_context["reason"], "aoi_too_complex");
        assert!(err.user_message.contains("顶点过多"));
    }
}
