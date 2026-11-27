//! Native application-owned AOI, GACOS planning, and report capabilities.
//!
//! These routes are deliberately side-effect-light and provider-free.  They
//! validate/normalize common geometry and build deterministic offline plans.
//! Vector codecs are owned by the product-native GIS boundary. The service
//! remains the only owner of the user-facing operation and normalization
//! policy.

use super::{ApplicationService, ApplicationServiceError};
use base64::Engine as _;
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use insar_core::FrozenSearchBatch;
use insar_provider::gacos_delivery::GacosOutputFormat;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MAX_AOI_BYTES: usize = 64 * 1024 * 1024;
const MAX_AOI_FEATURES: usize = 500;
const MAX_GACOS_BATCH_DATES: usize = 20;
const MAX_GACOS_SPAN_DEGREES: f64 = 10.0;
type GacosSceneTime = (String, Option<(u32, u32)>, bool);

fn gacos_manual_dates(value: Option<&Value>) -> Result<Vec<String>, ApplicationServiceError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("GACOS manual_dates must be a non-empty array"))?;
    if values.is_empty() {
        return Err(invalid("GACOS manual_dates must contain at least one date"));
    }
    let mut dates = BTreeSet::new();
    for value in values {
        let text = value
            .as_str()
            .map(str::trim)
            .filter(|value| value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or_else(|| invalid("GACOS manual_dates must use YYYYMMDD strings"))?;
        if text.starts_with("0000") {
            return Err(invalid(
                "GACOS manual_dates contains an invalid calendar date",
            ));
        }
        let date = NaiveDate::parse_from_str(text, "%Y%m%d")
            .map_err(|_| invalid("GACOS manual_dates contains an invalid calendar date"))?;
        dates.insert(date.format("%Y-%m-%d").to_string());
    }
    Ok(dates.into_iter().collect())
}

fn gacos_manual_utc_time(
    supplied: &Map<String, Value>,
) -> Result<(u32, u32), ApplicationServiceError> {
    let hour = supplied
        .get("hour")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("GACOS manual UTC hour is required"))?;
    let minute = supplied
        .get("minute")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("GACOS manual UTC minute is required"))?;
    if hour > 23 || minute > 59 {
        return Err(invalid("GACOS manual UTC time is outside 00:00-23:59"));
    }
    Ok((hour as u32, minute as u32))
}

fn gacos_output_format(
    supplied: &Map<String, Value>,
) -> Result<GacosOutputFormat, ApplicationServiceError> {
    let value = supplied
        .get("output_format")
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase)
                .ok_or_else(|| invalid("GACOS output_format must be geotiff or binary"))
        })
        .transpose()?
        .unwrap_or_else(|| "geotiff".to_owned());
    serde_json::from_value(Value::String(value))
        .map_err(|_| invalid("GACOS output_format must be geotiff or binary"))
}

fn gacos_is_sentinel1_label(value: &str) -> bool {
    let normalized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    normalized == "sentinel1"
}

fn gacos_snapshot_scene_id(candidate: &insar_core::SnapshotItem) -> Option<String> {
    [
        candidate.metadata.get("scene_id"),
        candidate.payload.get("scene_id"),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn gacos_sentinel1_source_batch(batch: &FrozenSearchBatch) -> bool {
    let provider_id = batch.provider_id.trim().to_ascii_lowercase();
    if !matches!(
        provider_id.as_str(),
        "asf.catalog" | "asf.sentinel1" | "asf.sentinel-1"
    ) {
        return false;
    }
    // Current SAR batches freeze a typed `SarSearchQuery`, whose required
    // `dataset` field is the authoritative source identity.  Do not let an
    // arbitrary collection/debug field turn an ALOS (or other) batch into S1.
    let query_identifies_sentinel1 = batch
        .query
        .get("dataset")
        .and_then(Value::as_str)
        .is_some_and(gacos_is_sentinel1_label);
    query_identifies_sentinel1
        && !batch.candidates.is_empty()
        && batch.candidates.iter().all(|candidate| {
            candidate
                .provider_id
                .eq_ignore_ascii_case(&batch.provider_id)
                && candidate
                    .metadata
                    .get("dataset")
                    .and_then(Value::as_str)
                    .is_some_and(gacos_is_sentinel1_label)
                && gacos_snapshot_scene_id(candidate).is_some()
        })
}

fn gacos_scenes_from_source_batch(
    batch: &FrozenSearchBatch,
) -> Result<Vec<Value>, ApplicationServiceError> {
    batch
        .candidates
        .iter()
        .map(|candidate| {
            let scene_id = gacos_snapshot_scene_id(candidate)
                .ok_or_else(|| invalid("GACOS Sentinel-1 source candidate is missing scene_id"))?;
            let mut scene = Map::new();
            scene.insert("scene_id".to_owned(), Value::String(scene_id));
            if let Some(stable_scene_id) = candidate
                .metadata
                .get("stable_scene_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                scene.insert(
                    "stable_scene_id".to_owned(),
                    Value::String(stable_scene_id.trim().to_owned()),
                );
            }
            let acquisition_datetime = candidate
                .acquisition_datetime
                .as_deref()
                .or_else(|| {
                    candidate
                        .metadata
                        .get("acquisition_datetime")
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    candidate
                        .payload
                        .get("acquisition_datetime")
                        .and_then(Value::as_str)
                })
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let Some(acquisition_datetime) = acquisition_datetime {
                scene.insert(
                    "acquisition_datetime".to_owned(),
                    Value::String(acquisition_datetime.to_owned()),
                );
            } else if let Some(date) = candidate
                .metadata
                .get("date")
                .and_then(Value::as_str)
                .or_else(|| candidate.payload.get("date").and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                scene.insert("date".to_owned(), Value::String(date.to_owned()));
            }
            Ok(Value::Object(scene))
        })
        .collect()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ApplicationAoiBundleFile {
    pub name: String,
    pub base64: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ApplicationAoiImportRequest {
    pub source: String,
    pub path: Option<String>,
    pub file_name: Option<String>,
    pub text: Option<String>,
    pub base64_data: Option<String>,
    pub files: Option<Vec<ApplicationAoiBundleFile>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ApplicationAoiNormalizeRequest {
    pub geojson: Value,
    pub feature_ids: Vec<String>,
    pub name_field: Option<String>,
    pub download_mode: Option<String>,
}

impl Default for ApplicationAoiNormalizeRequest {
    fn default() -> Self {
        Self {
            geojson: Value::Null,
            feature_ids: Vec::new(),
            name_field: None,
            download_mode: Some("merge".to_owned()),
        }
    }
}

fn invalid(reason: impl Into<String>) -> ApplicationServiceError {
    ApplicationServiceError::InvalidRequest {
        reason: reason.into(),
    }
}

fn gacos_user_supplied_time(
    supplied: Option<&Map<String, Value>>,
    keys: &[&str],
) -> Result<Option<(u32, u32)>, ApplicationServiceError> {
    let Some(value) = supplied.and_then(|values| {
        keys.iter()
            .filter(|key| !key.is_empty())
            .find_map(|key| values.get(*key))
    }) else {
        return Ok(None);
    };
    let pair = if let Some(values) = value.as_array() {
        (
            values.first().and_then(Value::as_u64),
            values.get(1).and_then(Value::as_u64),
        )
    } else if let Some(values) = value.as_object() {
        (
            values.get("hour").and_then(Value::as_u64),
            values.get("minute").and_then(Value::as_u64),
        )
    } else {
        (None, None)
    };
    let (Some(hour), Some(minute)) = pair else {
        return Err(invalid(
            "GACOS user-supplied UTC time must contain hour and minute",
        ));
    };
    if hour > 23 || minute > 59 {
        return Err(invalid(
            "GACOS user-supplied UTC time is outside 00:00-23:59",
        ));
    }
    Ok(Some((hour as u32, minute as u32)))
}

fn gacos_scene_time(
    scene: &Value,
    supplied: Option<&Map<String, Value>>,
) -> Result<GacosSceneTime, ApplicationServiceError> {
    let scene_id = scene
        .get("scene_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let value = ["acquisition_datetime", "date"]
        .iter()
        .find_map(|key| {
            scene
                .get(*key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("GACOS selected scene is missing acquisition time"))?;
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        let utc = parsed.with_timezone(&Utc);
        return Ok((
            utc.format("%Y-%m-%d").to_string(),
            Some((utc.hour(), utc.minute())),
            false,
        ));
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        invalid("GACOS acquisition time must be RFC3339 or an explicit YYYY-MM-DD date")
    })?;
    let date_text = date.format("%Y-%m-%d").to_string();
    let time =
        gacos_user_supplied_time(supplied, &[scene_id, date_text.as_str(), "date_only", "*"])?;
    Ok((date_text, time, time.is_none()))
}

/// Merge only the supplied frozen scene evidence. A second alias of the same
/// stable scene cannot consume another request date or hide a time conflict.
fn merged_gacos_scene_times(
    scenes: &[Value],
    supplied: Option<&Map<String, Value>>,
) -> Result<(Vec<GacosSceneTime>, usize, usize), ApplicationServiceError> {
    let mut by_identity = BTreeMap::<&str, Vec<GacosSceneTime>>::new();
    for scene in scenes {
        let stable_id = scene
            .get("stable_scene_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| scene.get("scene_id").and_then(Value::as_str).map(str::trim))
            .ok_or_else(|| invalid("GACOS scene identity is missing"))?;
        by_identity
            .entry(stable_id)
            .or_default()
            .push(gacos_scene_time(scene, supplied)?);
    }
    let mut merged = Vec::new();
    let mut duplicates = 0;
    let mut conflicts = 0;
    for observations in by_identity.into_values() {
        duplicates += observations.len().saturating_sub(1);
        let dates = observations
            .iter()
            .map(|(date, _, _)| date)
            .collect::<BTreeSet<_>>();
        let exact_times = observations
            .iter()
            .filter_map(|(date, time, _)| time.map(|time| (date, time)))
            .collect::<BTreeSet<_>>();
        if dates.len() > 1 || exact_times.len() > 1 {
            conflicts += 1;
            continue;
        }
        if let Some(selected) = observations
            .iter()
            .find(|(_, time, _)| time.is_some())
            .or_else(|| observations.first())
        {
            merged.push(selected.clone());
        }
    }
    Ok((merged, duplicates, conflicts))
}

fn extension(value: &str) -> String {
    Path::new(value)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn path_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("boundary.geojson")
        .to_owned()
}

fn json_features(value: &Value) -> Vec<Value> {
    match value.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => value
            .get("features")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        Some("Feature") => vec![value.clone()],
        Some("Polygon") | Some("MultiPolygon") | Some("GeometryCollection") => vec![json!({
            "type": "Feature",
            "properties": {},
            "geometry": value,
        })],
        _ => Vec::new(),
    }
}

fn geometry_of(feature: &Value) -> Option<&Value> {
    if feature.get("type").and_then(Value::as_str) == Some("Feature") {
        feature.get("geometry")
    } else {
        Some(feature)
    }
}

fn polygonal_geometry(value: &Value) -> bool {
    match value.get("type").and_then(Value::as_str) {
        Some("Polygon") | Some("MultiPolygon") => true,
        Some("GeometryCollection") => value
            .get("geometries")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(polygonal_geometry)),
        _ => false,
    }
}

fn collect_numbers(value: &Value, numbers: &mut Vec<(f64, f64)>) {
    match value {
        Value::Array(items) => {
            if items.len() >= 2 && items[0].is_number() && items[1].is_number() {
                if let (Some(x), Some(y)) = (items[0].as_f64(), items[1].as_f64()) {
                    if x.is_finite() && y.is_finite() {
                        numbers.push((x, y));
                        return;
                    }
                }
            }
            for item in items {
                collect_numbers(item, numbers);
            }
        }
        Value::Object(object) => {
            for item in object.values() {
                collect_numbers(item, numbers);
            }
        }
        _ => {}
    }
}

fn bbox_for_geojson(value: &Value) -> Option<[f64; 4]> {
    let mut coordinates = Vec::new();
    collect_numbers(value, &mut coordinates);
    let mut iter = coordinates.into_iter();
    let (first_x, first_y) = iter.next()?;
    let mut west = first_x;
    let mut east = first_x;
    let mut south = first_y;
    let mut north = first_y;
    for (x, y) in iter {
        west = west.min(x);
        east = east.max(x);
        south = south.min(y);
        north = north.max(y);
    }
    if west < east && south < north {
        Some([west, south, east, north])
    } else {
        None
    }
}

fn bbox_value(bbox: [f64; 4]) -> Value {
    json!({
        "west": bbox[0],
        "east": bbox[2],
        "south": bbox[1],
        "north": bbox[3],
        "crs": "EPSG:4326",
    })
}

fn feature_name(feature: &Value, name_field: Option<&str>, index: usize) -> String {
    let properties = feature
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let field = name_field
        .filter(|value| !value.trim().is_empty())
        .or_else(|| properties.contains_key("name").then_some("name"))
        .or_else(|| properties.contains_key("名称").then_some("名称"));
    field
        .and_then(|key| properties.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("AOI 要素 {}", index + 1))
}

fn feature_collection(value: &Value) -> Result<(Value, Vec<Value>), ApplicationServiceError> {
    let features = json_features(value);
    if features.is_empty() {
        return Err(invalid(
            "AOI 只接受 Polygon、MultiPolygon 或 FeatureCollection",
        ));
    }
    if features.len() > MAX_AOI_FEATURES {
        return Err(invalid("AOI 要素数量超过应用边界"));
    }
    if features
        .iter()
        .any(|feature| geometry_of(feature).is_none_or(|geometry| !polygonal_geometry(geometry)))
    {
        return Err(invalid("AOI 中存在无效或非面几何"));
    }
    let normalized = json!({
        "type": "FeatureCollection",
        "features": features,
    });
    Ok((normalized.clone(), json_features(&normalized)))
}

fn native_preview(
    value: Value,
    source: &str,
    path: String,
    file_name: String,
) -> Result<Value, ApplicationServiceError> {
    let (geojson, features) = feature_collection(&value)?;
    let mut fields = BTreeSet::new();
    for feature in &features {
        if let Some(properties) = feature.get("properties").and_then(Value::as_object) {
            fields.extend(properties.keys().cloned());
        }
    }
    let display_field = ["name", "名称"]
        .into_iter()
        .find(|field| fields.contains(*field))
        .map(ToOwned::to_owned)
        .or_else(|| fields.iter().next().cloned());
    let preview_features = features
        .iter()
        .take(MAX_AOI_FEATURES)
        .enumerate()
        .map(|(index, feature)| {
            let properties = feature
                .get("properties")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            let bbox = geometry_of(feature).and_then(bbox_for_geojson);
            let feature_id = feature
                .get("id")
                .and_then(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .or_else(|| Some(value.to_string()))
                })
                .unwrap_or_else(|| index.to_string());
            json!({
                "id": feature_id,
                "index": index + 1,
                "source_index": index,
                "name": feature_name(feature, display_field.as_deref(), index),
                "area_km2": Value::Null,
                "bbox": bbox.map(bbox_value),
                "properties": properties,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "ok": true,
        "path": path,
        "file_name": file_name,
        "source_kind": source,
        "geojson": geojson,
        "total_features": features.len(),
        "feature_preview_limit": MAX_AOI_FEATURES,
        "features_truncated": features.len() > MAX_AOI_FEATURES,
        "fields": fields.into_iter().collect::<Vec<_>>(),
        "display_field": display_field,
        "features": preview_features,
    }))
}

fn decode_base64(value: &str) -> Result<Vec<u8>, ApplicationServiceError> {
    let bytes = super::BASE64_STANDARD
        .decode(value)
        .map_err(|_| invalid("AOI bytes 不是有效的 base64"))?;
    if bytes.len() > MAX_AOI_BYTES {
        return Err(invalid("AOI 文件超过应用边界"));
    }
    Ok(bytes)
}

fn read_path(path: &str) -> Result<Vec<u8>, ApplicationServiceError> {
    let metadata = std::fs::metadata(path).map_err(|_| invalid("AOI 文件不可读"))?;
    if !metadata.is_file() || metadata.len() as usize > MAX_AOI_BYTES {
        return Err(invalid("AOI 文件不存在或超过应用边界"));
    }
    std::fs::read(path).map_err(|_| invalid("AOI 文件读取失败"))
}

fn read_vector_sidecars(path: &str) -> Vec<(String, Vec<u8>)> {
    let source = Path::new(path);
    let Some(stem) = source.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let Some(parent) = source.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return ["dbf", "prj", "shx", "cpg"]
            .into_iter()
            .filter_map(|extension| {
                let sidecar = source.with_extension(extension);
                let bytes = std::fs::read(&sidecar).ok()?;
                Some((
                    sidecar
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_owned(),
                    bytes,
                ))
            })
            .collect();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut sidecars = Vec::new();
    for entry in entries.flatten() {
        let entry_path = entry.path();
        let Some(entry_stem) = entry_path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(entry_ext) = entry_path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        if !entry_stem.eq_ignore_ascii_case(stem)
            || !matches!(
                entry_ext.to_ascii_lowercase().as_str(),
                "dbf" | "prj" | "shx" | "cpg"
            )
        {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&entry_path) {
            if let Some(name) = entry_path.file_name().and_then(|v| v.to_str()) {
                sidecars.push((name.to_owned(), bytes));
            }
        }
    }
    sidecars
}

impl ApplicationService {
    /// Import the bounded AOI formats through the product-native codec boundary.
    /// GeoJSON remains on the existing Rust path; Shapefile/KML/KMZ are parsed
    /// by `insar-gis` and never delegated to the compatibility Worker.
    pub fn aoi_import(
        &self,
        request: &ApplicationAoiImportRequest,
    ) -> Result<Value, ApplicationServiceError> {
        let source = request.source.trim();
        if source.is_empty() {
            return Err(invalid("AOI import source is required"));
        }
        let (file_name, path, bytes, mut bundle) = match source {
            "path" => {
                let path = request
                    .path
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| invalid("AOI path is required"))?;
                let file_name = path_name(path);
                let bytes = read_path(path)?;
                let mut bundle = vec![(file_name.clone(), bytes.clone())];
                bundle.extend(read_vector_sidecars(path));
                (file_name, path.to_owned(), bytes, bundle)
            }
            "text" => {
                let file_name = request
                    .file_name
                    .clone()
                    .unwrap_or_else(|| "boundary.geojson".to_owned());
                let text = request
                    .text
                    .as_deref()
                    .ok_or_else(|| invalid("AOI text is required"))?;
                if text.len() > MAX_AOI_BYTES {
                    return Err(invalid("AOI text exceeds application bound"));
                }
                (
                    file_name.clone(),
                    file_name,
                    text.as_bytes().to_vec(),
                    Vec::new(),
                )
            }
            "bytes" => {
                let file_name = request
                    .file_name
                    .clone()
                    .unwrap_or_else(|| "boundary.geojson".to_owned());
                let bytes = decode_base64(request.base64_data.as_deref().unwrap_or_default())?;
                (file_name.clone(), file_name, bytes, Vec::new())
            }
            "bundle" => {
                let files = request
                    .files
                    .as_deref()
                    .ok_or_else(|| invalid("AOI bundle is required"))?;
                let mut decoded = Vec::with_capacity(files.len());
                for file in files {
                    let bytes = decode_base64(&file.base64)?;
                    decoded.push((file.name.clone(), bytes));
                }
                let file = decoded
                    .iter()
                    .find(|(name, _)| {
                        matches!(
                            extension(name).as_str(),
                            "geojson" | "json" | "shp" | "kml" | "kmz" | "zip"
                        )
                    })
                    .or_else(|| decoded.first())
                    .ok_or_else(|| invalid("AOI bundle is empty"))?;
                (file.0.clone(), file.0.clone(), file.1.clone(), decoded)
            }
            _ => return Err(invalid("AOI import source is unsupported")),
        };

        let extension = extension(&file_name);
        if matches!(extension.as_str(), "geojson" | "json" | "geojsonl") {
            let text = String::from_utf8(bytes).map_err(|_| invalid("AOI JSON 不是有效 UTF-8"))?;
            let value: Value =
                serde_json::from_str(&text).map_err(|_| invalid("AOI JSON 无法解析"))?;
            return native_preview(value, source, path, file_name);
        }
        if bundle.is_empty() {
            bundle.push((file_name.clone(), bytes.clone()));
        }
        let engine = insar_gis::LazyNativeGisEngine::discover_product();
        let mut preview = match engine.import_vector(&file_name, &bytes, &bundle) {
            Ok(preview) => preview,
            Err(err) if err.code() == insar_gis::NativeGisErrorCode::GisEngineUnavailable => {
                insar_gis::import_vector(&file_name, &bytes, &bundle)
                    .map_err(|error| invalid(error.code().as_str()))?
            }
            Err(err) => return Err(invalid(err.code().as_str())),
        };
        preview["path"] = Value::String(path);
        preview["source_kind"] = Value::String(source.to_owned());
        Ok(preview)
    }

    /// Normalize selected polygonal features in Rust before persistence.
    pub fn aoi_normalize(
        &self,
        request: &ApplicationAoiNormalizeRequest,
    ) -> Result<Value, ApplicationServiceError> {
        let (_, features) = feature_collection(&request.geojson)?;
        let selected = if request.feature_ids.is_empty() {
            features
        } else {
            features
                .into_iter()
                .enumerate()
                .filter(|(index, feature)| {
                    let id = feature
                        .get("id")
                        .and_then(|value| {
                            value
                                .as_str()
                                .map(ToOwned::to_owned)
                                .or_else(|| Some(value.to_string()))
                        })
                        .unwrap_or_else(|| index.to_string());
                    request
                        .feature_ids
                        .iter()
                        .any(|candidate| candidate == &id || candidate == &index.to_string())
                })
                .map(|(_, feature)| feature)
                .collect::<Vec<_>>()
        };
        if selected.is_empty() {
            return Err(invalid("AOI 未选择任何有效面要素"));
        }
        let download_mode = request
            .download_mode
            .as_deref()
            .unwrap_or("merge")
            .trim()
            .to_ascii_lowercase();
        if !matches!(download_mode.as_str(), "merge" | "split") {
            return Err(invalid("AOI download_mode 只支持 merge 或 split"));
        }
        let normalized = json!({"type": "FeatureCollection", "features": selected});
        let bbox = bbox_for_geojson(&normalized).ok_or_else(|| invalid("AOI 无法计算 bbox"))?;
        let geometry = geometry_for_binding(&normalized)?;
        Ok(json!({
            "ok": true,
            "geojson": normalized,
            "geometry": geometry,
            "bbox": bbox_value(bbox),
            "feature_count": normalized["features"].as_array().map_or(0, Vec::len),
            "download_mode": download_mode,
            "name_field": request.name_field,
        }))
    }

    /// Build a deterministic provider-free GACOS request plan.
    pub fn gacos_plan(&self, payload: &Value) -> Result<Value, ApplicationServiceError> {
        let object = payload
            .as_object()
            .ok_or_else(|| invalid("GACOS plan payload must be an object"))?;
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let request_id = object
            .get("request_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| invalid("GACOS request_id is required"))?;
        let aoi_geojson = object
            .get("aoi_geojson")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("GACOS exact aoi_geojson is required"))?;
        let aoi_name = object
            .get("aoi_name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| invalid("GACOS aoi_name is required"))?;
        let aoi_source = match object.get("aoi_source") {
            None => None,
            Some(value) => Some(
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| invalid("GACOS aoi_source is invalid"))?
                    .to_ascii_lowercase(),
            ),
        };
        if let Some(source) = aoi_source.as_deref() {
            if source != "frozen_import" {
                return Err(invalid("GACOS aoi_source is not supported"));
            }
        }
        let active = if aoi_source.is_some() {
            None
        } else {
            Some(
                self.active_operation_aoi()?
                    .ok_or_else(|| invalid("GACOS requires a current persisted AOI"))?,
            )
        };
        if let Some(active) = active.as_ref() {
            if &active.geometry != aoi_geojson || active.display_name != aoi_name {
                return Err(invalid(
                    "GACOS AOI geometry or display identity is stale relative to Store authority",
                ));
            }
        }
        let authoritative = if aoi_source.is_some() {
            super::validated_authoritative_polygonal_geojson(aoi_geojson)
                .ok_or_else(|| invalid("GACOS frozen AOI geometry is invalid"))?
        } else {
            let active = active
                .as_ref()
                .expect("active AOI exists when no frozen import source is supplied");
            super::validated_authoritative_polygonal_geojson(&active.geometry)
                .ok_or_else(|| invalid("persisted GACOS AOI geometry is invalid"))?
        };
        let envelope = super::wrapped_aoi_envelope(&authoritative.geometry)
            .ok_or_else(|| invalid("GACOS AOI cannot derive request windows"))?;
        super::validate_capability_windows(&envelope.request_windows)?;
        let processing_windows = envelope
            .request_windows
            .iter()
            .map(super::capability_window_value)
            .collect::<Vec<_>>();
        let source_kind = object
            .get("source_kind")
            .and_then(Value::as_str)
            .unwrap_or("online_selected")
            .trim()
            .to_ascii_lowercase();
        if !matches!(
            source_kind.as_str(),
            "online_selected" | "online_batch" | "local" | "mixed" | "manual_dates"
        ) {
            return Err(invalid("GACOS source_kind is not supported"));
        }
        let output_format = gacos_output_format(object)?;
        let manual_dates = if source_kind == "manual_dates" {
            Some(gacos_manual_dates(object.get("manual_dates"))?)
        } else {
            None
        };
        let manual_time = manual_dates
            .as_ref()
            .map(|_| gacos_manual_utc_time(object))
            .transpose()?;
        let selected_id_values = object
            .get("selected_scene_ids")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let selected_ids = selected_id_values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| {
                        invalid("GACOS selected_scene_ids must contain non-empty strings")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let selected_id_set = selected_ids.iter().cloned().collect::<BTreeSet<_>>();
        if selected_id_set.len() != selected_ids.len() {
            return Err(invalid("GACOS selected_scene_ids must be unique"));
        }
        if source_kind == "manual_dates" && !selected_ids.is_empty() {
            return Err(invalid(
                "GACOS manual_dates cannot be combined with selected_scene_ids",
            ));
        }
        let source_batch_id = object
            .get("source_batch_id")
            .map(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| invalid("GACOS source_batch_id is invalid"))
            })
            .transpose()?;
        if source_kind == "online_batch" && source_batch_id.is_none() {
            return Err(invalid("GACOS online_batch requires source_batch_id"));
        }
        if !matches!(source_kind.as_str(), "online_selected" | "online_batch")
            && source_batch_id.is_some()
        {
            return Err(invalid(
                "GACOS source_batch_id is only supported for Sentinel-1 sources",
            ));
        }
        let source_batch = if let Some(source_batch_id) = source_batch_id.as_deref() {
            let batch = self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let batch = store.load_search_batch(source_batch_id)?.ok_or_else(|| {
                    invalid("GACOS source_batch_id is not an immutable Store batch")
                })?;
                if batch.batch_id != source_batch_id {
                    return Err(invalid(
                        "GACOS source batch identity does not match source_batch_id",
                    ));
                }
                Ok(batch)
            })?;
            if !gacos_sentinel1_source_batch(&batch) {
                return Err(invalid(
                    "GACOS source batch is not a frozen Sentinel-1 search batch",
                ));
            }
            Some(batch)
        } else {
            None
        };
        let (scenes, source_scope_all, source_batch_candidate_count) = if let Some(source_batch) =
            source_batch.as_ref()
        {
            let source_scenes = gacos_scenes_from_source_batch(source_batch)?;
            let requested_scope = object
                .get("import_scope")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase);
            let scope = requested_scope
                .as_deref()
                .unwrap_or(if source_kind == "online_batch" {
                    "all"
                } else {
                    "selected"
                });
            if !matches!(scope, "all" | "selected") {
                return Err(invalid("GACOS import_scope must be all or selected"));
            }
            if scope == "all" && !selected_ids.is_empty() {
                return Err(invalid(
                    "GACOS all source-batch imports cannot include selected_scene_ids",
                ));
            }
            if scope == "selected" && selected_ids.is_empty() {
                return Err(invalid(
                    "GACOS selected source-batch imports require selected_scene_ids",
                ));
            }
            if scope == "all" {
                (source_scenes, true, Some(source_batch.candidates.len()))
            } else {
                let selected_scenes = source_scenes
                    .into_iter()
                    .filter(|scene| {
                        scene
                            .get("scene_id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| selected_id_set.contains(id))
                    })
                    .collect::<Vec<_>>();
                if selected_scenes.len() != selected_ids.len() {
                    return Err(invalid(
                        "GACOS selected_scene_ids are not all present in the frozen source batch",
                    ));
                }
                (selected_scenes, false, Some(source_batch.candidates.len()))
            }
        } else if manual_dates.is_some() {
            (Vec::new(), false, None)
        } else {
            (
                object
                    .get("scenes")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                false,
                None,
            )
        };
        let scene_ids = scenes
            .iter()
            .map(|scene| {
                scene
                    .get("scene_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| invalid("GACOS frozen scenes require stable scene_id values"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let scene_id_set = scene_ids.iter().cloned().collect::<BTreeSet<_>>();
        if scene_id_set.len() != scene_ids.len() {
            return Err(invalid("GACOS frozen scene IDs must be unique"));
        }
        let requires_exact_selected_snapshot = if source_batch.is_some() {
            !source_scope_all
        } else {
            source_kind == "online_selected"
        };
        if requires_exact_selected_snapshot
            && (selected_id_set.is_empty() || scene_id_set != selected_id_set)
        {
            return Err(invalid(
                "GACOS online_selected scenes must exactly match the non-empty selected_scene_ids snapshot",
            ));
        }
        let selected_scenes = if selected_id_set.is_empty() {
            scenes
        } else {
            scenes
                .into_iter()
                .filter(|scene| {
                    scene
                        .get("scene_id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| selected_id_set.contains(id))
                })
                .collect::<Vec<_>>()
        };
        if selected_scenes.is_empty() && manual_dates.is_none() {
            return Err(invalid("GACOS 规划至少需要一个冻结的已选场景"));
        }
        let supplied_times = object.get("user_supplied_times").and_then(Value::as_object);
        let (scene_times, duplicate_scene_count, time_conflict_count) =
            if let (Some(manual_dates), Some(time)) = (manual_dates.as_ref(), manual_time) {
                (
                    manual_dates
                        .iter()
                        .map(|date| (date.clone(), Some(time), false))
                        .collect::<Vec<_>>(),
                    0,
                    0,
                )
            } else {
                merged_gacos_scene_times(&selected_scenes, supplied_times)?
            };
        let date_only_count = scene_times
            .iter()
            .filter(|(_, _, date_only)| *date_only)
            .count();
        let mut grouped_dates = BTreeMap::<Option<(u32, u32)>, BTreeSet<String>>::new();
        for (date, time, _) in &scene_times {
            grouped_dates.entry(*time).or_default().insert(date.clone());
        }
        let dates = scene_times
            .iter()
            .map(|(date, _, _)| date.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let max_dates = object
            .get("max_dates_per_batch")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_GACOS_BATCH_DATES as u64)
            .clamp(1, MAX_GACOS_BATCH_DATES as u64) as usize;
        let buffer_degrees = object
            .get("buffer_degrees")
            .and_then(Value::as_f64)
            .unwrap_or(0.05);
        let request_windows = super::buffered_capability_windows(
            envelope.display_west,
            envelope.display_east,
            envelope.south,
            envelope.north,
            buffer_degrees,
        )?
        .iter()
        .map(super::capability_window_value)
        .collect::<Vec<_>>();
        let output_directory = object
            .get("output_dir")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let batch_specs = grouped_dates
            .iter()
            .flat_map(|(time, group_dates)| {
                group_dates
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .chunks(max_dates)
                    .map(|chunk| (*time, chunk.to_vec()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let batch_count = batch_specs.len();
        let batches = batch_specs
            .iter()
            .enumerate()
            .map(|(index, (time, chunk))| {
                // The provider form uses one compact UTC date per line. The
                // plan retains ISO dates separately for human inspection.
                let form_dates = chunk.iter().map(|date| date.replace('-', ""))
                    .collect::<Vec<_>>().join("\n");
                let requests = request_windows
                    .iter()
                    .enumerate()
                    .map(|(window_index, bbox)| {
                        json!({
                            "request_id": format!("{request_id}:batch:{}:window:{window_index}", index + 1),
                            "window_index": window_index,
                            "window_count": request_windows.len(),
                            "bbox": bbox,
                            "method": "POST",
                            "endpoint": "http://www.gacos.net/M/action_page.php",
                            "content_type": "application/x-www-form-urlencoded",
                            "form_fields": {
                                "N": bbox.get("north").cloned().unwrap_or(Value::String(String::new())),
                                "S": bbox.get("south").cloned().unwrap_or(Value::String(String::new())),
                                "W": bbox.get("west").cloned().unwrap_or(Value::String(String::new())),
                                "E": bbox.get("east").cloned().unwrap_or(Value::String(String::new())),
                                "H": time.map(|(hour, _)| hour.to_string()).unwrap_or_default(),
                                "M": time.map(|(_, minute)| minute.to_string()).unwrap_or_default(),
                                "date": form_dates,
                                "type": output_format.form_value(),
                                "output_format": output_format,
                                "seq": "OSM Map"
                            },
                            "required_sensitive_fields": ["email"]
                        })
                    })
                    .collect::<Vec<_>>();
                json!({
                    "batch_id": format!("gacos-batch-{}", index + 1),
                    "batch_index": index + 1,
                    "batch_count": batch_count,
                    "date_count": chunk.len(),
                    "dates": chunk,
                    "date_text": chunk.join(","),
                    "hour": time.map(|(hour, _)| hour),
                    "minute": time.map(|(_, minute)| minute),
                    "time_source": if time.is_some() { "source_or_user_utc" } else { "date_only_missing" },
                    "output_format": output_format,
                    "processing_windows": processing_windows,
                    "request_windows": request_windows,
                    "requests": requests
                })
            })
            .collect::<Vec<_>>();
        let source_scope = if source_batch.is_some() {
            if source_scope_all {
                "all"
            } else {
                "selected"
            }
        } else if manual_dates.is_some() {
            "manual_dates"
        } else {
            "inline_legacy"
        };
        let source_provenance = source_batch.as_ref().map(|batch| {
            json!({
                "kind": "frozen_sentinel1_search_batch",
                "batch_id": batch.batch_id,
                "provider_id": batch.provider_id,
                "schema_version": batch.schema_version,
                "candidate_count": batch.candidates.len(),
                "import_scope": source_scope,
            })
        });
        let mut plan = json!({
            "kind": "gacos_offline_plan",
            "provider": "gacos",
            "request_id": request_id,
            "aoi_name": aoi_name,
            "aoi_geojson": aoi_geojson,
            "aoi_source": aoi_source.as_deref().unwrap_or("active_store"),
            "execution_mode": "offline_plan",
            "processing_windows": processing_windows,
            "request_windows": request_windows,
            "buffer_degrees": buffer_degrees,
            "unique_dates": dates,
            "manual_dates": manual_dates.clone().unwrap_or_default(),
            "hour": manual_time.map(|(hour, _)| hour),
            "minute": manual_time.map(|(_, minute)| minute),
            "output_format": output_format,
            "source_kind": source_kind,
            "source_scope": source_scope,
            "source_provenance": source_provenance,
            "source_batch_id": source_batch_id,
            "source_batch_candidate_count": source_batch_candidate_count,
            "selected_scene_count": selected_scenes.len(),
            "recognized_scene_count": selected_scenes.len() - duplicate_scene_count,
            "date_only_count": date_only_count,
            "time_conflict_count": time_conflict_count,
            "duplicate_scene_count": duplicate_scene_count,
            "time_group_count": grouped_dates.len(),
            "batch_count": batches.len(),
            "batches": batches,
            "output_directory": output_directory,
            "requires_credentials": false,
            "manual_action_required": true,
            "offline": true
        });
        let mut issues = request_windows
            .iter()
            .enumerate()
            .filter_map(|(window_index, window)| {
                let width = window.get("east").and_then(Value::as_f64)?
                    - window.get("west").and_then(Value::as_f64)?;
                let height = window.get("north").and_then(Value::as_f64)?
                    - window.get("south").and_then(Value::as_f64)?;
                (width > MAX_GACOS_SPAN_DEGREES || height > MAX_GACOS_SPAN_DEGREES).then(|| {
                    json!({
                        "code": "GACOS_BBOX_TOO_LARGE",
                        "severity": "error",
                        "window_index": window_index,
                        "message": "GACOS request window exceeds the 10 degree service limit",
                        "details": {"east_west": width, "north_south": height}
                    })
                })
            })
            .collect::<Vec<_>>();
        if time_conflict_count > 0 {
            issues.push(json!({
                "code": "GACOS_TIME_CONFLICT",
                "severity": "error",
                "message": format!("{time_conflict_count} stable scene identity group(s) have conflicting acquisition times"),
                "details": {"time_conflict_count": time_conflict_count}
            }));
        }
        if date_only_count > 0 {
            issues.push(json!({
                "code": "GACOS_TIME_REQUIRED",
                "severity": "error",
                "message": format!("{date_only_count} selected scene/date group(s) require an explicit UTC hour and minute"),
                "details": {"date_only_count": date_only_count}
            }));
        }
        let request_count = batches
            .iter()
            .filter_map(|batch| batch.get("requests").and_then(Value::as_array))
            .map(Vec::len)
            .sum::<usize>();
        if issues.is_empty() && !output_directory.trim().is_empty() {
            let frozen_scene_ids = selected_scenes
                .iter()
                .filter_map(|scene| scene.get("scene_id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            self.freeze_gacos_plan(&mut plan, &frozen_scene_ids)?;
        }
        let report = json!({
            "valid": issues.is_empty(),
            "has_errors": !issues.is_empty(),
            "issues": issues,
            "estimated_minimum_seconds": request_count.saturating_sub(1) * 20,
            "offline": true
        });
        Ok(json!({"ok": true, "plan": plan, "report": report, "offline": true}))
    }

    /// Build a redacted form preview without receiving credentials.
    pub fn gacos_dry_run(&self, payload: &Value) -> Result<Value, ApplicationServiceError> {
        let submission = payload
            .get("submission")
            .unwrap_or(payload)
            .as_object()
            .ok_or_else(|| invalid("GACOS dry-run submission must be an object"))?;
        let provider = submission
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("gacos");
        let portal_url = submission
            .get("portal_url")
            .and_then(Value::as_str)
            .unwrap_or("http://www.gacos.net/");
        let submit_endpoint = submission
            .get("submit_endpoint")
            .and_then(Value::as_str)
            .unwrap_or("http://www.gacos.net/M/action_page.php");
        let batches = submission
            .get("batches")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let dry_batches = batches
            .iter()
            .flat_map(|batch| {
                let requests = batch
                    .get("requests")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_else(|| vec![batch.clone()]);
                requests.into_iter().map(move |request| {
                let fields = request.get("form_fields").and_then(Value::as_object).cloned().unwrap_or_default();
                let mut masked = Map::new();
                for (key, value) in fields {
                    let sensitive = ["email", "token", "password", "secret", "authorization"]
                        .iter()
                        .any(|part| key.to_ascii_lowercase().contains(part));
                    masked.insert(key, if sensitive { Value::String("<REDACTED>".to_owned()) } else { value });
                }
                let form_data = masked
                    .iter()
                    .map(|(key, value)| format!("-d \"{}={}\"", key, value.as_str().unwrap_or(&value.to_string())))
                    .collect::<Vec<_>>()
                    .join(" \\\n  ");
                json!({
                    "batch_id": batch.get("batch_id"),
                    "batch_index": batch.get("batch_index"),
                    "request_id": request.get("request_id"),
                    "window_index": request.get("window_index"),
                    "window_count": request.get("window_count"),
                    "bbox": request.get("bbox"),
                    "form_fields_masked": masked,
                    "form_fields_real": Value::Null,
                    "curl_preview_masked": format!("curl -X POST \"{}\" \\\n  -H \"Content-Type: application/x-www-form-urlencoded\" \\\n  {}", submit_endpoint, form_data),
                    "curl_preview_real": ""
                })
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "ok": true,
            "offline": true,
            "provider": provider,
            "portal_url": portal_url,
            "submit_endpoint": submit_endpoint,
            "batches": dry_batches
        }))
    }

    /// Generate a read-only report projection. File emission remains outside
    /// this migration; the returned paths are deterministic output targets.
    pub fn report_generate(&self, payload: &Value) -> Result<Value, ApplicationServiceError> {
        let object = payload
            .as_object()
            .ok_or_else(|| invalid("report payload must be an object"))?;
        let output_dir = object
            .get("output_dir")
            .and_then(Value::as_str)
            .unwrap_or("");
        let safe_name = object
            .get("region_safe_name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("current_region");
        let reports_dir = if output_dir.is_empty() {
            "07_reports".to_owned()
        } else {
            PathBuf::from(output_dir)
                .join(safe_name)
                .join("07_reports")
                .to_string_lossy()
                .to_string()
        };
        let report = json!({
            "title": format!("InSAR data preparation report: {safe_name}"),
            "created_at": chrono::Utc::now().to_rfc3339(),
            "summary": {"overall_status": "ready", "offline": true},
            "offline": true
        });
        let markdown = format!("# InSAR data preparation report: {safe_name}\n\nGenerated offline by ApplicationService.\n");
        Ok(json!({
            "ok": true,
            "report": report,
            "markdown": markdown,
            "reports_dir": reports_dir,
            "included": object.get("included").cloned().unwrap_or_else(|| json!([])),
            "paths": {
                "json": format!("{reports_dir}/{safe_name}_data_preparation_report.json"),
                "markdown": format!("{reports_dir}/{safe_name}_data_preparation_report.md"),
                "html": format!("{reports_dir}/{safe_name}_data_preparation_report.html"),
                "manifest": format!("{reports_dir}/{safe_name}_manifest.csv"),
                "warnings": format!("{reports_dir}/{safe_name}_warnings.csv")
            },
            "offline": true
        }))
    }
}

fn geometry_for_binding(value: &Value) -> Result<Value, ApplicationServiceError> {
    let features = value
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("normalized AOI is not a FeatureCollection"))?;
    if features.len() == 1 {
        return geometry_of(&features[0])
            .cloned()
            .ok_or_else(|| invalid("normalized AOI geometry is missing"));
    }
    let mut polygons = Vec::new();
    for feature in features {
        let geometry =
            geometry_of(feature).ok_or_else(|| invalid("normalized AOI geometry is missing"))?;
        match geometry.get("type").and_then(Value::as_str) {
            Some("Polygon") => {
                polygons.push(geometry.get("coordinates").cloned().unwrap_or(Value::Null))
            }
            Some("MultiPolygon") => {
                if let Some(items) = geometry.get("coordinates").and_then(Value::as_array) {
                    polygons.extend(items.iter().cloned());
                }
            }
            _ => return Err(invalid("normalized AOI contains unsupported geometry")),
        }
    }
    Ok(json!({"type": "MultiPolygon", "coordinates": polygons}))
}

#[cfg(test)]
mod tests {
    use super::{ApplicationAoiImportRequest, ApplicationAoiNormalizeRequest, ApplicationService};
    use crate::ApplicationServiceConfig;
    use insar_core::{FrozenSearchBatch, SnapshotItem};
    use serde_json::{json, Map};

    fn service() -> ApplicationService {
        ApplicationService::new(ApplicationServiceConfig::liveness_only())
    }

    fn geojson() -> serde_json::Value {
        json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "id": "wuhan",
                "properties": {"name": "武汉"},
                "geometry": {"type": "Polygon", "coordinates": [[[113.9,30.4],[114.5,30.4],[114.5,30.8],[113.9,30.4]]]}
            }]
        })
    }

    #[test]
    fn common_geojson_import_and_normalize_are_native() {
        let service = service();
        let imported = service
            .aoi_import(&ApplicationAoiImportRequest {
                source: "text".to_owned(),
                file_name: Some("aoi.geojson".to_owned()),
                text: Some(geojson().to_string()),
                ..Default::default()
            })
            .expect("native import");
        assert_eq!(imported["ok"], true);
        assert_eq!(imported["total_features"], 1);
        let normalized = service
            .aoi_normalize(&ApplicationAoiNormalizeRequest {
                geojson: imported["geojson"].clone(),
                feature_ids: vec!["wuhan".to_owned()],
                name_field: Some("name".to_owned()),
                download_mode: Some("merge".to_owned()),
            })
            .expect("native normalize");
        assert_eq!(normalized["feature_count"], 1);
        assert_eq!(normalized["bbox"]["crs"], "EPSG:4326");
        assert_eq!(normalized["geometry"]["type"], "Polygon");
        let invalid_mode = service.aoi_normalize(&ApplicationAoiNormalizeRequest {
            geojson: imported["geojson"].clone(),
            feature_ids: vec!["wuhan".to_owned()],
            download_mode: Some("unexpected".to_owned()),
            ..Default::default()
        });
        assert!(invalid_mode.is_err());
    }

    #[test]
    fn production_aoi_kml_import_is_owned_by_native_gis_without_a_compatibility_port() {
        let service = service();
        let imported = service
            .aoi_import(&ApplicationAoiImportRequest {
                source: "text".to_owned(),
                file_name: Some("boundary.kml".to_owned()),
                text: Some(
                    "<kml><Placemark><Polygon><outerBoundaryIs><LinearRing><coordinates>100,30 101,30 101,31 100,30</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark></kml>".to_owned(),
                ),
                ..Default::default()
            })
            .expect("native KML import must not require a Worker port");
        assert_eq!(imported["ok"], true);
        assert_eq!(imported["source_kind"], "text");
        assert_eq!(imported["total_features"], 1);
    }

    #[test]
    fn report_is_a_native_offline_route() {
        let service = service();
        let report = service
            .report_generate(&json!({"region_safe_name": "fixture"}))
            .expect("native report");
        assert_eq!(report["ok"], true);
        assert_eq!(report["report"]["offline"], true);
    }

    #[test]
    fn gacos_source_batch_requires_the_real_sentinel1_query_and_candidate_identity() {
        let make_batch =
            |provider: &str, query_dataset: &str, candidate_dataset: &str| FrozenSearchBatch {
                schema_version: 1,
                batch_id: "source-batch".to_owned(),
                provider_id: provider.to_owned(),
                created_at: "2026-09-06T00:00:00Z".to_owned(),
                aoi_name: "AOI".to_owned(),
                aoi_geojson: None,
                query: Map::from_iter([("dataset".to_owned(), json!(query_dataset))]),
                search_stats: None,
                candidates: vec![SnapshotItem {
                    item_id: "candidate-1".to_owned(),
                    provider_id: provider.to_owned(),
                    display_name: "candidate-1".to_owned(),
                    acquisition_datetime: Some("2024-01-02T03:04:00Z".to_owned()),
                    footprint_geojson: None,
                    metadata: Map::from_iter([
                        ("dataset".to_owned(), json!(candidate_dataset)),
                        ("scene_id".to_owned(), json!("S1-1")),
                    ]),
                    payload: Map::new(),
                    asset_access: None,
                    execution_policy: None,
                }],
            };
        let valid = make_batch("asf.catalog", "SENTINEL-1", "SENTINEL-1");
        assert!(super::gacos_sentinel1_source_batch(&valid));

        let mut wrong_query = valid.clone();
        wrong_query
            .query
            .insert("dataset".to_owned(), json!("ALOS-2"));
        assert!(!super::gacos_sentinel1_source_batch(&wrong_query));

        let wrong_candidate = make_batch("asf.catalog", "SENTINEL-1", "ALOS-2");
        assert!(!super::gacos_sentinel1_source_batch(&wrong_candidate));

        let wrong_provider = make_batch("asf.nasa.earthdata", "SENTINEL-1", "SENTINEL-1");
        assert!(!super::gacos_sentinel1_source_batch(&wrong_provider));
    }
}
