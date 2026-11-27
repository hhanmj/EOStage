use super::mapgis::{
    write_shapefile_bundle, MapGisShapeKind, ParsedMapGisDataset, ParsedMapGisFeature,
};
use super::runtime::NativeGisRuntime;
use super::vector::{import_vector_with_runtime, transform_geojson_value_with_proj};
use super::{NativeGisError, NativeGisErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static VECTOR_STAGING_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VectorProcessRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub operation: String,
    #[serde(default)]
    pub distance_m: Option<f64>,
    #[serde(default)]
    pub quad_segs: Option<i32>,
}

type OgrGeometry = *mut c_void;
type OgrGCreateGeometryFromJson = unsafe extern "C" fn(*const c_char) -> OgrGeometry;
type OgrGBuffer = unsafe extern "C" fn(OgrGeometry, f64, c_int) -> OgrGeometry;
type OgrGExportToJson = unsafe extern "C" fn(OgrGeometry) -> *mut c_char;
type OgrGDestroyGeometry = unsafe extern "C" fn(OgrGeometry);
type VsiFree = unsafe extern "C" fn(*mut c_void);

struct OgrGeometryGuard {
    handle: OgrGeometry,
    destroy: OgrGDestroyGeometry,
}

impl OgrGeometryGuard {
    fn new(handle: OgrGeometry, destroy: OgrGDestroyGeometry) -> Self {
        Self { handle, destroy }
    }

    fn raw(&self) -> OgrGeometry {
        self.handle
    }
}

impl Drop for OgrGeometryGuard {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.destroy)(self.handle) };
        }
    }
}

struct VsiStringGuard {
    ptr: *mut c_char,
    free: VsiFree,
}

impl Drop for VsiStringGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { (self.free)(self.ptr.cast()) };
        }
    }
}

fn error(code: NativeGisErrorCode, reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(code, reason)
}

fn load_input_feature_collection(
    runtime: &NativeGisRuntime,
    input_path: &Path,
) -> Result<Vec<Value>, NativeGisError> {
    if !input_path.is_file() {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            format!("输入矢量文件不存在: {}", input_path.display()),
        ));
    }
    let ext = input_path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if matches!(ext.as_str(), "geojson" | "json" | "geojsonl") {
        let bytes = fs::read(input_path).map_err(|_| {
            error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "无法读取输入 GeoJSON 文件",
            )
        })?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "GeoJSON 文件不是有效 UTF-8",
            )
        })?;
        let val: Value = serde_json::from_str(text)
            .map_err(|_| error(NativeGisErrorCode::AoiGeometryInvalid, "GeoJSON 解析失败"))?;
        return extract_features_any_geometry(&val);
    }

    if !matches!(ext.as_str(), "shp" | "kml" | "kmz") {
        return Err(error(
            NativeGisErrorCode::AoiFormatUnsupported,
            format!("不支持的矢量输入格式: .{ext}"),
        ));
    }

    let bytes = fs::read(input_path).map_err(|_| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "无法读取输入矢量文件",
        )
    })?;
    let mut bundle: Vec<(String, Vec<u8>)> = Vec::new();
    if ext == "shp" {
        for companion_ext in ["shx", "dbf", "prj", "cpg"] {
            let p = input_path.with_extension(companion_ext);
            if p.is_file() {
                if let (Some(name), Ok(data)) =
                    (p.file_name().and_then(|n| n.to_str()), fs::read(&p))
                {
                    bundle.push((name.to_owned(), data));
                }
            }
        }
    }
    let file_name = input_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vector.geojson");
    let imported = import_vector_with_runtime(Some(runtime), file_name, &bytes, &bundle)?;
    let geojson = imported.get("geojson").ok_or_else(|| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "矢量导入结果缺少 GeoJSON 数据",
        )
    })?;
    extract_features_any_geometry(geojson)
}

fn extract_features_any_geometry(val: &Value) -> Result<Vec<Value>, NativeGisError> {
    let raw_features = match val.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => val
            .get("features")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        Some("Feature") => vec![val.clone()],
        Some(
            "Point" | "MultiPoint" | "LineString" | "MultiLineString" | "Polygon" | "MultiPolygon"
            | "GeometryCollection",
        ) => vec![json!({
            "type": "Feature",
            "properties": {},
            "geometry": val,
        })],
        _ => {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "输入 GeoJSON 必须是 FeatureCollection、Feature 或标准几何对象",
            ))
        }
    };
    let mut normalized = Vec::with_capacity(raw_features.len());
    for feat in raw_features {
        let Some(geom) = feat.get("geometry").filter(|g| g.is_object()) else {
            continue;
        };
        let props = feat
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        normalized.push(json!({
            "type": "Feature",
            "properties": props,
            "geometry": geom,
        }));
    }
    if normalized.is_empty() {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "输入矢量不包含有效几何要素",
        ));
    }
    Ok(normalized)
}

fn accumulate_bbox(value: &Value, bbox: &mut [f64; 4]) -> Result<(), NativeGisError> {
    match value {
        Value::Array(items) if items.len() >= 2 && items[0].is_number() && items[1].is_number() => {
            let x = items[0]
                .as_f64()
                .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "几何 X 坐标无效"))?;
            let y = items[1]
                .as_f64()
                .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "几何 Y 坐标无效"))?;
            if !x.is_finite() || !y.is_finite() {
                return Err(error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "几何包含非有限坐标",
                ));
            }
            bbox[0] = bbox[0].min(x);
            bbox[1] = bbox[1].min(y);
            bbox[2] = bbox[2].max(x);
            bbox[3] = bbox[3].max(y);
        }
        Value::Array(items) => {
            for item in items {
                accumulate_bbox(item, bbox)?;
            }
        }
        Value::Object(obj) => {
            if let Some(coords) = obj.get("coordinates") {
                accumulate_bbox(coords, bbox)?;
            }
            if let Some(geoms) = obj.get("geometries") {
                accumulate_bbox(geoms, bbox)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn geometry_bbox(geom: &Value) -> Result<[f64; 4], NativeGisError> {
    let mut bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    accumulate_bbox(geom, &mut bbox)?;
    if !bbox.iter().all(|v| v.is_finite()) {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "几何坐标为空，无法计算外包框",
        ));
    }
    Ok(bbox)
}

fn ensure_closed_polygon_rings(geom: &mut Value) -> Result<(), NativeGisError> {
    let kind = geom
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let Some(coords) = geom.get_mut("coordinates") else {
        return Ok(());
    };
    let close_ring = |ring: &mut Vec<Value>| -> Result<(), NativeGisError> {
        if ring.len() < 3 {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "缓冲区输出多边形环顶点数不足",
            ));
        }
        if ring.first() != ring.last() {
            ring.push(ring[0].clone());
        }
        if ring.len() < 4 {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "缓冲区输出多边形未能有效闭合",
            ));
        }
        Ok(())
    };
    match kind.as_str() {
        "Polygon" => {
            if let Some(rings) = coords.as_array_mut() {
                for ring in rings {
                    if let Some(pts) = ring.as_array_mut() {
                        close_ring(pts)?;
                    }
                }
            }
        }
        "MultiPolygon" => {
            if let Some(polys) = coords.as_array_mut() {
                for poly in polys {
                    if let Some(rings) = poly.as_array_mut() {
                        for ring in rings {
                            if let Some(pts) = ring.as_array_mut() {
                                close_ring(pts)?;
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn buffer_single_geometry(
    runtime: &NativeGisRuntime,
    geom: &Value,
    distance_m: f64,
    quad_segs: c_int,
) -> Result<Value, NativeGisError> {
    let bbox = geometry_bbox(geom)?;
    let lon0 = 0.5 * (bbox[0] + bbox[2]);
    let lat0 = 0.5 * (bbox[1] + bbox[3]);
    if !(-180.0..=180.0).contains(&lon0) || !(-90.0..=90.0).contains(&lat0) {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "缓冲区输入矢量必须为 WGS84 (EPSG:4326) 经纬度坐标",
        ));
    }
    let aeqd =
        format!("+proj=aeqd +lat_0={lat0:.8} +lon_0={lon0:.8} +datum=WGS84 +units=m +no_defs");

    let mut local_geom = geom.clone();
    transform_geojson_value_with_proj(runtime, &mut local_geom, "EPSG:4326", &aeqd)?;

    let local_json = serde_json::to_string(&local_geom).map_err(|_| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "序列化投影矢量几何失败",
        )
    })?;
    let local_c = CString::new(local_json).map_err(|_| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "矢量几何 JSON 包含无效空字符",
        )
    })?;

    let mut buffered = unsafe {
        let create_from_json = *runtime
            .gdal_symbol::<OgrGCreateGeometryFromJson>(b"OGR_G_CreateGeometryFromJson\0")?;
        let ogr_buffer = *runtime.gdal_symbol::<OgrGBuffer>(b"OGR_G_Buffer\0")?;
        let export_json = *runtime.gdal_symbol::<OgrGExportToJson>(b"OGR_G_ExportToJson\0")?;
        let destroy_geom =
            *runtime.gdal_symbol::<OgrGDestroyGeometry>(b"OGR_G_DestroyGeometry\0")?;
        let vsi_free = *runtime.gdal_symbol::<VsiFree>(b"VSIFree\0")?;

        let raw_geom = create_from_json(local_c.as_ptr());
        if raw_geom.is_null() {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "OGR 无法解析矢量几何对象",
            ));
        }
        let geom_guard = OgrGeometryGuard::new(raw_geom, destroy_geom);
        let raw_buf = ogr_buffer(geom_guard.raw(), distance_m, quad_segs);
        if raw_buf.is_null() {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "OGR 缓冲区计算返回空几何",
            ));
        }
        let buf_guard = OgrGeometryGuard::new(raw_buf, destroy_geom);
        let json_ptr = export_json(buf_guard.raw());
        if json_ptr.is_null() {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "OGR 导出缓冲区 GeoJSON 失败",
            ));
        }
        let _str_guard = VsiStringGuard {
            ptr: json_ptr,
            free: vsi_free,
        };
        let exported = CStr::from_ptr(json_ptr).to_str().map_err(|_| {
            error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "OGR 导出 GeoJSON 不是有效 UTF-8",
            )
        })?;
        serde_json::from_str::<Value>(exported).map_err(|_| {
            error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "解析 OGR 缓冲区 GeoJSON 失败",
            )
        })?
    };

    transform_geojson_value_with_proj(runtime, &mut buffered, &aeqd, "EPSG:4326")?;
    ensure_closed_polygon_rings(&mut buffered)?;
    Ok(buffered)
}

fn parse_xy_point(item: &Value, bbox: &mut [f64; 4]) -> Result<[f64; 2], NativeGisError> {
    let arr = item
        .as_array()
        .filter(|a| a.len() >= 2)
        .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "几何坐标点格式无效"))?;
    let x = arr[0]
        .as_f64()
        .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "坐标 X 非有限数"))?;
    let y = arr[1]
        .as_f64()
        .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "坐标 Y 非有限数"))?;
    bbox[0] = bbox[0].min(x);
    bbox[1] = bbox[1].min(y);
    bbox[2] = bbox[2].max(x);
    bbox[3] = bbox[3].max(y);
    Ok([x, y])
}

fn parse_xy_ring(ring: &Value, bbox: &mut [f64; 4]) -> Result<Vec<[f64; 2]>, NativeGisError> {
    ring.as_array()
        .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "面环格式无效"))?
        .iter()
        .map(|item| parse_xy_point(item, bbox))
        .collect()
}

fn feature_collection_to_mapgis(features: &[Value]) -> Result<ParsedMapGisDataset, NativeGisError> {
    let mut parsed_features: Vec<ParsedMapGisFeature> = Vec::new();
    let mut kind: Option<MapGisShapeKind> = None;
    let mut bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];

    for feat in features.iter() {
        let geom = feat
            .get("geometry")
            .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "要素缺少几何"))?;
        let props = feat
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let gtype = geom.get("type").and_then(Value::as_str).unwrap_or_default();
        let coords_val = geom.get("coordinates").ok_or_else(|| {
            error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "几何缺少 coordinates",
            )
        })?;

        let (feat_kind, parts): (MapGisShapeKind, Vec<Vec<Vec<[f64; 2]>>>) = match gtype {
            "Point" => {
                let pt = parse_xy_point(coords_val, &mut bbox)?;
                (MapGisShapeKind::Point, vec![vec![vec![pt]]])
            }
            "LineString" => {
                let ring = parse_xy_ring(coords_val, &mut bbox)?;
                (MapGisShapeKind::Line, vec![vec![ring]])
            }
            "Polygon" => {
                let rings = coords_val
                    .as_array()
                    .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "面环无效"))?
                    .iter()
                    .map(|ring| parse_xy_ring(ring, &mut bbox))
                    .collect::<Result<Vec<_>, _>>()?;
                (MapGisShapeKind::Polygon, vec![rings])
            }
            "MultiPolygon" => {
                let polys = coords_val
                    .as_array()
                    .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "多面格式无效"))?
                    .iter()
                    .map(|poly| {
                        poly.as_array()
                            .ok_or_else(|| {
                                error(NativeGisErrorCode::AoiGeometryInvalid, "面环格式无效")
                            })?
                            .iter()
                            .map(|ring| parse_xy_ring(ring, &mut bbox))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                (MapGisShapeKind::Polygon, polys)
            }
            other => {
                return Err(error(
                    NativeGisErrorCode::AoiFormatUnsupported,
                    format!("Shapefile 导出暂不支持几何类型 '{other}'"),
                ))
            }
        };

        if let Some(prev) = kind {
            if prev != feat_kind {
                return Err(error(
                    NativeGisErrorCode::AoiFormatUnsupported,
                    "Shapefile 要求所有要素保持单一几何类型 (Point / LineString / Polygon)",
                ));
            }
        } else {
            kind = Some(feat_kind);
        }

        let mut dbf_props: Map<String, Value> = Map::new();
        if let Some(name) = props.get("NAME").or_else(|| props.get("name")) {
            dbf_props.insert("NAME".to_owned(), name.clone());
        }
        for part in parts {
            parsed_features.push(ParsedMapGisFeature {
                id: parsed_features.len() + 1,
                coords: part,
                properties: dbf_props.clone(),
            });
        }
    }

    Ok(ParsedMapGisDataset {
        kind: kind.ok_or_else(|| {
            error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "未找到可写入的矢量要素",
            )
        })?,
        format_variant: "GeoJSON".to_owned(),
        crs_proj4: Some("+proj=longlat +datum=WGS84 +no_defs".to_owned()),
        crs_wkt: None,
        coordinate_scale: 1.0,
        fields: Vec::new(),
        features: parsed_features,
        bbox: bbox.iter().all(|v| v.is_finite()).then_some(bbox),
    })
}

pub(crate) fn process_vector(
    runtime: &NativeGisRuntime,
    request: &VectorProcessRequest,
) -> Result<Value, NativeGisError> {
    let op = request.operation.trim().to_ascii_lowercase();
    if !matches!(op.as_str(), "buffer" | "convert") {
        return Err(error(
            NativeGisErrorCode::AoiFormatUnsupported,
            format!("不支持的矢量处理算子: {op} (支持 buffer / convert)"),
        ));
    }

    let features = load_input_feature_collection(runtime, &request.input_path)?;
    let out_features = if op == "buffer" {
        let distance_m = request.distance_m.unwrap_or(0.0);
        if !distance_m.is_finite() || distance_m <= 0.0 || distance_m > 1_000_000.0 {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "buffer 操作的 distance_m 必须为 (0, 1000000] 米范围内的有限正数",
            ));
        }
        let quad_segs = request.quad_segs.unwrap_or(16).clamp(4, 64) as c_int;
        let mut out = Vec::with_capacity(features.len());
        for feat in &features {
            let geom = feat.get("geometry").unwrap_or(&Value::Null);
            let buf_geom = buffer_single_geometry(runtime, geom, distance_m, quad_segs)?;
            out.push(json!({
                "type": "Feature",
                "properties": feat.get("properties").cloned().unwrap_or_else(|| json!({})),
                "geometry": buf_geom,
            }));
        }
        out
    } else {
        features
    };

    let mut fc_bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for feat in &out_features {
        if let Some(geom) = feat.get("geometry") {
            accumulate_bbox(geom, &mut fc_bbox)?;
        }
    }
    let out_ext = request
        .output_path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !matches!(out_ext.as_str(), "geojson" | "json" | "shp") {
        return Err(error(
            NativeGisErrorCode::AoiFormatUnsupported,
            format!("不支持的输出矢量格式: .{out_ext} (支持 .geojson / .json / .shp)"),
        ));
    }

    let parent = request
        .output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|_| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "无法创建矢量处理输出目录",
        )
    })?;
    let file_name = request
        .output_path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| error(NativeGisErrorCode::AoiGeometryInvalid, "输出矢量文件名无效"))?;

    let nonce = VECTOR_STAGING_NONCE.fetch_add(1, Ordering::Relaxed);
    let staging_dir = parent.join(format!(".eostage_vec_{}_{}", std::process::id(), nonce));
    let _ = fs::remove_dir_all(&staging_dir);
    fs::create_dir_all(&staging_dir).map_err(|_| {
        error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "无法创建矢量处理暂存目录",
        )
    })?;

    let write_res: Result<Vec<String>, NativeGisError> = (|| {
        if matches!(out_ext.as_str(), "geojson" | "json") {
            let fc = json!({
                "type": "FeatureCollection",
                "features": out_features,
            });
            let text = serde_json::to_string_pretty(&fc).map_err(|_| {
                error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "序列化 GeoJSON 输出失败",
                )
            })?;
            let tmp_file = staging_dir.join(file_name);
            fs::write(&tmp_file, text.as_bytes()).map_err(|_| {
                error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    "写入 GeoJSON 暂存文件失败",
                )
            })?;
            if request.output_path.exists() {
                let _ = fs::remove_file(&request.output_path);
            }
            if fs::rename(&tmp_file, &request.output_path).is_err() {
                fs::copy(&tmp_file, &request.output_path).map_err(|_| {
                    error(
                        NativeGisErrorCode::AoiGeometryInvalid,
                        "提交 GeoJSON 输出文件失败",
                    )
                })?;
            }
            Ok(vec![request.output_path.to_string_lossy().to_string()])
        } else {
            let parsed = feature_collection_to_mapgis(&out_features)?;
            let tmp_shp = staging_dir.join(file_name);
            let tmp_written = write_shapefile_bundle(&parsed, &tmp_shp)?;
            let mut final_files = Vec::with_capacity(tmp_written.len());
            for tmp_path_str in tmp_written {
                let tmp_p = Path::new(&tmp_path_str);
                let Some(ext_p) = tmp_p.extension() else {
                    continue;
                };
                let dest_p = request.output_path.with_extension(ext_p);
                if dest_p.exists() {
                    let _ = fs::remove_file(&dest_p);
                }
                if fs::rename(tmp_p, &dest_p).is_err() {
                    fs::copy(tmp_p, &dest_p).map_err(|_| {
                        error(
                            NativeGisErrorCode::AoiGeometryInvalid,
                            "提交 Shapefile 输出文件失败",
                        )
                    })?;
                }
                final_files.push(dest_p.to_string_lossy().to_string());
            }
            Ok(final_files)
        }
    })();

    let _ = fs::remove_dir_all(&staging_dir);
    let written_files = write_res?;
    let bytes_written = fs::metadata(&request.output_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(json!({
        "ok": true,
        "executed": true,
        "operation": op,
        "input_path": request.input_path,
        "output_path": request.output_path,
        "written_files": written_files,
        "feature_count": out_features.len(),
        "bbox": fc_bbox.iter().all(|v| v.is_finite()).then_some(fc_bbox),
        "bytes_written": bytes_written,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::NativeGisRuntimeIdentity;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn require_test_runtime() -> NativeGisRuntime {
        let mut candidates = Vec::new();
        if let Some(root) = std::env::var_os("INSAR_GIS_RUNTIME_ROOT") {
            candidates.push(PathBuf::from(root));
        }
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        candidates.push(manifest_dir.join("../../runtime/gis"));
        candidates.push(manifest_dir.join("../../../runtime/gis"));
        for candidate in candidates {
            if candidate.join("manifest.json").is_file() {
                let identity = NativeGisRuntimeIdentity::from_product_root(&candidate)
                    .unwrap_or_else(|e| {
                        panic!(
                            "invalid GIS runtime manifest at {}: {e}",
                            candidate.display()
                        )
                    });
                return NativeGisRuntime::load_identity(&identity).unwrap_or_else(|e| {
                    panic!("failed to load GIS runtime at {}: {e}", candidate.display())
                });
            }
        }
        panic!("INSAR_GIS_RUNTIME_ROOT product GIS runtime must be available for vector_ops tests");
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn native_vector_buffer_metric_precision_and_shapefile_roundtrip_work() {
        let runtime = require_test_runtime();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("insar_gis_vec_ops_{stamp}"));
        fs::create_dir_all(&dir).unwrap();

        let point_in = dir.join("hefei_station.geojson");
        fs::write(
            &point_in,
            serde_json::to_string_pretty(&json!({
                "type": "FeatureCollection",
                "features": [{
                    "type": "Feature",
                    "properties": { "NAME": "HefeiStation" },
                    "geometry": { "type": "Point", "coordinates": [117.28, 31.86] }
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        // 1. Metric 500 m buffer around (117.28, 31.86)
        let buf_out = dir.join("hefei_buffer_500m.geojson");
        let buf_res = process_vector(
            &runtime,
            &VectorProcessRequest {
                input_path: point_in.clone(),
                output_path: buf_out.clone(),
                operation: "buffer".to_owned(),
                distance_m: Some(500.0),
                quad_segs: Some(24),
            },
        )
        .expect("metric 500m buffer");
        assert!(buf_out.is_file());
        assert_eq!(buf_res["feature_count"], 1);
        let fc: Value = serde_json::from_str(&fs::read_to_string(&buf_out).unwrap()).unwrap();
        let geom = &fc["features"][0]["geometry"];
        assert_eq!(geom["type"], "Polygon");
        let ring = geom["coordinates"][0].as_array().unwrap();
        assert_eq!(ring.first(), ring.last(), "polygon ring must be closed");
        let bbox = buf_res["bbox"].as_array().unwrap();
        let west = bbox[0].as_f64().unwrap();
        let south = bbox[1].as_f64().unwrap();
        let east = bbox[2].as_f64().unwrap();
        let north = bbox[3].as_f64().unwrap();
        let lat_rad = 31.86f64.to_radians();
        let half_ew_m = 0.5 * (east - west) * 111_320.0 * lat_rad.cos();
        let half_ns_m = 0.5 * (north - south) * 111_320.0;
        assert!(
            (half_ew_m - 500.0).abs() < 5.0,
            "expected ~500m E-W radius, got {half_ew_m}"
        );
        assert!(
            (half_ns_m - 500.0).abs() < 5.0,
            "expected ~500m N-S radius, got {half_ns_m}"
        );

        // 2. Convert GeoJSON -> Shapefile bundle -> GeoJSON
        let shp_out = dir.join("hefei_buffer.shp");
        let to_shp = process_vector(
            &runtime,
            &VectorProcessRequest {
                input_path: buf_out.clone(),
                output_path: shp_out.clone(),
                operation: "convert".to_owned(),
                ..Default::default()
            },
        )
        .expect("convert geojson to shp");
        assert!(shp_out.is_file());
        assert!(dir.join("hefei_buffer.shx").is_file());
        assert!(dir.join("hefei_buffer.dbf").is_file());
        assert!(dir.join("hefei_buffer.prj").is_file());
        assert_eq!(to_shp["feature_count"], 1);

        let roundtrip_out = dir.join("hefei_buffer_roundtrip.geojson");
        let back = process_vector(
            &runtime,
            &VectorProcessRequest {
                input_path: shp_out,
                output_path: roundtrip_out.clone(),
                operation: "convert".to_owned(),
                ..Default::default()
            },
        )
        .expect("convert shp back to geojson");
        assert!(roundtrip_out.is_file());
        assert_eq!(back["feature_count"], 1);

        let _ = fs::remove_dir_all(&dir);
    }
}
