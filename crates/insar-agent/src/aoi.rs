//! AOI facts that may leave the machine: name, bounding box and area. The
//! geometry stays local; the model names it only by reference and the host
//! resolves the reference when it freezes a plan.
use serde_json::{json, Map, Value};

/// The AOI the user attached to the message.
pub const CONTEXT_REF: &str = "context_aoi";
/// The desktop's current working area.
pub const WORKING_REF: &str = "working_aoi";

/// The GeoJSON geometry inside an AOI value: a bare geometry, a Feature, or a
/// wrapper carrying `geometry` / `geojson` / `aoi_geojson`.
pub fn geometry(aoi: &Value) -> Option<&Map<String, Value>> {
    let object = aoi.as_object()?;
    if object.contains_key("coordinates") {
        return Some(object);
    }
    ["geometry", "geojson", "aoi_geojson"]
        .iter()
        .find_map(|key| object.get(*key).and_then(geometry))
}

pub fn name(aoi: &Value) -> Option<String> {
    let object = aoi.as_object()?;
    ["name", "display_name", "aoi_name"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .or_else(|| {
            object
                .get("properties")
                .and_then(|properties| properties.get("name"))
                .and_then(Value::as_str)
        })
        .map(|name| name.chars().take(160).collect())
}

fn points(value: &Value, output: &mut Vec<(f64, f64)>) {
    let Some(items) = value.as_array() else {
        return;
    };
    match (
        items.first().and_then(Value::as_f64),
        items.get(1).and_then(Value::as_f64),
    ) {
        (Some(x), Some(y)) => output.push((x, y)),
        _ => items.iter().for_each(|item| points(item, output)),
    }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// `[west, south, east, north]` rounded to 1e-4 degrees, or null.
pub fn bbox(geometry: Option<&Map<String, Value>>) -> Value {
    let mut all = Vec::new();
    if let Some(geometry) = geometry {
        points(
            geometry.get("coordinates").unwrap_or(&Value::Null),
            &mut all,
        );
    }
    if all.is_empty() {
        return Value::Null;
    }
    let fold = |pick: fn(&(f64, f64)) -> f64, min: bool| {
        all.iter().map(pick).fold(
            if min {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            },
            |acc, v| if min { acc.min(v) } else { acc.max(v) },
        )
    };
    json!([
        round4(fold(|p| p.0, true)),
        round4(fold(|p| p.1, true)),
        round4(fold(|p| p.0, false)),
        round4(fold(|p| p.1, false)),
    ])
}

/// Approximate polygon area in km² (outer rings, local equirectangular
/// scale), one decimal, or null.
pub fn area_km2(geometry: Option<&Map<String, Value>>) -> Value {
    let Some(geometry) = geometry else {
        return Value::Null;
    };
    let rings: Vec<&Value> = match geometry.get("type").and_then(Value::as_str) {
        Some("Polygon") => geometry
            .get("coordinates")
            .and_then(Value::as_array)
            .and_then(|rings| rings.first())
            .into_iter()
            .collect(),
        Some("MultiPolygon") => geometry
            .get("coordinates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|polygon| polygon.as_array().and_then(|rings| rings.first()))
            .collect(),
        _ => vec![],
    };
    let mut total = 0.0_f64;
    for ring in rings {
        let points: Vec<(f64, f64)> = ring
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
            .collect();
        if points.len() < 3 {
            continue;
        }
        let mean_lat = points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64;
        let kx = 111.320 * mean_lat.to_radians().cos();
        let ky = 110.574;
        let mut sum = 0.0;
        for pair in points.windows(2) {
            sum += pair[0].0 * kx * pair[1].1 * ky - pair[1].0 * kx * pair[0].1 * ky;
        }
        let (first, last) = (points[0], points[points.len() - 1]);
        sum += last.0 * kx * first.1 * ky - first.0 * kx * last.1 * ky;
        total += (sum / 2.0).abs();
    }
    if total == 0.0 {
        Value::Null
    } else {
        json!((total * 10.0).round() / 10.0)
    }
}

/// What the model may see of the AOI attached to a message.
pub fn context_summary(aoi: &Value) -> Value {
    let geometry = geometry(aoi);
    json!({
        "name": name(aoi),
        "bbox": bbox(geometry),
        "area_km2": area_km2(geometry),
        "geometry_ref": geometry.map(|_| CONTEXT_REF),
    })
}

/// What `working_aoi_get` returns: state, name, bbox, area and a reference.
pub fn working_summary(raw: &Value) -> Value {
    let geometry = geometry(raw);
    let state = raw
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or(if geometry.is_some() { "active" } else { "none" });
    let bbox = match bbox(geometry) {
        Value::Null => raw
            .get("bbox")
            .and_then(Value::as_array)
            .filter(|values| values.len() == 4)
            .and_then(|values| values.iter().map(Value::as_f64).collect::<Option<Vec<_>>>())
            .map_or(Value::Null, |values| {
                json!(values.into_iter().map(round4).collect::<Vec<_>>())
            }),
        computed => computed,
    };
    json!({
        "state": state,
        "local_ref": (state != "none").then_some(WORKING_REF),
        "display_name": raw.get("display_name").and_then(Value::as_str),
        "bbox": bbox,
        "area_km2": area_km2(geometry),
    })
}

/// Default area warning/guardrail ceiling (km²) for automated place-name AOI
/// binding (~half a medium province; prevents homonym province/country pulls
/// from silently queuing massive multi-hundred-scene downloads).
pub const DEFAULT_MAX_AOI_AREA_KM2: f64 = 150_000.0;
/// Default maximum allowed distance (km) between a resolved polygon centroid
/// and a reference geocode point before treating the polygon as a homonym mismatch.
pub const DEFAULT_MAX_CENTROID_DRIFT_KM: f64 = 120.0;
/// Default minimum vertex count expected for a non-degenerate polygon ring.
pub const DEFAULT_MIN_POLYGON_VERTICES: usize = 4;
/// Default half-span in degrees (~9 km latitudinal radius) for safe fallback bboxes.
pub const DEFAULT_FALLBACK_HALF_SPAN_DEG: f64 = 0.08;

/// Great-circle distance in kilometers between two WGS84 `(lat, lon)` points.
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0088_f64;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let lat1_r = lat1.to_radians();
    let lat2_r = lat2.to_radians();
    let a = (dlat / 2.0).sin().powi(2) + lat1_r.cos() * lat2_r.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.clamp(0.0, 1.0).sqrt().asin();
    r * c
}

/// Validates a `[west, south, east, north]` WGS84 bounding box.
pub fn is_valid_bbox(bbox: [f64; 4]) -> bool {
    let [west, south, east, north] = bbox;
    west.is_finite()
        && south.is_finite()
        && east.is_finite()
        && north.is_finite()
        && (-180.0..=180.0).contains(&west)
        && (-180.0..=180.0).contains(&east)
        && (-90.0..=90.0).contains(&south)
        && (-90.0..=90.0).contains(&north)
        && west < east
        && south < north
}

/// Approximate area in km² of a `[west, south, east, north]` WGS84 bounding box
/// using spherical strip integration (accurate across latitudes, equal-area).
pub fn bbox_area_km2(bbox: [f64; 4]) -> Option<f64> {
    if !is_valid_bbox(bbox) {
        return None;
    }
    let [west, south, east, north] = bbox;
    let r = 6371.0088_f64;
    let dlon = (east - west).to_radians();
    let sin_diff = (north.to_radians().sin() - south.to_radians().sin()).abs();
    let area = r * r * dlon * sin_diff;
    if area.is_finite() && area > 0.0 {
        Some((area * 10.0).round() / 10.0)
    } else {
        None
    }
}

/// Midpoint `(lat, lon)` of a valid `[west, south, east, north]` bounding box.
pub fn bbox_centroid(bbox: [f64; 4]) -> Option<(f64, f64)> {
    if !is_valid_bbox(bbox) {
        return None;
    }
    let [west, south, east, north] = bbox;
    Some((round4(0.5 * (south + north)), round4(0.5 * (west + east))))
}

/// Count of coordinate vertices inside a GeoJSON geometry object.
pub fn vertex_count(geometry: Option<&Map<String, Value>>) -> usize {
    let mut all = Vec::new();
    if let Some(geom) = geometry {
        points(geom.get("coordinates").unwrap_or(&Value::Null), &mut all);
    }
    all.len()
}

/// Builds a clamped WGS84 `[west, south, east, north]` box centered on `(lat, lon)`.
pub fn reference_fallback_bbox(lat: f64, lon: f64, half_span_deg: f64) -> Option<[f64; 4]> {
    if !lat.is_finite()
        || !lon.is_finite()
        || !half_span_deg.is_finite()
        || !(-89.9..=89.9).contains(&lat)
        || !(-180.0..=180.0).contains(&lon)
    {
        return None;
    }
    let span = half_span_deg.clamp(0.005, 2.0);
    let west = (lon - span).clamp(-180.0, 179.99);
    let east = (lon + span).clamp(-179.99, 180.0);
    let south = (lat - span).clamp(-89.9, 89.89);
    let north = (lat + span).clamp(-89.89, 89.9);
    let candidate = [round4(west), round4(south), round4(east), round4(north)];
    is_valid_bbox(candidate).then_some(candidate)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AoiValidationConfig {
    pub max_area_km2: f64,
    pub max_centroid_drift_km: f64,
    pub min_polygon_vertices: usize,
    pub fallback_half_span_deg: f64,
}

impl Default for AoiValidationConfig {
    fn default() -> Self {
        Self {
            max_area_km2: DEFAULT_MAX_AOI_AREA_KM2,
            max_centroid_drift_km: DEFAULT_MAX_CENTROID_DRIFT_KM,
            min_polygon_vertices: DEFAULT_MIN_POLYGON_VERTICES,
            fallback_half_span_deg: DEFAULT_FALLBACK_HALF_SPAN_DEG,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AoiValidationStatus {
    Valid,
    OversizedWarning,
    CentroidDriftFallback,
    DegenerateFallback,
}

impl AoiValidationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::OversizedWarning => "oversized_warning",
            Self::CentroidDriftFallback => "centroid_drift_fallback",
            Self::DegenerateFallback => "degenerate_fallback",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AoiValidationReport {
    pub status: AoiValidationStatus,
    pub area_km2: Option<f64>,
    pub vertex_count: usize,
    pub centroid_lat_lon: Option<(f64, f64)>,
    pub centroid_drift_km: Option<f64>,
    pub effective_bbox: Option<[f64; 4]>,
    pub fallback_applied: bool,
    pub reason_zh: String,
    pub recommended_action_zh: String,
}

impl AoiValidationReport {
    /// Outbound-safe JSON summary (never contains raw polygon coordinates).
    pub fn to_json(&self) -> Value {
        json!({
            "status": self.status.as_str(),
            "area_km2": self.area_km2,
            "vertex_count": self.vertex_count,
            "centroid_lat_lon": self.centroid_lat_lon.map(|(lat, lon)| json!([lat, lon])),
            "centroid_drift_km": self.centroid_drift_km,
            "effective_bbox": self.effective_bbox,
            "fallback_applied": self.fallback_applied,
            "reason_zh": self.reason_zh,
            "recommended_action_zh": self.recommended_action_zh,
        })
    }
}

/// Dual-source AOI sanity validator inspired by geospatial boundary cross-checks:
/// checks geometry degeneracy, centroid drift against an optional reference
/// geocode coordinate, and equal-area cap against `config.max_area_km2`.
pub fn validate_place_aoi(
    geom: Option<&Map<String, Value>>,
    candidate_bbox: Option<[f64; 4]>,
    reference_lat_lon: Option<(f64, f64)>,
    config: &AoiValidationConfig,
) -> AoiValidationReport {
    let vertices = vertex_count(geom);
    let geom_bbox = bbox(geom)
        .as_array()
        .filter(|arr| arr.len() == 4)
        .and_then(|arr| {
            let b = [
                arr[0].as_f64()?,
                arr[1].as_f64()?,
                arr[2].as_f64()?,
                arr[3].as_f64()?,
            ];
            is_valid_bbox(b).then_some(b)
        });
    let resolved_bbox = geom_bbox.or_else(|| candidate_bbox.filter(|b| is_valid_bbox(*b)));
    let computed_area = area_km2(geom)
        .as_f64()
        .or_else(|| resolved_bbox.and_then(bbox_area_km2));
    let centroid = resolved_bbox.and_then(bbox_centroid);
    let fallback_from_ref = reference_lat_lon
        .and_then(|(lat, lon)| reference_fallback_bbox(lat, lon, config.fallback_half_span_deg));

    // 1. Degenerate geometry / missing valid bbox
    let geom_degenerate = geom.is_some() && vertices < config.min_polygon_vertices;
    if resolved_bbox.is_none() || geom_degenerate {
        return AoiValidationReport {
            status: AoiValidationStatus::DegenerateFallback,
            area_km2: fallback_from_ref.and_then(bbox_area_km2),
            vertex_count: vertices,
            centroid_lat_lon: reference_lat_lon.map(|(lat, lon)| (round4(lat), round4(lon))),
            centroid_drift_km: None,
            effective_bbox: fallback_from_ref,
            fallback_applied: fallback_from_ref.is_some(),
            reason_zh: "候选边界缺少有效多边形或顶点数不足，已拒绝退化几何".into(),
            recommended_action_zh: if fallback_from_ref.is_some() {
                "已基于参考坐标生成安全包围盒，请向用户确认范围后再启动检索".into()
            } else {
                "请补充省市/国家限定词重新检索，或请用户在地图上框选目标研究区".into()
            },
        };
    }

    // 2. Centroid drift against reference coordinate (catches homonym place mismatch)
    let drift_km = match (centroid, reference_lat_lon) {
        (Some((clat, clon)), Some((rlat, rlon))) => {
            Some((haversine_km(clat, clon, rlat, rlon) * 10.0).round() / 10.0)
        }
        _ => None,
    };
    if let (Some(drift), Some(fallback)) = (drift_km, fallback_from_ref) {
        if drift > config.max_centroid_drift_km {
            return AoiValidationReport {
                status: AoiValidationStatus::CentroidDriftFallback,
                area_km2: bbox_area_km2(fallback),
                vertex_count: vertices,
                centroid_lat_lon: centroid,
                centroid_drift_km: Some(drift),
                effective_bbox: Some(fallback),
                fallback_applied: true,
                reason_zh: format!(
                    "候选边界质心偏离参考坐标约 {drift:.1} km（超出 {:.0} km 容差），疑似同名异地边界",
                    config.max_centroid_drift_km
                ),
                recommended_action_zh:
                    "已自动回退到参考坐标周边的安全包围盒，建议确认具体地名归属或直接使用回退范围"
                        .into(),
            };
        }
    }

    // 3. Equal-area cap check (prevents accidental province/country-wide mass download)
    if let Some(area) = computed_area {
        if area > config.max_area_km2 {
            let suggested = fallback_from_ref.or_else(|| {
                centroid.and_then(|(lat, lon)| {
                    reference_fallback_bbox(lat, lon, config.fallback_half_span_deg)
                })
            });
            return AoiValidationReport {
                status: AoiValidationStatus::OversizedWarning,
                area_km2: Some(area),
                vertex_count: vertices,
                centroid_lat_lon: centroid,
                centroid_drift_km: drift_km,
                effective_bbox: suggested.or(resolved_bbox),
                fallback_applied: suggested.is_some(),
                reason_zh: format!(
                    "候选研究区面积约 {area:.0} km²，超出单次自动化检索建议上限 {:.0} km²（可能覆盖整省或超大区域）",
                    config.max_area_km2
                ),
                recommended_action_zh:
                    "建议缩小至市/县或核心灾区范围（或使用建议的局部包围盒），避免一次拉取过多景数与海量流量"
                        .into(),
            };
        }
    }

    AoiValidationReport {
        status: AoiValidationStatus::Valid,
        area_km2: computed_area,
        vertex_count: vertices,
        centroid_lat_lon: centroid,
        centroid_drift_km: drift_km,
        effective_bbox: resolved_bbox,
        fallback_applied: false,
        reason_zh: "研究区边界几何、面积与位置校验通过".into(),
        recommended_action_zh: "可直接用于 working_aoi_set 与影像检索".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_city_polygon_passes_dual_source_validation() {
        let poly = json!({
            "type": "Polygon",
            "coordinates": [[[117.1, 31.7], [117.5, 31.7], [117.5, 32.0], [117.1, 32.0], [117.1, 31.7]]]
        });
        let report = validate_place_aoi(
            geometry(&poly),
            None,
            Some((31.85, 117.3)),
            &AoiValidationConfig::default(),
        );
        assert_eq!(report.status, AoiValidationStatus::Valid);
        assert!(!report.fallback_applied);
        assert!(report.area_km2.unwrap() > 1000.0 && report.area_km2.unwrap() < 2000.0);
        assert!(report.centroid_drift_km.unwrap() < 5.0);
        assert!(!report.to_json().to_string().contains("coordinates"));
    }

    #[test]
    fn oversized_province_triggers_area_cap_warning_and_bounded_suggestion() {
        // Roughly Sichuan-sized bounding box (~480k+ km²)
        let bbox = [97.35, 26.05, 108.55, 34.32];
        let report = validate_place_aoi(
            None,
            Some(bbox),
            Some((30.67, 104.06)),
            &AoiValidationConfig::default(),
        );
        assert_eq!(report.status, AoiValidationStatus::OversizedWarning);
        assert!(report.fallback_applied);
        assert!(report.area_km2.unwrap() > DEFAULT_MAX_AOI_AREA_KM2);
        let eff = report.effective_bbox.unwrap();
        assert!(bbox_area_km2(eff).unwrap() < 500.0);
    }

    #[test]
    fn homonym_centroid_drift_falls_back_to_reference_bbox() {
        // Candidate bbox in Beijing, reference point in Chengdu (~1500 km away)
        let beijing_bbox = [116.2, 39.8, 116.6, 40.1];
        let report = validate_place_aoi(
            None,
            Some(beijing_bbox),
            Some((30.67, 104.06)),
            &AoiValidationConfig::default(),
        );
        assert_eq!(report.status, AoiValidationStatus::CentroidDriftFallback);
        assert!(report.fallback_applied);
        assert!(report.centroid_drift_km.unwrap() > 1000.0);
        let eff = report.effective_bbox.unwrap();
        assert!((eff[0]..=eff[2]).contains(&104.06));
        assert!((eff[1]..=eff[3]).contains(&30.67));
    }

    #[test]
    fn degenerate_point_geometry_falls_back_cleanly() {
        let point = json!({
            "type": "Point",
            "coordinates": [104.06, 30.67]
        });
        let report = validate_place_aoi(
            geometry(&point),
            None,
            Some((30.67, 104.06)),
            &AoiValidationConfig::default(),
        );
        assert_eq!(report.status, AoiValidationStatus::DegenerateFallback);
        assert!(report.fallback_applied);
        assert!(report.effective_bbox.is_some());
    }
}
