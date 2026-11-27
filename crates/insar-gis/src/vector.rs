use super::runtime::NativeGisRuntime;
use super::{NativeGisError, NativeGisErrorCode};
use flate2::read::DeflateDecoder;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::ffi::CString;
use std::io::Read;
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;

const MAX_AOI_BYTES: usize = 64 * 1024 * 1024;
const MAX_AOI_FEATURES: usize = 500;

pub fn import_vector(
    file_name: &str,
    bytes: &[u8],
    bundle: &[(String, Vec<u8>)],
) -> Result<Value, NativeGisError> {
    import_vector_with_runtime(None, file_name, bytes, bundle)
}

pub(crate) fn import_vector_with_runtime(
    runtime: Option<&NativeGisRuntime>,
    file_name: &str,
    bytes: &[u8],
    bundle: &[(String, Vec<u8>)],
) -> Result<Value, NativeGisError> {
    if bytes.len() > MAX_AOI_BYTES {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "AOI 文件超过应用边界",
        ));
    }
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let value = match extension.as_str() {
        "geojson" | "json" | "geojsonl" => parse_geojson(bytes)?,
        "kml" => parse_kml(bytes)?,
        "kmz" => parse_kmz(bytes)?,
        "shp" => {
            let shp = if bytes.is_empty() {
                bundle_bytes(bundle, ".shp").ok_or_else(|| {
                    error(
                        NativeGisErrorCode::AoiGeometryInvalid,
                        "Shapefile 主文件内容缺失",
                    )
                })?
            } else {
                bytes.to_vec()
            };
            let stem = Path::new(file_name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let dbf = bundle_bytes_for_stem(bundle, stem, ".dbf")
                .or_else(|| bundle_bytes(bundle, ".dbf"));
            let prj = bundle_bytes_for_stem(bundle, stem, ".prj")
                .or_else(|| bundle_bytes(bundle, ".prj"));
            let mut value = parse_shapefile(&shp, dbf.as_deref())?;
            validate_shapefile_crs(runtime, &mut value, prj.as_deref())?;
            value
        }
        "zip" => parse_zip_vector(runtime, bytes)?,
        _ => {
            return Err(error(
                NativeGisErrorCode::AoiFormatUnsupported,
                "AOI 只支持 GeoJSON、JSON、Shapefile (.shp / .zip)、KML 或 KMZ",
            ))
        }
    };
    build_preview(value, file_name)
}

fn error(code: NativeGisErrorCode, reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(code, reason)
}

fn bundle_bytes(bundle: &[(String, Vec<u8>)], suffix: &str) -> Option<Vec<u8>> {
    bundle.iter().find_map(|(name, bytes)| {
        (Path::new(name)
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case(suffix.trim_start_matches('.'))
            }))
        .then(|| bytes.clone())
    })
}

fn bundle_bytes_for_stem(
    bundle: &[(String, Vec<u8>)],
    stem: &str,
    suffix: &str,
) -> Option<Vec<u8>> {
    if stem.is_empty() {
        return None;
    }
    let ext = suffix.trim_start_matches('.');
    bundle.iter().find_map(|(name, bytes)| {
        let p = Path::new(name);
        let same_stem = p
            .file_stem()
            .and_then(|v| v.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case(stem));
        let same_ext = p
            .extension()
            .and_then(|v| v.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(ext));
        (same_stem && same_ext).then(|| bytes.clone())
    })
}

fn parse_geojson(bytes: &[u8]) -> Result<Value, NativeGisError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "AOI JSON 不是有效 UTF-8",
        )
    })?;
    let value: Value = serde_json::from_str(text)
        .map_err(|_| error(NativeGisErrorCode::AoiGeometryInvalid, "AOI JSON 无法解析"))?;
    let (_, features) = feature_collection(&value)?;
    Ok(json!({ "type": "FeatureCollection", "features": features }))
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
        Some("Polygon") => polygon_coordinates_valid(value.get("coordinates")),
        Some("MultiPolygon") => value
            .get("coordinates")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                !items.is_empty()
                    && items
                        .iter()
                        .all(|coordinates| polygon_coordinates_valid(Some(coordinates)))
            }),
        Some("GeometryCollection") => value
            .get("geometries")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(polygonal_geometry)),
        _ => false,
    }
}

fn polygon_coordinates_valid(value: Option<&Value>) -> bool {
    let Some(Value::Array(rings)) = value else {
        return false;
    };
    !rings.is_empty()
        && rings.iter().all(|ring| {
            let Some(points) = ring.as_array() else {
                return false;
            };
            points.len() >= 4
                && points.iter().all(|point| {
                    point.as_array().is_some_and(|point| {
                        point.len() >= 2
                            && finite_number(point[0].as_f64())
                            && finite_number(point[1].as_f64())
                    })
                })
        })
}

fn finite_number(value: Option<f64>) -> bool {
    value.is_some_and(f64::is_finite)
}

fn feature_collection(value: &Value) -> Result<(Value, Vec<Value>), NativeGisError> {
    let features = json_features(value);
    if features.is_empty() || features.len() > MAX_AOI_FEATURES {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "AOI 只接受有限数量的 Polygon、MultiPolygon 或 FeatureCollection",
        ));
    }
    if features
        .iter()
        .any(|feature| geometry_of(feature).is_none_or(|geometry| !polygonal_geometry(geometry)))
    {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "AOI 中存在无效或非面几何",
        ));
    }
    Ok((value.clone(), features))
}

fn collect_points(value: &Value, points: &mut Vec<(f64, f64)>) {
    match value {
        Value::Array(items) if items.len() >= 2 && items[0].is_number() && items[1].is_number() => {
            if let (Some(x), Some(y)) = (items[0].as_f64(), items[1].as_f64()) {
                if x.is_finite() && y.is_finite() {
                    points.push((x, y));
                } else {
                    items.iter().for_each(|item| collect_points(item, points));
                }
            } else {
                items.iter().for_each(|item| collect_points(item, points));
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_points(item, points)),
        Value::Object(items) => items.values().for_each(|item| collect_points(item, points)),
        _ => {}
    }
}

fn bbox_for_geometry(value: &Value) -> Option<[f64; 4]> {
    let mut points = Vec::new();
    collect_points(value, &mut points);
    let (first_x, first_y) = points.first().copied()?;
    let mut bbox = [first_x, first_y, first_x, first_y];
    for (x, y) in points.into_iter().skip(1) {
        bbox[0] = bbox[0].min(x);
        bbox[1] = bbox[1].min(y);
        bbox[2] = bbox[2].max(x);
        bbox[3] = bbox[3].max(y);
    }
    (bbox[0] < bbox[2] && bbox[1] < bbox[3]).then_some(bbox)
}

fn bbox_value(bbox: [f64; 4]) -> Value {
    json!({
        "west": bbox[0],
        "south": bbox[1],
        "east": bbox[2],
        "north": bbox[3],
        "crs": "EPSG:4326"
    })
}

fn feature_name(feature: &Value, display_field: Option<&str>, index: usize) -> String {
    feature
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| {
            display_field
                .and_then(|field| properties.get(field))
                .and_then(Value::as_str)
        })
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("AOI 要素 {}", index + 1))
}

fn build_preview(value: Value, file_name: &str) -> Result<Value, NativeGisError> {
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
        .enumerate()
        .map(|(index, feature)| {
            let geometry = geometry_of(feature).unwrap_or(&Value::Null);
            let feature_id = feature
                .get("id")
                .map(|id| id.as_str().map(ToOwned::to_owned).unwrap_or_else(|| id.to_string()))
                .unwrap_or_else(|| index.to_string());
            json!({
                "id": feature_id,
                "index": index + 1,
                "source_index": index,
                "name": feature_name(feature, display_field.as_deref(), index),
                "area_km2": Value::Null,
                "bbox": bbox_for_geometry(geometry).map(bbox_value),
                "properties": feature.get("properties").cloned().filter(Value::is_object).unwrap_or_else(|| json!({})),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "ok": true,
        "path": file_name,
        "file_name": file_name,
        "source_kind": "content",
        "geojson": geojson,
        "total_features": features.len(),
        "feature_preview_limit": MAX_AOI_FEATURES,
        "features_truncated": false,
        "fields": fields.into_iter().collect::<Vec<_>>(),
        "display_field": display_field,
        "features": preview_features,
    }))
}

fn validate_shapefile_crs(
    runtime: Option<&NativeGisRuntime>,
    value: &mut Value,
    prj: Option<&[u8]>,
) -> Result<(), NativeGisError> {
    let initial_coordinates_are_lonlat = coordinates_are_lonlat(value);
    let Some(prj) = prj else {
        if initial_coordinates_are_lonlat {
            return Ok(());
        }
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "Shapefile 坐标超出经纬度范围且缺少同名 .prj，无法安全转换",
        ));
    };
    let prj_text = String::from_utf8_lossy(prj).to_ascii_lowercase();
    let looks_wgs84 = prj_text.contains("wgs_1984")
        || prj_text.contains("wgs 84")
        || (prj_text.contains("epsg") && prj_text.contains("4326"));
    if !looks_wgs84 {
        let Some(runtime) = runtime else {
            return Err(error(
                NativeGisErrorCode::GisEngineUnavailable,
                "Shapefile .prj 需要产品自带 PROJ 坐标转换 runtime",
            ));
        };
        transform_shapefile_geometry(runtime, value, prj)?;
    }
    if coordinates_are_lonlat(value) {
        Ok(())
    } else {
        Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "Shapefile .prj 转换后的坐标超出经纬度范围",
        ))
    }
}

fn coordinates_are_lonlat(value: &Value) -> bool {
    value
        .get("features")
        .and_then(Value::as_array)
        .is_some_and(|features| {
            features.iter().all(|feature| {
                let mut points = Vec::new();
                geometry_of(feature)
                    .map(|geometry| collect_points(geometry, &mut points))
                    .is_some()
                    && !points.is_empty()
                    && points
                        .iter()
                        .all(|(x, y)| (-180.0..=180.0).contains(x) && (-90.0..=90.0).contains(y))
            })
        })
}

type ProjContext = *mut c_void;
type ProjObject = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct ProjCoord {
    x: f64,
    y: f64,
    z: f64,
    t: f64,
}

type ProjCreateCrsToCrs =
    unsafe extern "C" fn(ProjContext, *const c_char, *const c_char, *const c_void) -> ProjObject;
type ProjNormalizeForVisualization = unsafe extern "C" fn(ProjContext, ProjObject) -> ProjObject;
type ProjDestroy = unsafe extern "C" fn(ProjObject) -> ProjObject;
type ProjTrans = unsafe extern "C" fn(ProjObject, c_int, ProjCoord) -> ProjCoord;

const PJ_FWD: c_int = 1;

fn transform_shapefile_geometry(
    runtime: &NativeGisRuntime,
    value: &mut Value,
    prj: &[u8],
) -> Result<(), NativeGisError> {
    let source = CString::new(prj).map_err(|_| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "Shapefile .prj 包含无效字符串",
        )
    })?;
    let target = CString::new("EPSG:4326").expect("static EPSG target");
    transform_geojson_cstr_with_proj(runtime, value, &source, &target, "Shapefile .prj")
}

pub(crate) fn transform_geojson_value_with_proj(
    runtime: &NativeGisRuntime,
    value: &mut Value,
    src_crs: &str,
    dst_crs: &str,
) -> Result<(), NativeGisError> {
    let source = CString::new(src_crs.trim()).map_err(|_| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "源坐标系定义包含无效空字符",
        )
    })?;
    let target = CString::new(dst_crs.trim()).map_err(|_| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "目标坐标系定义包含无效空字符",
        )
    })?;
    transform_geojson_cstr_with_proj(runtime, value, &source, &target, "矢量几何")
}

fn transform_geojson_cstr_with_proj(
    runtime: &NativeGisRuntime,
    value: &mut Value,
    source: &std::ffi::CStr,
    target: &std::ffi::CStr,
    label: &str,
) -> Result<(), NativeGisError> {
    let proj_context = runtime.create_proj_context()?;
    unsafe {
        let create = runtime.proj_symbol::<ProjCreateCrsToCrs>(b"proj_create_crs_to_crs\0")?;
        let destroy = runtime.proj_symbol::<ProjDestroy>(b"proj_destroy\0")?;
        let operation = create(
            proj_context.raw(),
            source.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
        );
        if operation.is_null() {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                format!("{label}无法建立 PROJ 坐标转换"),
            ));
        }
        let normalize = runtime
            .proj_symbol::<ProjNormalizeForVisualization>(b"proj_normalize_for_visualization\0")?;
        let normalized = normalize(proj_context.raw(), operation);
        let transform = if normalized.is_null() {
            operation
        } else {
            destroy(operation);
            normalized
        };
        let trans = runtime.proj_symbol::<ProjTrans>(b"proj_trans\0")?;
        let res = transform_coordinate_arrays(value, transform, *trans);
        destroy(transform);
        res
    }
}

fn transform_coordinate_arrays(
    value: &mut Value,
    transform: ProjObject,
    trans: ProjTrans,
) -> Result<(), NativeGisError> {
    match value {
        Value::Array(items) if items.len() >= 2 && items[0].is_number() && items[1].is_number() => {
            let x = items[0].as_f64().ok_or_else(|| {
                error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "Shapefile X 坐标不是有限数值",
                )
            })?;
            let y = items[1].as_f64().ok_or_else(|| {
                error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "Shapefile Y 坐标不是有限数值",
                )
            })?;
            let z = items.get(2).and_then(Value::as_f64).unwrap_or(0.0);
            let output = unsafe { trans(transform, PJ_FWD, ProjCoord { x, y, z, t: 0.0 }) };
            if !output.x.is_finite() || !output.y.is_finite() {
                return Err(error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "PROJ 转换返回了无效坐标",
                ));
            }
            items[0] = json!(output.x);
            items[1] = json!(output.y);
        }
        Value::Array(items) => {
            for item in items {
                transform_coordinate_arrays(item, transform, trans)?;
            }
        }
        Value::Object(object) => {
            if let Some(coordinates) = object.get_mut("coordinates") {
                transform_coordinate_arrays(coordinates, transform, trans)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_kml(bytes: &[u8]) -> Result<Value, NativeGisError> {
    let mut reader = Reader::from_reader(bytes);
    let mut buffer = Vec::new();
    let mut polygon_depth = 0usize;
    let mut outer_depth = 0usize;
    let mut placemark_depth = 0usize;
    let mut capture_name = false;
    let mut name_text = String::new();
    let mut placemark_name = None;
    let mut coordinates = false;
    let mut text = String::new();
    let mut features = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match local_name(event.name().as_ref()) {
                b"Placemark" => {
                    placemark_depth += 1;
                    placemark_name = None;
                }
                b"name" if placemark_depth > 0 => {
                    capture_name = true;
                    name_text.clear();
                }
                b"Polygon" => polygon_depth += 1,
                b"outerBoundaryIs" if polygon_depth > 0 => outer_depth += 1,
                b"coordinates" if polygon_depth > 0 && outer_depth > 0 => {
                    coordinates = true;
                    text.clear();
                }
                _ => {}
            },
            Ok(Event::Text(event)) if coordinates || capture_name => {
                if coordinates {
                    text.push_str(&String::from_utf8_lossy(event.as_ref()));
                } else {
                    name_text.push_str(&String::from_utf8_lossy(event.as_ref()));
                }
            }
            Ok(Event::CData(event)) if coordinates || capture_name => {
                if coordinates {
                    text.push_str(&String::from_utf8_lossy(event.as_ref()));
                } else {
                    name_text.push_str(&String::from_utf8_lossy(event.as_ref()));
                }
            }
            Ok(Event::End(event)) => match local_name(event.name().as_ref()) {
                b"name" if capture_name => {
                    let value = name_text.trim();
                    if !value.is_empty() {
                        placemark_name = Some(value.to_owned());
                    }
                    capture_name = false;
                    name_text.clear();
                }
                b"coordinates" if coordinates => {
                    let ring = parse_kml_coordinates(&text)?;
                    let properties = placemark_name
                        .as_ref()
                        .map(|name| json!({"name": name}))
                        .unwrap_or_else(|| json!({}));
                    features.push(json!({
                        "type": "Feature",
                        "properties": properties,
                        "geometry": {"type": "Polygon", "coordinates": [ring]}
                    }));
                    coordinates = false;
                    text.clear();
                }
                b"outerBoundaryIs" if outer_depth > 0 => outer_depth -= 1,
                b"Polygon" if polygon_depth > 0 => polygon_depth -= 1,
                b"Placemark" if placemark_depth > 0 => placemark_depth -= 1,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => {
                return Err(error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "KML XML 无法解析",
                ))
            }
            _ => {}
        }
        buffer.clear();
    }
    if features.is_empty() {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "KML 不包含 Polygon 几何",
        ));
    }
    Ok(json!({ "type": "FeatureCollection", "features": features }))
}

fn local_name(value: &[u8]) -> &[u8] {
    value.rsplit(|byte| *byte == b':').next().unwrap_or(value)
}

fn parse_kml_coordinates(value: &str) -> Result<Vec<Value>, NativeGisError> {
    let mut ring = Vec::new();
    for item in value.split_whitespace() {
        let mut fields = item.split(',');
        let x = fields.next().and_then(|value| value.parse::<f64>().ok());
        let y = fields.next().and_then(|value| value.parse::<f64>().ok());
        let (Some(x), Some(y)) = (x, y) else {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "KML coordinates 无效",
            ));
        };
        if !x.is_finite() || !y.is_finite() {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "KML coordinates 不是有限数值",
            ));
        }
        ring.push(json!([x, y]));
    }
    if ring.len() < 4 {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "KML Polygon ring 点数不足",
        ));
    }
    if ring.first() != ring.last() {
        let first = ring.first().cloned().expect("ring has a first point");
        ring.push(first);
    }
    Ok(ring)
}

fn parse_kmz(bytes: &[u8]) -> Result<Value, NativeGisError> {
    let entries = read_zip_entries(bytes, "KMZ")?;
    let kml = entries
        .iter()
        .find(|(name, _)| {
            Path::new(name)
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v.eq_ignore_ascii_case("doc.kml"))
        })
        .or_else(|| {
            entries
                .iter()
                .find(|(name, _)| name.to_ascii_lowercase().ends_with(".kml"))
        })
        .map(|(_, data)| data.clone())
        .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "KMZ 不包含 KML"))?;
    parse_kml(&kml)
}

fn parse_zip_vector(
    runtime: Option<&NativeGisRuntime>,
    bytes: &[u8],
) -> Result<Value, NativeGisError> {
    let entries = read_zip_entries(bytes, "ZIP")?;
    if let Some((shp_name, shp_bytes)) = entries
        .iter()
        .find(|(name, _)| name.to_ascii_lowercase().ends_with(".shp"))
    {
        let stem = Path::new(shp_name)
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        let dbf = bundle_bytes_for_stem(&entries, stem, ".dbf")
            .or_else(|| bundle_bytes(&entries, ".dbf"));
        let prj = bundle_bytes_for_stem(&entries, stem, ".prj")
            .or_else(|| bundle_bytes(&entries, ".prj"));
        let mut value = parse_shapefile(shp_bytes, dbf.as_deref())?;
        validate_shapefile_crs(runtime, &mut value, prj.as_deref())?;
        return Ok(value);
    }
    if let Some((_, kml_bytes)) = entries
        .iter()
        .find(|(name, _)| {
            Path::new(name)
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v.eq_ignore_ascii_case("doc.kml"))
        })
        .or_else(|| {
            entries
                .iter()
                .find(|(name, _)| name.to_ascii_lowercase().ends_with(".kml"))
        })
    {
        return parse_kml(kml_bytes);
    }
    if let Some((_, gj_bytes)) = entries.iter().find(|(name, _)| {
        let lower = name.to_ascii_lowercase();
        lower.ends_with(".geojson") || lower.ends_with(".json") || lower.ends_with(".geojsonl")
    }) {
        return parse_geojson(gj_bytes);
    }
    Err(error(
        NativeGisErrorCode::AoiFormatUnsupported,
        "ZIP 压缩包内未包含有效的 Shapefile (.shp)、KML 或 GeoJSON 文件",
    ))
}

fn read_zip_entries(bytes: &[u8], label: &str) -> Result<Vec<(String, Vec<u8>)>, NativeGisError> {
    const EOCD: u32 = 0x0605_4b50;
    const CENTRAL: u32 = 0x0201_4b50;
    const LOCAL: u32 = 0x0403_4b50;
    const MAX_ZIP_ENTRIES: usize = 512;
    const MAX_TOTAL_UNCOMPRESSED: usize = MAX_AOI_BYTES * 2;

    let start = bytes.len().saturating_sub(65_557);
    let eocd = (start..bytes.len().saturating_sub(3))
        .rev()
        .find(|offset| read_u32(bytes, *offset) == Some(EOCD))
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::AoiGeometryInvalid,
                format!("{label} 不是有效 ZIP"),
            )
        })?;
    let count = (read_u16(bytes, eocd + 10).unwrap_or(0) as usize).min(MAX_ZIP_ENTRIES);
    let central_size = read_u32(bytes, eocd + 12).unwrap_or(0) as usize;
    let central_offset = read_u32(bytes, eocd + 16).unwrap_or(0) as usize;
    let central_end = central_offset.saturating_add(central_size);
    if central_end > bytes.len() {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            format!("{label} ZIP 目录损坏"),
        ));
    }
    let mut entries = Vec::new();
    let mut total_uncompressed = 0usize;
    let mut cursor = central_offset;
    for _ in 0..count {
        if read_u32(bytes, cursor) != Some(CENTRAL) || cursor + 46 > central_end {
            break;
        }
        let method = read_u16(bytes, cursor + 10).unwrap_or(u16::MAX);
        let compressed_size = read_u32(bytes, cursor + 20).unwrap_or(0) as usize;
        let uncompressed_size = read_u32(bytes, cursor + 24).unwrap_or(0) as usize;
        let name_len = read_u16(bytes, cursor + 28).unwrap_or(0) as usize;
        let extra_len = read_u16(bytes, cursor + 30).unwrap_or(0) as usize;
        let comment_len = read_u16(bytes, cursor + 32).unwrap_or(0) as usize;
        let local_offset = read_u32(bytes, cursor + 42).unwrap_or(0) as usize;
        let name_start = cursor + 46;
        let name_end = name_start.saturating_add(name_len);
        let next = name_end
            .saturating_add(extra_len)
            .saturating_add(comment_len);
        if next > central_end {
            break;
        }
        let name = String::from_utf8_lossy(&bytes[name_start..name_end]).to_string();
        cursor = next;

        let base_name = Path::new(&name)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("");
        if name.ends_with('/') || name.starts_with("__MACOSX/") || base_name.starts_with("._") {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let wanted = lower.ends_with(".shp")
            || lower.ends_with(".dbf")
            || lower.ends_with(".prj")
            || lower.ends_with(".cpg")
            || lower.ends_with(".kml")
            || lower.ends_with(".geojson")
            || lower.ends_with(".json");
        if !wanted {
            continue;
        }
        if uncompressed_size > MAX_AOI_BYTES
            || total_uncompressed.saturating_add(uncompressed_size) > MAX_TOTAL_UNCOMPRESSED
            || read_u32(bytes, local_offset) != Some(LOCAL)
        {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                format!("{label} 矢量条目大小超限或文件损坏"),
            ));
        }
        let local_name_len = read_u16(bytes, local_offset + 26).unwrap_or(0) as usize;
        let local_extra_len = read_u16(bytes, local_offset + 28).unwrap_or(0) as usize;
        let data_start = local_offset
            .saturating_add(30)
            .saturating_add(local_name_len)
            .saturating_add(local_extra_len);
        let data_end = data_start.saturating_add(compressed_size);
        if data_end > bytes.len() {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                format!("{label} 条目数据损坏"),
            ));
        }
        let compressed = &bytes[data_start..data_end];
        let mut output = Vec::with_capacity(uncompressed_size);
        match method {
            0 => output.extend_from_slice(compressed),
            8 => {
                DeflateDecoder::new(compressed)
                    .read_to_end(&mut output)
                    .map_err(|_| {
                        error(
                            NativeGisErrorCode::AoiGeometryInvalid,
                            format!("{label} 解压失败"),
                        )
                    })?;
            }
            _ => {
                return Err(error(
                    NativeGisErrorCode::AoiFormatUnsupported,
                    format!("{label} 使用了不支持的压缩方法"),
                ))
            }
        }
        total_uncompressed = total_uncompressed.saturating_add(output.len());
        entries.push((name, output));
    }
    Ok(entries)
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes
        .get(offset..offset + 2)
        .map(|value| u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)
        .map(|value| u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_i32_le(bytes: &[u8], offset: usize) -> Option<i32> {
    read_u32(bytes, offset).map(|value| value as i32)
}

fn read_f64_le(bytes: &[u8], offset: usize) -> Option<f64> {
    bytes
        .get(offset..offset + 8)
        .and_then(|value| value.try_into().ok())
        .map(f64::from_le_bytes)
}

fn parse_shapefile(shp: &[u8], dbf: Option<&[u8]>) -> Result<Value, NativeGisError> {
    let magic = shp
        .get(0..4)
        .and_then(|value| value.try_into().ok())
        .map(i32::from_be_bytes);
    if shp.len() < 100 || magic != Some(9994) {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "Shapefile 主文件头无效",
        ));
    }
    let shape_type = read_i32_le(shp, 32).unwrap_or(-1);
    if !matches!(shape_type, 0 | 5 | 15 | 25) {
        return Err(error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "Shapefile 不是 Polygon 类型",
        ));
    }
    let properties = dbf.map(parse_dbf).transpose()?.unwrap_or_default();
    let mut features = Vec::new();
    let mut cursor = 100usize;
    while cursor + 8 <= shp.len() {
        let content_words = shp
            .get(cursor + 4..cursor + 8)
            .and_then(|value| value.try_into().ok())
            .map(u32::from_be_bytes)
            .unwrap_or(0) as usize;
        let start = cursor + 8;
        let end = start.saturating_add(content_words.saturating_mul(2));
        if end > shp.len() {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "Shapefile 记录被截断",
            ));
        }
        let content = &shp[start..end];
        let record_type = read_i32_le(content, 0).unwrap_or(-1);
        if record_type != 0 {
            if !matches!(record_type, 5 | 15 | 25) {
                return Err(error(
                    NativeGisErrorCode::AoiFormatUnsupported,
                    "Shapefile 记录不是 Polygon 类型",
                ));
            }
            let geometry = parse_polygon_record(content)?;
            let property = properties.get(features.len()).cloned().unwrap_or_default();
            features.push(json!({
                "type": "Feature",
                "id": features.len().to_string(),
                "properties": property,
                "geometry": geometry,
            }));
        }
        cursor = end;
    }
    if features.is_empty() {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "Shapefile 不包含可用 Polygon",
        ));
    }
    Ok(json!({ "type": "FeatureCollection", "features": features }))
}

fn parse_polygon_record(content: &[u8]) -> Result<Value, NativeGisError> {
    let parts = read_i32_le(content, 36).unwrap_or(-1);
    let points = read_i32_le(content, 40).unwrap_or(-1);
    if parts <= 0 || points < 4 {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "Shapefile Polygon 结构无效",
        ));
    }
    let parts = parts as usize;
    let points = points as usize;
    let parts_start = 44usize;
    let points_start = parts_start.saturating_add(parts.saturating_mul(4));
    if points_start.saturating_add(points.saturating_mul(16)) > content.len() {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "Shapefile Polygon 坐标被截断",
        ));
    }
    let mut rings = Vec::new();
    let mut starts = Vec::with_capacity(parts);
    for index in 0..parts {
        starts.push(read_i32_le(content, parts_start + index * 4).unwrap_or(-1));
    }
    for index in 0..parts {
        let start = starts[index];
        let end = starts.get(index + 1).copied().unwrap_or(points as i32);
        if start < 0 || end <= start || end as usize > points {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "Shapefile Polygon ring 范围无效",
            ));
        }
        let mut ring = Vec::new();
        for point_index in start as usize..end as usize {
            let offset = points_start + point_index * 16;
            let x = read_f64_le(content, offset).ok_or_else(|| {
                error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "Shapefile X 坐标无效",
                )
            })?;
            let y = read_f64_le(content, offset + 8).ok_or_else(|| {
                error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "Shapefile Y 坐标无效",
                )
            })?;
            if !x.is_finite() || !y.is_finite() {
                return Err(error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "Shapefile 坐标不是有限数值",
                ));
            }
            ring.push(json!([x, y]));
        }
        if ring.len() < 4 {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "Shapefile Polygon ring 点数不足",
            ));
        }
        if ring.first() != ring.last() {
            ring.push(ring.first().cloned().expect("ring has a first point"));
        }
        rings.push(ring);
    }
    let geometry_type = if rings.len() == 1 {
        "Polygon"
    } else {
        "MultiPolygon"
    };
    let coordinates = if geometry_type == "Polygon" {
        json!(rings)
    } else {
        Value::Array(rings.into_iter().map(|ring| json!([ring])).collect())
    };
    Ok(json!({ "type": geometry_type, "coordinates": coordinates }))
}

fn parse_dbf(bytes: &[u8]) -> Result<Vec<Map<String, Value>>, NativeGisError> {
    if bytes.len() < 32 {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "DBF 文件头无效",
        ));
    }
    let header_len = read_u16(bytes, 8).unwrap_or(0) as usize;
    let record_len = read_u16(bytes, 10).unwrap_or(0) as usize;
    if header_len < 33 || record_len < 1 || header_len > bytes.len() {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "DBF 字段布局无效",
        ));
    }
    let mut fields = Vec::new();
    let mut cursor = 32usize;
    while cursor + 32 <= header_len && bytes[cursor] != 0x0d {
        let name_end = bytes[cursor..cursor + 11]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(11);
        let name = String::from_utf8_lossy(&bytes[cursor..cursor + name_end])
            .trim()
            .to_owned();
        let kind = bytes[cursor + 11] as char;
        let length = bytes[cursor + 16] as usize;
        fields.push((name, kind, length));
        cursor += 32;
    }
    let mut records = Vec::new();
    let mut offset = header_len;
    while offset + record_len <= bytes.len() && bytes[offset] != 0x1a {
        if bytes[offset] != b'*' {
            let mut object = Map::new();
            let mut field_offset = offset + 1;
            for (name, kind, length) in &fields {
                let end = field_offset.saturating_add(*length);
                if end > offset + record_len {
                    break;
                }
                let raw = &bytes[field_offset..end];
                object.insert(name.clone(), dbf_value(*kind, raw));
                field_offset = end;
            }
            records.push(object);
        }
        offset += record_len;
    }
    Ok(records)
}

fn dbf_value(kind: char, raw: &[u8]) -> Value {
    let text = String::from_utf8_lossy(raw)
        .trim_matches(|character: char| character.is_whitespace() || character == '\0')
        .to_owned();
    if text.is_empty() {
        return Value::Null;
    }
    match kind {
        'N' | 'F' => text
            .parse::<f64>()
            .map(|value| json!(value))
            .unwrap_or_else(|_| Value::String(text)),
        'L' => Value::Bool(matches!(text.to_ascii_uppercase().as_str(), "Y" | "T")),
        _ => Value::String(text),
    }
}
