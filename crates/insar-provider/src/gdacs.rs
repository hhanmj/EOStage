//! Keyless public Global Disaster Alert and Coordination System (GDACS) client.
//!
//! Queries the EC JRC / UN OCHA public read-only GeoJSON endpoint
//! (`https://www.gdacs.org/gdacsapi/api/events/geteventlist/SEARCH`) without requiring
//! an API key or user account. Normalizes active global earthquakes (`EQ`), floods
//! (`FL`), tropical cyclones (`TC`), volcanoes (`VO`), wildfires (`WF`), and
//! droughts (`DR`) into WGS84 bounding boxes and recommended pre/post-disaster
//! imagery retrieval time windows for EOStage data preparation.
//!
//! Strict honesty contract: network or parse failures return an explicit error
//! and never fabricate demo or mock disaster events.

use crate::network_proxy::blocking_builder_with_proxy;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, NaiveDateTime, Utc};
use insar_core::ProxyPolicy;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;

pub const GDACS_EVENTS_SEARCH_URL: &str =
    "https://www.gdacs.org/gdacsapi/api/events/geteventlist/SEARCH";

const DEFAULT_EVENT_TYPES: &[&str] = &["EQ", "FL", "TC", "VO", "WF"];
const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 30;
const POINT_FALLBACK_HALF_SPAN_DEG: f64 = 0.25;

#[derive(Debug, Error)]
pub enum GdacsError {
    #[error("failed to initialize GDACS HTTP client: {0}")]
    ClientBuild(String),
    #[error("GDACS request failed: {0}")]
    Transport(String),
    #[error("GDACS service returned HTTP {0}")]
    HttpStatus(u16),
    #[error("invalid GDACS GeoJSON payload: {0}")]
    InvalidPayload(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GdacsQuery {
    /// Raw hazard codes or Chinese/English aliases (e.g. `["EQ", "FL"]`, `["地震", "洪水"]`).
    /// Empty defaults to `EQ,FL,TC,VO,WF`.
    #[serde(default)]
    pub event_types: Vec<String>,
    /// Optional case-insensitive substring filter over event name, country, or description.
    #[serde(default)]
    pub keyword: Option<String>,
    /// Minimum alert severity: `"green"` (default), `"orange"`, or `"red"`.
    #[serde(default)]
    pub min_alert_level: Option<String>,
    /// Optional WGS84 `[west, south, east, north]` filter (intersects event bbox).
    #[serde(default)]
    pub bbox: Option<[f64; 4]>,
    /// Maximum returned events (1..=30, default 10).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GdacsDisasterEvent {
    pub event_id: String,
    pub event_type: String,
    pub event_type_zh: String,
    pub name: String,
    pub country: Option<String>,
    pub alert_level: String,
    pub severity_score: f64,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    /// WGS84 `[west, south, east, north]`.
    pub bbox: Option<[f64; 4]>,
    /// `"gdacs_extent"` when GDACS supplied a feature bbox, or `"point_buffer"` when
    /// synthesized from the event epicenter/point geometry.
    pub bbox_source: Option<String>,
    /// Suggested pre-disaster baseline search window `[start_rfc3339, end_rfc3339]`.
    pub recommended_pre_window: Option<[String; 2]>,
    /// Suggested post-disaster imagery search window `[start_rfc3339, end_rfc3339]`.
    pub recommended_post_window: Option<[String; 2]>,
    pub report_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GdacsSearchResponse {
    pub source: String,
    pub source_url: String,
    pub queried_event_types: Vec<String>,
    pub total_matched: usize,
    pub events: Vec<GdacsDisasterEvent>,
}

/// Maps Chinese/English hazard terms and official codes into normalized GDACS codes.
pub fn normalize_event_type_code(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "eq" | "earthquake" | "earthquakes" | "quake" => Some("EQ"),
        "fl" | "flood" | "floods" | "inundation" => Some("FL"),
        "tc" | "cyclone" | "typhoon" | "hurricane" | "tropical_cyclone" | "storm" => Some("TC"),
        "vo" | "volcano" | "volcanoes" | "eruption" => Some("VO"),
        "wf" | "wildfire" | "wildfires" | "fire" | "forest_fire" => Some("WF"),
        "dr" | "drought" | "droughts" => Some("DR"),
        _ => {
            if trimmed.contains("地震") || trimmed.contains("强震") {
                Some("EQ")
            } else if trimmed.contains("洪水")
                || trimmed.contains("洪涝")
                || trimmed.contains("暴雨")
            {
                Some("FL")
            } else if trimmed.contains("台风")
                || trimmed.contains("飓风")
                || trimmed.contains("气旋")
            {
                Some("TC")
            } else if trimmed.contains("火山") {
                Some("VO")
            } else if trimmed.contains("野火")
                || trimmed.contains("山火")
                || trimmed.contains("林火")
                || trimmed.contains("火灾")
            {
                Some("WF")
            } else if trimmed.contains("干旱") {
                Some("DR")
            } else {
                None
            }
        }
    }
}

pub fn event_type_label_zh(code: &str) -> &'static str {
    match code {
        "EQ" => "地震",
        "FL" => "洪水/洪涝",
        "TC" => "热带气旋/台风",
        "VO" => "火山",
        "WF" => "野火/林火",
        "DR" => "干旱",
        _ => "其他灾害",
    }
}

fn alert_rank(level: &str) -> u8 {
    match level.trim().to_ascii_lowercase().as_str() {
        "red" | "红" | "红色" => 3,
        "orange" | "橙" | "橙色" => 2,
        "green" | "绿" | "绿色" => 1,
        _ => 1,
    }
}

fn alert_severity_score(level: &str) -> f64 {
    match alert_rank(level) {
        3 => 0.9,
        2 => 0.6,
        _ => 0.3,
    }
}

fn normalize_alert_level(raw: Option<&str>, icon_url: Option<&str>) -> String {
    if let Some(level) = raw.map(str::trim).filter(|s| !s.is_empty()) {
        match level.to_ascii_lowercase().as_str() {
            "red" => return "red".to_owned(),
            "orange" => return "orange".to_owned(),
            "green" => return "green".to_owned(),
            _ => {}
        }
    }
    if let Some(icon) = icon_url {
        let lower = icon.to_ascii_lowercase();
        for level in ["red", "orange", "green"] {
            if lower.contains(&format!("/{level}/")) || lower.contains(&format!("{level}.png")) {
                return level.to_owned();
            }
        }
    }
    "green".to_owned()
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn valid_bbox(b: [f64; 4]) -> bool {
    let [w, s, e, n] = b;
    w.is_finite()
        && s.is_finite()
        && e.is_finite()
        && n.is_finite()
        && (-180.0..=180.0).contains(&w)
        && (-180.0..=180.0).contains(&e)
        && (-90.0..=90.0).contains(&s)
        && (-90.0..=90.0).contains(&n)
        && w <= e
        && s <= n
}

fn bboxes_intersect(a: [f64; 4], b: [f64; 4]) -> bool {
    !(b[0] > a[2] || b[2] < a[0] || b[1] > a[3] || b[3] < a[1])
}

fn parse_feature_bbox(feature: &Value) -> (Option<[f64; 4]>, Option<String>) {
    if let Some(arr) = feature.get("bbox").and_then(Value::as_array) {
        if arr.len() >= 4 {
            if let (Some(w), Some(s), Some(e), Some(n)) = (
                arr[0].as_f64(),
                arr[1].as_f64(),
                arr[2].as_f64(),
                arr[3].as_f64(),
            ) {
                let candidate = [round4(w), round4(s), round4(e), round4(n)];
                if valid_bbox(candidate)
                    && candidate[0] < candidate[2]
                    && candidate[1] < candidate[3]
                {
                    return (Some(candidate), Some("gdacs_extent".to_owned()));
                }
                // Degenerate zero-area point bbox returned by some GDACS point events:
                // expand symmetrically around the point.
                if valid_bbox(candidate) {
                    let lon = candidate[0];
                    let lat = candidate[1];
                    let buffered = [
                        round4((lon - POINT_FALLBACK_HALF_SPAN_DEG).clamp(-180.0, 179.9)),
                        round4((lat - POINT_FALLBACK_HALF_SPAN_DEG).clamp(-89.9, 89.8)),
                        round4((lon + POINT_FALLBACK_HALF_SPAN_DEG).clamp(-179.9, 180.0)),
                        round4((lat + POINT_FALLBACK_HALF_SPAN_DEG).clamp(-89.8, 89.9)),
                    ];
                    if valid_bbox(buffered) {
                        return (Some(buffered), Some("point_buffer".to_owned()));
                    }
                }
            }
        }
    }
    // Fallback to GeoJSON Point coordinates `[lon, lat]`
    if let Some(coords) = feature
        .pointer("/geometry/coordinates")
        .and_then(Value::as_array)
    {
        if let (Some(lon), Some(lat)) = (
            coords.first().and_then(Value::as_f64),
            coords.get(1).and_then(Value::as_f64),
        ) {
            if (-180.0..=180.0).contains(&lon) && (-89.9..=89.9).contains(&lat) {
                let buffered = [
                    round4((lon - POINT_FALLBACK_HALF_SPAN_DEG).clamp(-180.0, 179.9)),
                    round4((lat - POINT_FALLBACK_HALF_SPAN_DEG).clamp(-89.9, 89.8)),
                    round4((lon + POINT_FALLBACK_HALF_SPAN_DEG).clamp(-179.9, 180.0)),
                    round4((lat + POINT_FALLBACK_HALF_SPAN_DEG).clamp(-89.8, 89.9)),
                ];
                if valid_bbox(buffered) {
                    return (Some(buffered), Some("point_buffer".to_owned()));
                }
            }
        }
    }
    (None, None)
}

fn parse_gdacs_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S") {
        return Some(ndt.and_utc());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
        return Some(ndt.and_utc());
    }
    if let Ok(date) = NaiveDate::parse_from_str(&trimmed[..trimmed.len().min(10)], "%Y-%m-%d") {
        return date.and_hms_opt(0, 0, 0).map(|ndt| ndt.and_utc());
    }
    None
}

fn recommended_windows(
    from_raw: Option<&str>,
    to_raw: Option<&str>,
) -> (Option<[String; 2]>, Option<[String; 2]>) {
    let Some(from_dt) = from_raw.and_then(parse_gdacs_timestamp) else {
        return (None, None);
    };
    let pre_start = (from_dt - ChronoDuration::days(30)).format("%Y-%m-%dT00:00:00Z");
    let pre_end = (from_dt - ChronoDuration::days(1)).format("%Y-%m-%dT23:59:59Z");

    let post_start = from_dt.format("%Y-%m-%dT00:00:00Z");
    let to_dt = to_raw
        .and_then(parse_gdacs_timestamp)
        .filter(|dt| *dt >= from_dt)
        .unwrap_or_else(|| from_dt + ChronoDuration::days(14));
    let post_end = (to_dt + ChronoDuration::days(7)).format("%Y-%m-%dT23:59:59Z");

    (
        Some([pre_start.to_string(), pre_end.to_string()]),
        Some([post_start.to_string(), post_end.to_string()]),
    )
}

fn sanitize_report_url(raw: Option<&str>) -> Option<String> {
    let url = raw?.trim();
    if url.starts_with("https://") && url.len() <= 512 {
        Some(url.to_owned())
    } else {
        None
    }
}

/// Resolves requested event types into deduplicated GDACS codes (defaults to EQ/FL/TC/VO/WF).
pub fn resolve_query_event_types(raw_types: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in raw_types {
        for piece in raw.split([',', ';', ' ', '、']) {
            if let Some(code) = normalize_event_type_code(piece) {
                if !out.iter().any(|existing| existing == code) {
                    out.push(code.to_owned());
                }
            }
        }
    }
    if out.is_empty() {
        DEFAULT_EVENT_TYPES
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    } else {
        out
    }
}

/// Parses and filters a GDACS GeoJSON `FeatureCollection` payload offline.
pub fn parse_gdacs_geojson(
    payload: &Value,
    query: &GdacsQuery,
    queried_types: Vec<String>,
) -> Result<GdacsSearchResponse, GdacsError> {
    let features = payload
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| GdacsError::InvalidPayload("missing 'features' array".into()))?;

    let min_rank = query
        .min_alert_level
        .as_deref()
        .map(alert_rank)
        .unwrap_or(1);
    let keyword_lower = query
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let filter_bbox = query.bbox.filter(|b| valid_bbox(*b));
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let mut events = Vec::new();
    for feat in features {
        let Some(props) = feat.get("properties").and_then(Value::as_object) else {
            continue;
        };
        let event_type = props
            .get("eventtype")
            .and_then(Value::as_str)
            .and_then(normalize_event_type_code)
            .unwrap_or("OTHER")
            .to_owned();
        if !queried_types.iter().any(|t| t == &event_type) {
            continue;
        }
        let alert_level = normalize_alert_level(
            props.get("alertlevel").and_then(Value::as_str),
            props.get("icon").and_then(Value::as_str),
        );
        if alert_rank(&alert_level) < min_rank {
            continue;
        }
        let (bbox, bbox_source) = parse_feature_bbox(feat);
        if let Some(req_bbox) = filter_bbox {
            let Some(ev_bbox) = bbox else {
                continue;
            };
            if !bboxes_intersect(req_bbox, ev_bbox) {
                continue;
            }
        }
        let event_id = props
            .get("eventid")
            .map(|v| match v {
                Value::String(s) => s.trim().to_owned(),
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".to_owned());
        let name = props
            .get("name")
            .or_else(|| props.get("htmldescription"))
            .or_else(|| props.get("description"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Unnamed GDACS event")
            .to_owned();
        let country = props
            .get("country")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let desc = props
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let type_zh = event_type_label_zh(&event_type).to_owned();

        if let Some(kw) = &keyword_lower {
            let haystack = format!(
                "{} {} {} {} {}",
                name.to_lowercase(),
                country.as_deref().unwrap_or("").to_lowercase(),
                desc.to_lowercase(),
                event_type.to_lowercase(),
                type_zh.to_lowercase()
            );
            if !haystack.contains(kw) {
                continue;
            }
        }

        let from_date = props
            .get("fromdate")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let to_date = props
            .get("todate")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let (recommended_pre_window, recommended_post_window) =
            recommended_windows(from_date.as_deref(), to_date.as_deref());
        let report_url = sanitize_report_url(
            props
                .get("url")
                .and_then(|u| u.get("report"))
                .and_then(Value::as_str),
        );

        events.push(GdacsDisasterEvent {
            event_id,
            event_type,
            event_type_zh: type_zh,
            name,
            country,
            alert_level: alert_level.clone(),
            severity_score: alert_severity_score(&alert_level),
            from_date,
            to_date,
            bbox,
            bbox_source,
            recommended_pre_window,
            recommended_post_window,
            report_url,
        });
    }

    events.sort_by(|a, b| {
        b.severity_score
            .total_cmp(&a.severity_score)
            .then_with(|| b.from_date.cmp(&a.from_date))
            .then_with(|| a.event_id.cmp(&b.event_id))
    });
    let total_matched = events.len();
    events.truncate(limit);

    Ok(GdacsSearchResponse {
        source: "GDACS (EC JRC / UN OCHA)".into(),
        source_url: GDACS_EVENTS_SEARCH_URL.into(),
        queried_event_types: queried_types,
        total_matched,
        events,
    })
}

fn gdacs_alertlevel_param(min_alert_level: Option<&str>) -> &'static str {
    match min_alert_level.map(alert_rank).unwrap_or(1) {
        3 => "Red",
        2 => "Orange;Red",
        _ => "Green;Orange;Red",
    }
}

/// Live keyless query to GDACS public active-event GeoJSON API.
pub fn search_gdacs_events(
    query: &GdacsQuery,
    proxy_policy: ProxyPolicy,
) -> Result<GdacsSearchResponse, GdacsError> {
    let queried_types = resolve_query_event_types(&query.event_types);
    let eventlist_param = queried_types.join(";");
    let alertlevel_param = gdacs_alertlevel_param(query.min_alert_level.as_deref());
    let client = blocking_builder_with_proxy(
        Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("EOStage/1.0 (GDACS Read-Only Disaster Locator)"),
        proxy_policy,
    )
    .build()
    .map_err(|e| GdacsError::ClientBuild(e.to_string()))?;

    let response = client
        .get(GDACS_EVENTS_SEARCH_URL)
        .query(&[
            ("eventlist", eventlist_param.as_str()),
            ("alertlevel", alertlevel_param),
        ])
        .send()
        .map_err(|e| GdacsError::Transport(e.to_string()))?;

    let status = response.status();
    if status.as_u16() == 204 {
        return Ok(GdacsSearchResponse {
            source: "GDACS (EC JRC / UN OCHA)".into(),
            source_url: GDACS_EVENTS_SEARCH_URL.into(),
            queried_event_types: queried_types,
            total_matched: 0,
            events: Vec::new(),
        });
    }
    if !status.is_success() {
        return Err(GdacsError::HttpStatus(status.as_u16()));
    }
    let payload: Value = response
        .json()
        .map_err(|e| GdacsError::InvalidPayload(e.to_string()))?;

    parse_gdacs_geojson(&payload, query, queried_types)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_gdacs_geojson() -> Value {
        json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "bbox": [103.5, 30.2, 104.5, 31.1],
                    "geometry": { "type": "Point", "coordinates": [104.0, 30.65] },
                    "properties": {
                        "eventid": 1001,
                        "eventtype": "EQ",
                        "name": "Earthquake in Sichuan, China",
                        "country": "China",
                        "alertlevel": "Red",
                        "fromdate": "2026-11-10T04:15:00",
                        "todate": "2026-11-10T04:15:00",
                        "url": { "report": "https://www.gdacs.org/report.aspx?eventid=1001&eventtype=EQ" }
                    }
                },
                {
                    "type": "Feature",
                    "bbox": [100.1, 13.5, 100.1, 13.5],
                    "geometry": { "type": "Point", "coordinates": [100.1, 13.5] },
                    "properties": {
                        "eventid": 2002,
                        "eventtype": "FL",
                        "name": "Flood in Chao Phraya Basin",
                        "country": "Thailand",
                        "icon": "https://www.gdacs.org/images/gdacs_icons/maps/Orange/FL.png",
                        "fromdate": "2026-11-01T00:00:00",
                        "todate": "2026-11-08T00:00:00",
                        "url": { "report": "https://www.gdacs.org/report.aspx?eventid=2002&eventtype=FL" }
                    }
                },
                {
                    "type": "Feature",
                    "bbox": [-119.0, 33.8, -118.2, 34.4],
                    "geometry": { "type": "Point", "coordinates": [-118.6, 34.1] },
                    "properties": {
                        "eventid": 3003,
                        "eventtype": "WF",
                        "name": "Wildfire near Los Angeles",
                        "country": "United States",
                        "alertlevel": "Green",
                        "fromdate": "2026-11-05T12:00:00",
                        "todate": "2026-11-07T12:00:00"
                    }
                }
            ]
        })
    }

    #[test]
    fn resolves_chinese_and_english_hazard_aliases() {
        assert_eq!(
            resolve_query_event_types(&["地震".into(), "flood".into(), "山火".into()]),
            vec!["EQ", "FL", "WF"]
        );
        assert_eq!(
            resolve_query_event_types(&[]),
            vec!["EQ", "FL", "TC", "VO", "WF"]
        );
    }

    #[test]
    fn parses_filters_sorts_and_computes_pre_post_windows() {
        let query = GdacsQuery {
            event_types: vec!["地震".into(), "洪水".into()],
            min_alert_level: Some("orange".into()),
            ..Default::default()
        };
        let types = resolve_query_event_types(&query.event_types);
        let resp = parse_gdacs_geojson(&sample_gdacs_geojson(), &query, types).unwrap();
        assert_eq!(resp.total_matched, 2);
        // Red EQ ranks above Orange FL
        assert_eq!(resp.events[0].event_id, "1001");
        assert_eq!(resp.events[0].alert_level, "red");
        assert_eq!(resp.events[0].bbox, Some([103.5, 30.2, 104.5, 31.1]));
        assert_eq!(resp.events[0].bbox_source.as_deref(), Some("gdacs_extent"));
        assert_eq!(
            resp.events[0].recommended_pre_window,
            Some(["2026-10-11T00:00:00Z".into(), "2026-11-09T23:59:59Z".into()])
        );
        assert_eq!(
            resp.events[0].recommended_post_window,
            Some(["2026-11-10T00:00:00Z".into(), "2026-11-17T23:59:59Z".into()])
        );

        // Zero-area point bbox expands to point_buffer & falls back to icon alert level
        assert_eq!(resp.events[1].event_id, "2002");
        assert_eq!(resp.events[1].alert_level, "orange");
        assert_eq!(resp.events[1].bbox_source.as_deref(), Some("point_buffer"));
        let b = resp.events[1].bbox.unwrap();
        assert!(b[0] < 100.1 && b[2] > 100.1);
    }

    #[test]
    fn filters_by_keyword_and_bbox_intersection() {
        let query = GdacsQuery {
            keyword: Some("Sichuan".into()),
            bbox: Some([103.0, 30.0, 105.0, 32.0]),
            ..Default::default()
        };
        let types = resolve_query_event_types(&query.event_types);
        let resp = parse_gdacs_geojson(&sample_gdacs_geojson(), &query, types).unwrap();
        assert_eq!(resp.events.len(), 1);
        assert_eq!(resp.events[0].event_id, "1001");
    }

    #[test]
    #[ignore = "live HTTPS network call to public GDACS API"]
    fn live_gdacs_global_disaster_events_return_real_geojson() {
        let query = GdacsQuery {
            event_types: vec!["EQ".into(), "FL".into(), "TC".into(), "WF".into()],
            limit: Some(5),
            ..Default::default()
        };
        let resp = search_gdacs_events(&query, ProxyPolicy::System)
            .expect("public keyless GDACS API should return active global events");
        assert!(
            !resp.events.is_empty(),
            "expected at least one active global GDACS disaster event"
        );
        for ev in &resp.events {
            assert!(!ev.event_id.is_empty());
            assert!(ev.bbox.is_some(), "event {} missing bbox", ev.event_id);
            eprintln!(
                "GDACS live event: [{}] {} ({}) alert={} bbox={:?} post={:?}",
                ev.event_type,
                ev.name,
                ev.event_id,
                ev.alert_level,
                ev.bbox,
                ev.recommended_post_window
            );
        }
    }
}
