use insar_gis::{
    convert_mapgis_file, inspect_mapgis_file, transform_lbs_coordinates, LazyNativeGisEngine,
    RasterProcessRequest, VectorProcessRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GisRasterInspectRequest {
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LbsCoordTransformRequest {
    pub from: String,
    pub to: String,
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapGisInspectRequest {
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapGisConvertRequest {
    pub input_path: String,
    pub output_path: String,
    #[serde(default)]
    pub scale: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpenGeodataQueryRequest {
    /// Dataset selector:
    /// - `"mca_xzqh"` (China Ministry of Civil Affairs administrative division codes)
    /// - `"mca_placename"` (China standard placename search)
    /// - `"datav_bound"` (Aliyun DataV GeoJSON boundary download)
    /// - `"open_meteo_era5"` (Open-Meteo ERA5 historical daily weather)
    /// - `"overpass_osm"` (Overpass OSM vector elements by bbox & tag)
    /// - `"worldpop"` (WorldPop 1km population grid metadata & COG URL)
    /// - `"open_dem_tiles"` (Multi-mirror COP30 / COP90 / NASADEM / AW3D30 tile resolver)
    pub dataset: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub bbox: Option<[f64; 4]>,
    #[serde(default)]
    pub aoi_geojson: Option<Value>,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lng: Option<f64>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub osm_tag: Option<String>,
    #[serde(default)]
    pub dem_product: Option<String>,
    #[serde(default)]
    pub iso3: Option<String>,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub full_children: Option<bool>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_args: Option<Vec<String>>,
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default)]
    pub gee_params: Option<Value>,
    #[serde(default)]
    pub zooms: Option<Vec<u8>>,
}

pub fn discover_native_gis_engine() -> LazyNativeGisEngine {
    let discovered = LazyNativeGisEngine::discover_product();
    if discovered.is_discovered() {
        return discovered;
    }
    let mut candidates = Vec::new();
    if let Some(env_root) = std::env::var_os("INSAR_GIS_RUNTIME_ROOT") {
        candidates.push(PathBuf::from(env_root));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/gis"));
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.parent();
        for _ in 0..7 {
            if let Some(dir) = cur {
                candidates.push(dir.join("runtime").join("gis"));
                candidates.push(dir.join("Resources").join("runtime").join("gis"));
                cur = dir.parent();
            }
        }
    }
    for cand in candidates {
        if cand.join("manifest.json").is_file() {
            if let Ok(engine) = LazyNativeGisEngine::from_product_root(&cand) {
                return engine;
            }
        }
    }
    LazyNativeGisEngine::unavailable()
}

pub fn execute_gis_raster_inspect(req: &GisRasterInspectRequest) -> Result<Value, String> {
    let engine = discover_native_gis_engine();
    engine
        .inspect_raster_source(&req.source)
        .map_err(|err| format!("{}: {err}", err.code().as_str()))
}

fn collect_geotiffs_bounded(dir: &Path, depth: usize, out: &mut Vec<(u64, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if depth < 2 {
                collect_geotiffs_bounded(&path, depth + 1, out);
            }
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if (lower.ends_with(".tif") || lower.ends_with(".tiff"))
            && !lower.ends_with(".lineage_source_index.tif")
        {
            if let Ok(meta) = fs::metadata(&path) {
                if meta.is_file() && meta.len() > 0 {
                    out.push((meta.len(), path));
                }
            }
        }
    }
}

pub fn execute_gis_raster_pixel_probe(
    req: &insar_gis::RasterPixelProbeRequest,
) -> Result<Value, String> {
    let trimmed = req.source.trim();
    let resolved_source = if !trimmed.starts_with("http://")
        && !trimmed.starts_with("https://")
        && !trimmed.starts_with("/vsicurl/")
        && Path::new(trimmed).is_dir()
    {
        let mut candidates = Vec::new();
        collect_geotiffs_bounded(Path::new(trimmed), 0, &mut candidates);
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        candidates
            .into_iter()
            .next()
            .map(|(_, path)| path.to_string_lossy().into_owned())
            .ok_or_else(|| {
                "DEM_INVALID_RASTER: 任务输出目录中未找到可供像元采样的 GeoTIFF 影像".to_owned()
            })?
    } else {
        trimmed.to_owned()
    };
    let resolved_req = insar_gis::RasterPixelProbeRequest {
        source: resolved_source,
        lon: req.lon,
        lat: req.lat,
    };
    let engine = discover_native_gis_engine();
    engine
        .probe_raster_pixel(&resolved_req)
        .map_err(|err| format!("{}: {err}", err.code().as_str()))
}

pub fn execute_gis_raster_process(req: &RasterProcessRequest) -> Result<Value, String> {
    let engine = discover_native_gis_engine();
    engine
        .process_raster_window(req)
        .map_err(|err| format!("{}: {err}", err.code().as_str()))
}

pub fn execute_gis_vector_process(req: &VectorProcessRequest) -> Result<Value, String> {
    let engine = discover_native_gis_engine();
    engine
        .process_vector(req)
        .map_err(|err| format!("{}: {err}", err.code().as_str()))
}

pub fn execute_lbs_coord_transform(req: &LbsCoordTransformRequest) -> Result<Value, String> {
    transform_lbs_coordinates(&req.from, &req.to, &req.points)
        .map_err(|err| format!("{}: {err}", err.code().as_str()))
}

pub fn execute_mapgis_inspect(req: &MapGisInspectRequest) -> Result<Value, String> {
    inspect_mapgis_file(Path::new(req.path.trim()))
        .map_err(|err| format!("{}: {err}", err.code().as_str()))
}

pub fn execute_mapgis_convert(req: &MapGisConvertRequest) -> Result<Value, String> {
    convert_mapgis_file(
        Path::new(req.input_path.trim()),
        Path::new(req.output_path.trim()),
        req.scale,
    )
    .map_err(|err| format!("{}: {err}", err.code().as_str()))
}

/// Resolve 1°x1° open DEM tiles (`cop30`, `cop90`, `nasadem`, `aw3d30`) across
/// AWS Open Data, OpenTopography S3, and Microsoft Planetary Computer Blob,
/// returning both direct HTTPS URLs and `/vsicurl/` streaming paths.
pub fn resolve_open_dem_tiles(product_raw: &str, bbox: [f64; 4]) -> Result<Value, String> {
    let [west, south, east, north] = bbox;
    if !west.is_finite()
        || !south.is_finite()
        || !east.is_finite()
        || !north.is_finite()
        || west >= east
        || south >= north
        || west < -180.0
        || east > 180.0
        || south < -90.0
        || north > 90.0
    {
        return Err("INVALID_BBOX: 经纬度范围无效，需满足 [west, south, east, north]".to_owned());
    }
    let product = product_raw.trim().to_ascii_lowercase();
    let lat_min = south.floor() as i32;
    let lat_max = (north - 1e-9).floor() as i32;
    let lon_min = west.floor() as i32;
    let lon_max = (east - 1e-9).floor() as i32;
    if (lat_max - lat_min + 1) * (lon_max - lon_min + 1) > 64 {
        return Err("TOO_MANY_TILES: 单次解析的 1°×1° DEM 瓦片数量上限为 64 幅".to_owned());
    }

    let mut tiles = Vec::new();
    for lat in lat_min..=lat_max {
        for lon in lon_min..=lon_max {
            let ns_upper = if lat >= 0 { 'N' } else { 'S' };
            let ew_upper = if lon >= 0 { 'E' } else { 'W' };
            let ns_lower = if lat >= 0 { 'n' } else { 's' };
            let ew_lower = if lon >= 0 { 'e' } else { 'w' };
            let lat_abs = lat.unsigned_abs();
            let lon_abs = lon.unsigned_abs();

            let (primary_url, mirror_urls, tile_id) = match product.as_str() {
                "cop90" | "copernicus90" | "glo90" => {
                    let stem = format!(
                        "Copernicus_DSM_COG_30_{ns_upper}{lat_abs:02}_00_{ew_upper}{lon_abs:03}_00_DEM"
                    );
                    let aws =
                        format!("https://copernicus-dem-90m.s3.amazonaws.com/{stem}/{stem}.tif");
                    let ot = format!(
                        "https://opentopography.s3.sdsc.edu/raster/COP90/COP90_hh/Copernicus_DSM_30_{ns_upper}{lat_abs:02}_00_{ew_upper}{lon_abs:03}_00_DEM.tif"
                    );
                    let pc = format!(
                        "https://elevationeuwest.blob.core.windows.net/copernicus-dem/COP90_hh/Copernicus_DSM_COG_30_{ns_upper}{lat_abs:02}_00_{ew_upper}{lon_abs:03}_00_DEM.tif"
                    );
                    (aws.clone(), vec![aws, ot, pc], stem)
                }
                "nasadem" => {
                    let id = format!("NASADEM_HGT_{ns_lower}{lat_abs:02}{ew_lower}{lon_abs:03}");
                    let pc = format!(
                        "https://nasademeuwest.blob.core.windows.net/nasadem-cog/v001/{id}.tif"
                    );
                    let ot = format!(
                        "https://opentopography.s3.sdsc.edu/raster/NASADEM/NASADEM_be/{id}.tif"
                    );
                    (ot.clone(), vec![ot, pc], id)
                }
                "aw3d30" | "alos" => {
                    let id = format!("ALPSMLC30_{ns_upper}{lat_abs:03}{ew_upper}{lon_abs:03}_DSM");
                    let ot = format!(
                        "https://opentopography.s3.sdsc.edu/raster/AW3D30/AW3D30_global/{id}.tif"
                    );
                    let pc = format!(
                        "https://ai4edataeuwest.blob.core.windows.net/alos-dem/AW3D30_global/{id}.tif"
                    );
                    (ot.clone(), vec![ot, pc], id)
                }
                "fabdem" | "fabdem_v1_2" => {
                    if let Ok((stem, url)) = crate::application::fabdem_tile_url(lat, lon) {
                        (url.clone(), vec![url], stem)
                    } else {
                        continue;
                    }
                }
                "gem15_dtm" | "gem15" => {
                    if let Ok((stem, url)) =
                        crate::application::gem15_tile_url("GEM15_DTM", lat, lon)
                    {
                        (url.clone(), vec![url], stem)
                    } else {
                        continue;
                    }
                }
                "gem15_dsm" => {
                    if let Ok((stem, url)) =
                        crate::application::gem15_tile_url("GEM15_DSM", lat, lon)
                    {
                        (url.clone(), vec![url], stem)
                    } else {
                        continue;
                    }
                }
                _ => {
                    let stem = format!(
                        "Copernicus_DSM_COG_10_{ns_upper}{lat_abs:02}_00_{ew_upper}{lon_abs:03}_00_DEM"
                    );
                    let aws =
                        format!("https://copernicus-dem-30m.s3.amazonaws.com/{stem}/{stem}.tif");
                    let ot = format!(
                        "https://opentopography.s3.sdsc.edu/raster/COP30/COP30_hh/Copernicus_DSM_10_{ns_upper}{lat_abs:02}_00_{ew_upper}{lon_abs:03}_00_DEM.tif"
                    );
                    let pc = format!(
                        "https://elevationeuwest.blob.core.windows.net/copernicus-dem/COP30_hh/{stem}.tif"
                    );
                    (aws.clone(), vec![aws, ot, pc], stem)
                }
            };
            let mut tile_entry = json!({
                "tile_id": tile_id,
                "tile_bbox": [lon as f64, lat as f64, (lon + 1) as f64, (lat + 1) as f64],
                "primary_url": primary_url,
                "vsicurl_path": format!("/vsicurl/{primary_url}"),
                "mirrors": mirror_urls,
            });
            if matches!(product.as_str(), "fabdem" | "fabdem_v1_2") {
                if let Some(obj) = tile_entry.as_object_mut() {
                    obj.insert(
                        "license".to_owned(),
                        json!("CC BY-NC-SA 4.0 (Non-Commercial)"),
                    );
                }
            }
            if matches!(product.as_str(), "gem15" | "gem15_dtm" | "gem15_dsm") {
                if let Some(obj) = tile_entry.as_object_mut() {
                    obj.insert("license".to_owned(), json!("CC BY 4.0"));
                    obj.insert(
                        "coverage_note".to_owned(),
                        json!("Excludes Mainland China, Russia, Mongolia, Greenland, and Antarctica in v1"),
                    );
                }
            }
            tiles.push(tile_entry);
        }
    }

    let mut payload = json!({
        "ok": true,
        "dataset": "open_dem_tiles",
        "product": if product.is_empty() { "cop30".to_owned() } else { product },
        "bbox": bbox,
        "tile_count": tiles.len(),
        "tiles": tiles,
    });
    if product_raw.trim().eq_ignore_ascii_case("fabdem")
        || product_raw.trim().eq_ignore_ascii_case("fabdem_v1_2")
    {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "license".to_owned(),
                json!("CC BY-NC-SA 4.0 (Non-Commercial)"),
            );
        }
    }
    if matches!(
        product_raw.trim().to_ascii_lowercase().as_str(),
        "gem15" | "gem15_dtm" | "gem15_dsm"
    ) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("license".to_owned(), json!("CC BY 4.0"));
            obj.insert(
                "coverage_note".to_owned(),
                json!("Excludes Mainland China, Russia, Mongolia, Greenland, and Antarctica in v1"),
            );
        }
    }
    Ok(payload)
}

fn closed_geojson_geometry_for_aoi_check(geom: &Value) -> Option<serde_json::Map<String, Value>> {
    let mut obj = geom.as_object()?.clone();
    if obj.get("type").and_then(Value::as_str) == Some("Polygon") {
        if let Some(rings) = obj.get_mut("coordinates").and_then(Value::as_array_mut) {
            for ring_val in rings.iter_mut() {
                if let Some(ring) = ring_val.as_array_mut() {
                    if ring.len() >= 3 && ring.first() != ring.last() {
                        if let Some(first) = ring.first().cloned() {
                            ring.push(first);
                        }
                    }
                }
            }
        }
    }
    Some(obj)
}

fn geojson_bbox_bounds(geom: &Value) -> Option<[f64; 4]> {
    fn visit_coords(val: &Value, bounds: &mut [f64; 4], any: &mut bool) {
        let Some(arr) = val.as_array() else {
            return;
        };
        if arr.len() >= 2 && arr[0].is_number() && arr[1].is_number() {
            if let (Some(lon), Some(lat)) = (arr[0].as_f64(), arr[1].as_f64()) {
                if lon.is_finite() && lat.is_finite() {
                    bounds[0] = bounds[0].min(lon);
                    bounds[1] = bounds[1].min(lat);
                    bounds[2] = bounds[2].max(lon);
                    bounds[3] = bounds[3].max(lat);
                    *any = true;
                }
            }
            return;
        }
        for child in arr {
            visit_coords(child, bounds, any);
        }
    }
    let coords = geom.get("coordinates")?;
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut any = false;
    visit_coords(coords, &mut bounds, &mut any);
    (any && bounds[0] < bounds[2] && bounds[1] < bounds[3]).then_some(bounds)
}

pub(crate) fn filter_open_geodata_items_by_aoi(
    items: &mut Vec<Value>,
    aoi_geojson: Option<&Value>,
) {
    let Some(aoi_obj) = aoi_geojson.and_then(Value::as_object) else {
        return;
    };
    let Some(prepared) = insar_provider::catalog::PreparedCatalogueAoi::from_geojson(aoi_obj)
    else {
        return;
    };
    items.retain(|item| {
        if let Some(geom_obj) = item
            .get("geometry")
            .and_then(closed_geojson_geometry_for_aoi_check)
        {
            if insar_core::validated_polygonal_geojson(&geom_obj).is_some() {
                return prepared.intersects_footprint(&geom_obj);
            }
        }
        if let Some(arr) = item.get("bbox").and_then(Value::as_array) {
            if arr.len() >= 4 {
                if let (Some(w), Some(s), Some(e), Some(n)) = (
                    arr[0].as_f64(),
                    arr[1].as_f64(),
                    arr[2].as_f64(),
                    arr[3].as_f64(),
                ) {
                    if w < e && s < n {
                        let rect = json!({
                            "type": "Polygon",
                            "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]
                        });
                        if let Some(rect_obj) = rect.as_object() {
                            return prepared.intersects_footprint(rect_obj);
                        }
                    }
                }
            }
        }
        true
    });
}

fn parse_scutil_proxy_output(raw: &str) -> Option<String> {
    let mut http_enable = false;
    let mut http_host = String::new();
    let mut http_port = String::new();
    let mut https_enable = false;
    let mut https_host = String::new();
    let mut https_port = String::new();
    let mut socks_enable = false;
    let mut socks_host = String::new();
    let mut socks_port = String::new();

    for line in raw.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        match key {
            "HTTPEnable" => http_enable = val == "1",
            "HTTPProxy" => http_host = val.to_owned(),
            "HTTPPort" => http_port = val.to_owned(),
            "HTTPSEnable" => https_enable = val == "1",
            "HTTPSProxy" => https_host = val.to_owned(),
            "HTTPSPort" => https_port = val.to_owned(),
            "SOCKSEnable" => socks_enable = val == "1",
            "SOCKSProxy" => socks_host = val.to_owned(),
            "SOCKSPort" => socks_port = val.to_owned(),
            _ => {}
        }
    }
    if https_enable && !https_host.is_empty() && !https_port.is_empty() {
        return Some(format!("http://{https_host}:{https_port}"));
    }
    if http_enable && !http_host.is_empty() && !http_port.is_empty() {
        return Some(format!("http://{http_host}:{http_port}"));
    }
    if socks_enable && !socks_host.is_empty() && !socks_port.is_empty() {
        return Some(format!("socks5h://{socks_host}:{socks_port}"));
    }
    None
}

#[cfg(target_os = "macos")]
fn detect_macos_scutil_proxy_url() -> Option<String> {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    type ScutilProxyCache = Option<(Instant, Option<String>)>;
    static CACHE: OnceLock<Mutex<ScutilProxyCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((ts, ref cached)) = *guard {
            if ts.elapsed() < Duration::from_secs(10) {
                return cached.clone();
            }
        }
    }
    let detected = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|txt| parse_scutil_proxy_output(&txt));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), detected.clone()));
    }
    detected
}

#[cfg(not(target_os = "macos"))]
fn detect_macos_scutil_proxy_url() -> Option<String> {
    None
}

fn has_ambient_proxy_env() -> bool {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .iter()
    .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

fn read_loopback_mpc_base_url(app_data_dir: &Path) -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let net_path = app_data_dir.join("network-settings.json");
    let raw = fs::read_to_string(net_path).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    let base = parsed["mpc_test_base_url"].as_str()?.trim();
    if base.is_empty() {
        return None;
    }
    let url = reqwest::Url::parse(base).ok()?;
    if url.scheme() != "http"
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let host = url.host_str()?.trim_matches(['[', ']']);
    let ip: std::net::IpAddr = host.parse().ok()?;
    if !ip.is_loopback() {
        return None;
    }
    Some(base.trim_end_matches('/').to_owned())
}

fn apply_geodata_client_proxy(
    mut builder: reqwest::blocking::ClientBuilder,
    app_data_dir: &Path,
    honor_loopback: bool,
) -> Result<(reqwest::blocking::ClientBuilder, bool), String> {
    let mut loopback_tls_bypass =
        honor_loopback && read_loopback_mpc_base_url(app_data_dir).is_some();
    let mut force_direct = loopback_tls_bypass;
    let mut manual_proxy_url: Option<String> = None;

    let net_path = app_data_dir.join("network-settings.json");
    if net_path.is_file() {
        if let Ok(raw) = fs::read_to_string(&net_path) {
            if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                if honor_loopback
                    && cfg!(debug_assertions)
                    && parsed.get("test_loopback").and_then(Value::as_bool) == Some(true)
                {
                    loopback_tls_bypass = true;
                    force_direct = true;
                }
                let proxy_enabled_opt = parsed.get("proxy_enabled").and_then(Value::as_bool);
                let proxy_url = parsed["proxy_url"]
                    .as_str()
                    .or_else(|| parsed["https_proxy"].as_str())
                    .or_else(|| parsed["http_proxy"].as_str())
                    .or_else(|| parsed["all_proxy"].as_str())
                    .map(str::trim)
                    .unwrap_or("");
                let proxy_mode = parsed["proxy_mode"]
                    .as_str()
                    .or_else(|| parsed["mode"].as_str())
                    .map(str::trim)
                    .unwrap_or("");
                if !loopback_tls_bypass {
                    if proxy_mode.eq_ignore_ascii_case("direct") || proxy_enabled_opt == Some(false)
                    {
                        force_direct = true;
                    } else if !proxy_url.is_empty()
                        && (proxy_enabled_opt == Some(true)
                            || (proxy_enabled_opt.is_none()
                                && (proxy_mode.eq_ignore_ascii_case("manual")
                                    || proxy_mode.eq_ignore_ascii_case("custom")
                                    || proxy_mode.is_empty())))
                    {
                        manual_proxy_url = Some(proxy_url.to_owned());
                    }
                }
            }
        }
    }

    if loopback_tls_bypass {
        builder = builder.tls_built_in_root_certs(false);
    }

    if force_direct {
        builder = builder.no_proxy();
    } else if let Some(proxy_url) = manual_proxy_url {
        let proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|err| format!("INVALID_PROXY_URL ({proxy_url}): {err}"))?;
        builder = builder.no_proxy().proxy(proxy);
    } else if !has_ambient_proxy_env() {
        if let Some(sys_proxy_url) = detect_macos_scutil_proxy_url() {
            if let Ok(proxy) = reqwest::Proxy::all(&sys_proxy_url) {
                let proxy =
                    proxy.no_proxy(reqwest::NoProxy::from_string("127.0.0.1,localhost,::1"));
                builder = builder.no_proxy().proxy(proxy);
            }
        }
    }

    Ok((builder, loopback_tls_bypass))
}

fn build_http_client(app_data_dir: &Path) -> Result<reqwest::blocking::Client, String> {
    build_http_client_with_timeout(app_data_dir, 30)
}

fn build_download_http_client(app_data_dir: &Path) -> Result<reqwest::blocking::Client, String> {
    let builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(1800))
        .user_agent("EOStage-GeoDataTools/1.0");
    let (builder, _) = apply_geodata_client_proxy(builder, app_data_dir, true)?;
    builder
        .build()
        .map_err(|err| format!("HTTP_CLIENT_INIT_FAILED: {err}"))
}

fn build_http_client_with_timeout(
    app_data_dir: &Path,
    timeout_secs: u64,
) -> Result<reqwest::blocking::Client, String> {
    let builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent("EOStage-GeoDataTools/1.0");
    let (builder, _) = apply_geodata_client_proxy(builder, app_data_dir, true)?;
    builder
        .build()
        .map_err(|err| format!("HTTP_CLIENT_INIT_FAILED: {err}"))
}

fn http_get_with_retry(
    client: &reqwest::blocking::Client,
    url: &str,
    query: Option<&[(&str, &str)]>,
    max_attempts: usize,
) -> Result<reqwest::blocking::Response, String> {
    let attempts = max_attempts.max(1);
    let mut last_err = String::new();
    for attempt in 0..attempts {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500 * attempt as u64));
        }
        let mut req_builder = client.get(url);
        if let Some(q) = query {
            req_builder = req_builder.query(q);
        }
        match req_builder.send() {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if (resp.status().is_server_error() || code == 408 || code == 429)
                    && attempt + 1 < attempts
                {
                    last_err = format!("HTTP {code}");
                    continue;
                }
                return Ok(resp);
            }
            Err(err) => {
                last_err = err.to_string();
            }
        }
    }
    Err(last_err)
}

pub const XYZ_MAX_TILES_MOSAIC: usize = 50_000;
pub const XYZ_MAX_TILES_ARCHIVE: usize = 200_000;
pub const XYZ_JPEG_MAX_DIMENSION: u64 = 65_535;
pub(crate) const EOSTAGE_TILE_USER_AGENT: &str = concat!("EOStage/", env!("CARGO_PKG_VERSION"));

#[path = "wayback.rs"]
pub(crate) mod wayback;

pub(crate) fn xyz_source_max_zoom(source_key: &str) -> u32 {
    if wayback::parse_wayback_version_id(source_key).is_some() {
        return 20;
    }
    match source_key.trim().to_ascii_lowercase().as_str() {
        "esri_wayback" => 20,
        "carto_light" | "cartolight" | "carto_dark" | "cartodark" => 16,
        "opentopomap" | "opentopo" => 17,
        "tianditu_satellite" | "tianditusatellite" | "tianditu_img" | "tianditu_vector"
        | "tiandituvector" | "tianditu_vec" | "gaode_satellite" | "gaodesatellite"
        | "gaode_sat" | "gaode" | "autonavi_satellite" | "amap_satellite" | "amap"
        | "gaode_map" | "gaodemap" | "gaode_vector" | "gaode_vec" | "autonavi_vector"
        | "amap_vector" => 18,
        "osm" | "openstreetmap" | "arcgis_topo" | "arcgistopo" | "esri_topo" | "arcgis_street"
        | "arcgisstreet" | "esri_street" => 19,
        "arcgis_satellite" | "arcgissatellite" | "esri_satellite" | "esri" => 20,
        "google_satellite" | "googlesatellite" | "google_sat" | "google" | "google_hybrid"
        | "googlehybrid" | "google_map" | "googlemap" | "google_roadmap" | "mt1" => 21,
        _ => 22,
    }
}

pub(crate) fn xyz_max_tiles_limit(requires_raster_stitch_or_vrt: bool) -> usize {
    if requires_raster_stitch_or_vrt {
        XYZ_MAX_TILES_MOSAIC
    } else {
        XYZ_MAX_TILES_ARCHIVE
    }
}

pub(crate) fn resolve_tianditu_token(app_data_dir: &Path) -> String {
    let net_path = app_data_dir.join("network-settings.json");
    if let Ok(raw) = fs::read_to_string(&net_path) {
        if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
            if let Some(tk) = parsed.get("tianditu_token").and_then(|v| v.as_str()) {
                let trimmed = tk.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_owned();
                }
            }
        }
    }
    String::new()
}

pub(crate) fn percent_encode_query_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        let ch = *byte;
        if ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_' | b'.' | b'~') {
            out.push(ch as char);
        } else {
            out.push_str(&format!("%{ch:02X}"));
        }
    }
    out
}

pub(crate) fn redact_tile_url_secrets(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    let (without_frag, frag_opt) = match trimmed.split_once('#') {
        Some((before, frag)) => (before, Some(frag)),
        None => (trimmed, None),
    };
    let Some((base, query)) = without_frag.split_once('?') else {
        return trimmed.to_owned();
    };
    let mut redacted_pairs = Vec::new();
    for pair in query.split('&') {
        if let Some((key, val)) = pair.split_once('=') {
            let lower_key = key.trim().to_ascii_lowercase();
            if matches!(
                lower_key.as_str(),
                "tk" | "token" | "key" | "apikey" | "access_token"
            ) && !val.is_empty()
            {
                redacted_pairs.push(format!("{key}=[REDACTED]"));
            } else {
                redacted_pairs.push(pair.to_owned());
            }
        } else {
            redacted_pairs.push(pair.to_owned());
        }
    }
    let mut rebuilt = format!("{base}?{}", redacted_pairs.join("&"));
    if let Some(frag) = frag_opt {
        rebuilt.push('#');
        rebuilt.push_str(frag);
    }
    rebuilt
}

pub(crate) fn is_tianditu_host(raw_url: &str) -> bool {
    let trimmed = raw_url.trim();
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..end];
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, hp)| hp)
        .unwrap_or(authority);
    let host = host_port
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "tianditu.gov.cn" || host.ends_with(".tianditu.gov.cn")
}

pub(crate) fn is_valid_tile_image_bytes(bytes: &[u8]) -> bool {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return true;
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return true;
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return true;
    }
    if bytes.len() >= 6 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
        return true;
    }
    if bytes.len() >= 2 && &bytes[0..2] == b"BM" {
        return true;
    }
    if bytes.len() >= 4 && (&bytes[0..4] == b"II*\0" || &bytes[0..4] == b"MM\0*") {
        return true;
    }
    false
}

pub(crate) fn classify_non_image_tile_response(url: &str, bytes: &[u8]) -> XyzTileFetchOutcome {
    let slice = &bytes[..bytes.len().min(512)];
    let snippet_lower = String::from_utf8_lossy(slice).to_ascii_lowercase();
    let snippet_utf8 = String::from_utf8_lossy(slice);
    let looks_like_auth_failure = snippet_lower.contains("invalid token")
        || snippet_lower.contains("invalid key")
        || snippet_lower.contains("unauthorized")
        || snippet_lower.contains("forbidden")
        || snippet_lower.contains("permission")
        || snippet_utf8.contains("非法")
        || (is_tianditu_host(url)
            && (snippet_lower.contains("key")
                || snippet_lower.contains("token")
                || snippet_lower.contains("resolve")));
    if looks_like_auth_failure {
        XyzTileFetchOutcome::Forbidden(403)
    } else {
        XyzTileFetchOutcome::TransientError
    }
}

pub(crate) fn build_xyz_tile_url(
    source_key: &str,
    custom_url_template: &str,
    z: u32,
    x: u32,
    y: u32,
    tianditu_token: &str,
) -> String {
    if let Some(version_id) = wayback::parse_wayback_version_id(source_key) {
        return wayback::wayback_tile_url(version_id, z, x, y);
    }
    if (source_key == "custom" || source_key == "custom_xyz" || source_key == "custom_tms")
        && !custom_url_template.is_empty()
    {
        let tms_y = (1u64 << z.min(28))
            .saturating_sub(1)
            .saturating_sub(y as u64);
        let sub = if custom_url_template.contains("t{s}.") {
            ((x + y) % 8).to_string()
        } else if custom_url_template.contains("mt{s}") {
            ((x + y) % 4).to_string()
        } else if custom_url_template.contains("webst0{s}")
            || custom_url_template.contains("webrd0{s}")
        {
            (((x + y) % 4) + 1).to_string()
        } else {
            "a".to_owned()
        };
        return custom_url_template
            .replace("{z}", &z.to_string())
            .replace("{Z}", &z.to_string())
            .replace("{x}", &x.to_string())
            .replace("{X}", &x.to_string())
            .replace("{-y}", &tms_y.to_string())
            .replace("{y}", &y.to_string())
            .replace("{Y}", &y.to_string())
            .replace("{s}", &sub);
    }
    match source_key {
        "arcgis_satellite" | "arcgissatellite" | "esri_satellite" | "esri" => {
            format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"
            )
        }
        "arcgis_topo" | "arcgistopo" | "esri_topo" => {
            format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Topo_Map/MapServer/tile/{z}/{y}/{x}"
            )
        }
        "arcgis_street" | "arcgisstreet" | "esri_street" => {
            format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/{z}/{y}/{x}"
            )
        }
        "opentopomap" | "opentopo" => {
            format!("https://tile.opentopomap.org/{z}/{x}/{y}.png")
        }
        "carto_light" | "cartolight" => {
            format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Light_Gray_Base/MapServer/tile/{z}/{y}/{x}"
            )
        }
        "carto_dark" | "cartodark" => {
            format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}"
            )
        }
        "google_satellite" | "googlesatellite" | "google_sat" | "google" => {
            format!("https://mt1.google.com/vt/lyrs=s&x={x}&y={y}&z={z}")
        }
        "google_hybrid" | "googlehybrid" => {
            format!("https://mt1.google.com/vt/lyrs=y&x={x}&y={y}&z={z}")
        }
        "google_map" | "googlemap" | "google_roadmap" => {
            format!("https://mt1.google.com/vt/lyrs=m&x={x}&y={y}&z={z}")
        }
        "gaode_satellite" | "gaodesatellite" | "gaode_sat" | "gaode" | "autonavi_satellite"
        | "amap_satellite" | "amap" => {
            let sub = ((x + y) % 4) + 1;
            format!("https://webst0{sub}.is.autonavi.com/appmaptile?style=6&x={x}&y={y}&z={z}")
        }
        "gaode_map" | "gaodemap" | "gaode_vector" | "gaode_vec" | "autonavi_vector"
        | "amap_vector" => {
            let sub = ((x + y) % 4) + 1;
            format!(
                "https://webrd0{sub}.is.autonavi.com/appmaptile?lang=zh_cn&size=1&scale=1&style=8&x={x}&y={y}&z={z}"
            )
        }
        "tianditu_satellite" | "tianditusatellite" | "tianditu_img" => {
            let sub = (x + y) % 8;
            let tk = percent_encode_query_component(tianditu_token);
            format!(
                "https://t{sub}.tianditu.gov.cn/img_w/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=img&STYLE=default&TILEMATRIXSET=w&FORMAT=tiles&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}&tk={tk}"
            )
        }
        "tianditu_vector" | "tiandituvector" | "tianditu_vec" => {
            let sub = (x + y) % 8;
            let tk = percent_encode_query_component(tianditu_token);
            format!(
                "https://t{sub}.tianditu.gov.cn/vec_w/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=vec&STYLE=default&TILEMATRIXSET=w&FORMAT=tiles&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}&tk={tk}"
            )
        }
        "osm" | "openstreetmap" => {
            format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png")
        }
        _ => {
            format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"
            )
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum XyzTileFetchOutcome {
    Ok(Vec<u8>),
    NotFoundOrEmpty,
    RateLimited(u16),
    Forbidden(u16),
    TransientError,
}

fn fetch_xyz_tile_bytes(
    client: &reqwest::blocking::Client,
    url: &str,
    abort_flag: &std::sync::atomic::AtomicBool,
) -> XyzTileFetchOutcome {
    let mut had_error = false;
    let is_tianditu = is_tianditu_host(url);
    for attempt in 0..2usize {
        if abort_flag.load(std::sync::atomic::Ordering::Acquire) {
            return XyzTileFetchOutcome::TransientError;
        }
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500));
            if abort_flag.load(std::sync::atomic::Ordering::Acquire) {
                return XyzTileFetchOutcome::TransientError;
            }
        }
        let mut req_builder = client
            .get(url)
            .header(reqwest::header::USER_AGENT, EOSTAGE_TILE_USER_AGENT);
        if is_tianditu {
            req_builder = req_builder.header(
                reqwest::header::ACCEPT,
                "image/avif,image/webp,image/apng,image/png,image/jpeg,image/*,*/*;q=0.8",
            );
        }
        match req_builder.send() {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if code == 429 {
                    return XyzTileFetchOutcome::RateLimited(code);
                }
                if code == 401 || code == 403 {
                    return XyzTileFetchOutcome::Forbidden(code);
                }
                if code == 404 {
                    return XyzTileFetchOutcome::NotFoundOrEmpty;
                }
                if resp.status().is_success() {
                    match resp.bytes() {
                        Ok(bytes) if !bytes.is_empty() => {
                            if is_valid_tile_image_bytes(&bytes) {
                                return XyzTileFetchOutcome::Ok(bytes.to_vec());
                            }
                            let classified = classify_non_image_tile_response(url, &bytes);
                            if matches!(classified, XyzTileFetchOutcome::Forbidden(_)) {
                                return classified;
                            }
                            had_error = true;
                        }
                        Ok(_) => return XyzTileFetchOutcome::NotFoundOrEmpty,
                        Err(_) => {
                            had_error = true;
                        }
                    }
                } else if resp.status().is_server_error() || code == 408 {
                    had_error = true;
                } else {
                    return XyzTileFetchOutcome::TransientError;
                }
            }
            Err(_) => {
                had_error = true;
            }
        }
    }
    let _ = had_error;
    XyzTileFetchOutcome::TransientError
}

fn base64url_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        out.push(TABLE[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
    }
    out
}

fn base64_standard_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        out.push(TABLE[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        out.push('=');
    }
    out
}

fn base64_standard_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut vals = Vec::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            break;
        }
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(format!("INVALID_BASE64_BYTE: 0x{b:02x}")),
        };
        vals.push(v);
    }
    let mut out = Vec::with_capacity(vals.len() * 3 / 4);
    let mut i = 0;
    while i + 4 <= vals.len() {
        let n = ((vals[i] as u32) << 18)
            | ((vals[i + 1] as u32) << 12)
            | ((vals[i + 2] as u32) << 6)
            | (vals[i + 3] as u32);
        out.push(((n >> 16) & 0xFF) as u8);
        out.push(((n >> 8) & 0xFF) as u8);
        out.push((n & 0xFF) as u8);
        i += 4;
    }
    let rem = vals.len() - i;
    if rem == 2 {
        let n = ((vals[i] as u32) << 18) | ((vals[i + 1] as u32) << 12);
        out.push(((n >> 16) & 0xFF) as u8);
    } else if rem == 3 {
        let n =
            ((vals[i] as u32) << 18) | ((vals[i + 1] as u32) << 12) | ((vals[i + 2] as u32) << 6);
        out.push(((n >> 16) & 0xFF) as u8);
        out.push(((n >> 8) & 0xFF) as u8);
    } else if rem == 1 {
        return Err("INVALID_BASE64_LENGTH: trailing single sextet".to_owned());
    }
    Ok(out)
}

fn sign_rs256_jwt_pure_rust(
    private_key_pem: &str,
    signing_input: &[u8],
) -> Result<Vec<u8>, String> {
    let mut body = String::new();
    let mut in_pem_block = false;
    for line in private_key_pem.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("-----BEGIN ") && trimmed.ends_with("-----") {
            in_pem_block = true;
            continue;
        }
        if trimmed.starts_with("-----END ") && trimmed.ends_with("-----") {
            break;
        }
        if in_pem_block || !private_key_pem.contains("-----BEGIN ") {
            body.push_str(trimmed);
        }
    }
    if body.is_empty() {
        return Err("INVALID_PEM_KEY: RSA 私钥为空".to_owned());
    }
    let der = base64_standard_decode(&body)
        .map_err(|e| format!("INVALID_PEM_BASE64: RSA 私钥 Base64 解码失败 ({e})"))?;
    let key_pair = ring::signature::RsaKeyPair::from_pkcs8(&der)
        .or_else(|_| ring::signature::RsaKeyPair::from_der(&der))
        .map_err(|_| {
            "INVALID_RSA_PRIVATE_KEY: RSA 私钥 DER/PKCS#8 解析失败，请检查 Service Account private_key"
                .to_owned()
        })?;
    let rng = ring::rand::SystemRandom::new();
    let mut signature = vec![0u8; key_pair.public().modulus_len()];
    key_pair
        .sign(
            &ring::signature::RSA_PKCS1_SHA256,
            &rng,
            signing_input,
            &mut signature,
        )
        .map_err(|_| "RSA_SIGN_FAILED: RS256 签名计算失败".to_owned())?;
    Ok(signature)
}

#[cfg(not(test))]
fn gee_scoped_channel_id(project_id: &str) -> Option<String> {
    let trimmed = project_id.trim();
    if trimmed.is_empty() || trimmed.len() > 48 {
        return None;
    }
    if trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Some(format!("gee-proj-{trimmed}"))
    } else {
        None
    }
}

#[cfg(test)]
static TEST_ISOLATE_EMPTY_KEYCHAIN_OVERRIDE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn parse_isolated_empty_keychain_flag(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed == "1" || trimmed.eq_ignore_ascii_case("true")
}

pub(crate) fn is_isolated_empty_keychain_mode() -> bool {
    #[cfg(test)]
    if TEST_ISOLATE_EMPTY_KEYCHAIN_OVERRIDE.load(std::sync::atomic::Ordering::SeqCst) {
        return true;
    }
    std::env::var("EOSTAGE_ISOLATE_EMPTY_KEYCHAIN")
        .map(|value| parse_isolated_empty_keychain_flag(&value))
        .unwrap_or(false)
}

#[cfg(test)]
fn gee_test_keyring_map() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    static MAP: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
fn store_gee_keyring_bundle(
    project_id: &str,
    auth_mode: &str,
    secret: &str,
    client_email: &str,
) -> Result<(), String> {
    if is_isolated_empty_keychain_mode() {
        return Err(
            "GEE_ISOLATED_EMPTY_KEYCHAIN_READONLY: 当前处于空钥匙串隔离测试模式，禁止修改本机凭据"
                .to_owned(),
        );
    }
    let mut map = gee_test_keyring_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.insert("project_id".to_owned(), project_id.to_owned());
    map.insert("auth_mode".to_owned(), auth_mode.to_owned());
    map.insert("secret".to_owned(), secret.to_owned());
    if !project_id.trim().is_empty() {
        map.insert(format!("secret:{}", project_id.trim()), secret.to_owned());
    }
    if !client_email.trim().is_empty() {
        map.insert("client_email".to_owned(), client_email.trim().to_owned());
    } else {
        map.remove("client_email");
    }
    Ok(())
}

#[cfg(not(test))]
fn store_gee_keyring_bundle(
    project_id: &str,
    auth_mode: &str,
    secret: &str,
    client_email: &str,
) -> Result<(), String> {
    if is_isolated_empty_keychain_mode() {
        return Err(
            "GEE_ISOLATED_EMPTY_KEYCHAIN_READONLY: 当前处于空钥匙串隔离测试模式，禁止修改本机凭据"
                .to_owned(),
        );
    }
    let bundle = json!({
        "project_id": project_id,
        "auth_mode": auth_mode,
        "secret": secret,
        "client_email": client_email,
    });
    let payload = bundle.to_string();
    insar_download::store_desktop_agent_model_key("gee-credentials", &payload).map_err(|e| {
        format!("GEE_KEYRING_STORE_FAILED: 无法将 GEE 凭据存入系统安全凭据库 ({e:?})")
    })?;
    if let Some(scoped_ch) = gee_scoped_channel_id(project_id) {
        let _ = insar_download::store_desktop_agent_model_key(&scoped_ch, secret);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn resolve_gee_keyring_secret_for_project(project_id: &str) -> Option<String> {
    if is_isolated_empty_keychain_mode() {
        return None;
    }
    if let Ok(test_sec) = std::env::var("EOSTAGE_GEE_TEST_SECRET") {
        if !test_sec.trim().is_empty() {
            return Some(test_sec);
        }
    }
    let map = gee_test_keyring_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let pid_trimmed = project_id.trim();
    if !pid_trimmed.is_empty() {
        if let Some(sec) = map.get(&format!("secret:{pid_trimmed}")) {
            if !sec.trim().is_empty() {
                return Some(sec.clone());
            }
        }
    }
    map.get("secret").filter(|s| !s.trim().is_empty()).cloned()
}

#[cfg(not(test))]
pub(crate) fn resolve_gee_keyring_secret_for_project(project_id: &str) -> Option<String> {
    if is_isolated_empty_keychain_mode() {
        return None;
    }
    if let Some(scoped_ch) = gee_scoped_channel_id(project_id) {
        if let Some(sec) =
            insar_download::with_desktop_agent_model_key(&scoped_ch, |s| s.to_owned())
                .ok()
                .flatten()
                .filter(|s| !s.trim().is_empty())
        {
            return Some(sec);
        }
    }
    resolve_gee_keyring_bundle()
        .map(|(_, _, sec, _)| sec)
        .filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
pub(crate) fn resolve_gee_keyring_bundle() -> Option<(String, String, String, String)> {
    if is_isolated_empty_keychain_mode() {
        return None;
    }
    let map = gee_test_keyring_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let pid = map
        .get("project_id")
        .filter(|s| !s.trim().is_empty())?
        .clone();
    let sec = map.get("secret").filter(|s| !s.trim().is_empty())?.clone();
    let mode = map
        .get("auth_mode")
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "service_account".to_owned());
    let email = map.get("client_email").cloned().unwrap_or_default();
    Some((pid, mode, sec, email))
}

fn extract_gee_bundle_from_raw_entry(
    raw: &str,
    fallback_pid: Option<&str>,
) -> Option<(String, String, String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let fb_pid = fallback_pid.map(str::trim).filter(|s| !s.is_empty());
    if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
        if let Some(sec) = val
            .get("secret")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let sec_json: Option<Value> = if sec.starts_with('{') {
                serde_json::from_str(sec).ok()
            } else {
                None
            };
            let pid = val
                .get("project_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or(fb_pid)
                .or_else(|| {
                    sec_json.as_ref().and_then(|sj| {
                        sj.get("project_id")
                            .or_else(|| sj.get("quota_project_id"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                    })
                })?
                .to_owned();
            let mode = val
                .get("auth_mode")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    if sec_json
                        .as_ref()
                        .and_then(|sj| sj.get("type"))
                        .and_then(Value::as_str)
                        == Some("authorized_user")
                    {
                        "authorized_user".to_owned()
                    } else if sec.starts_with('{') {
                        "service_account".to_owned()
                    } else {
                        "oauth_token".to_owned()
                    }
                });
            let email = val
                .get("client_email")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    sec_json
                        .as_ref()
                        .and_then(|sj| sj.get("client_email"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or("")
                .to_owned();
            return Some((pid, mode, sec.to_owned(), email));
        }

        if val.get("private_key").and_then(Value::as_str).is_some()
            || val.get("refresh_token").and_then(Value::as_str).is_some()
            || val.get("access_token").and_then(Value::as_str).is_some()
        {
            let pid = val
                .get("project_id")
                .or_else(|| val.get("quota_project_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or(fb_pid)?
                .to_owned();
            let mode = if val.get("type").and_then(Value::as_str) == Some("authorized_user") {
                "authorized_user".to_owned()
            } else {
                "service_account".to_owned()
            };
            let email = val
                .get("client_email")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_owned();
            return Some((pid, mode, trimmed.to_owned(), email));
        }
        return None;
    }

    if !trimmed.starts_with('{') {
        let pid = fb_pid?.to_owned();
        let clean_tok = trimmed.trim_start_matches("Bearer ").trim().to_owned();
        if !clean_tok.is_empty() {
            return Some((pid, "oauth_token".to_owned(), clean_tok, String::new()));
        }
    }
    None
}

#[cfg(not(test))]
pub(crate) fn resolve_gee_keyring_bundle() -> Option<(String, String, String, String)> {
    if is_isolated_empty_keychain_mode() {
        return None;
    }
    insar_download::with_desktop_agent_model_key("gee-credentials", |s| {
        extract_gee_bundle_from_raw_entry(s, None)
    })
    .ok()
    .flatten()
    .flatten()
}

pub(crate) fn resolve_gee_local_vault_bundle(
    app_data_dir: &Path,
    fallback_pid: Option<&str>,
) -> Option<(String, String, String, String)> {
    if is_isolated_empty_keychain_mode() {
        return None;
    }
    let fb_pid = fallback_pid.map(str::trim).filter(|s| !s.is_empty());
    let local_vault = app_data_dir.join("credentials-vault.v1.json");
    if local_vault.is_file() {
        if let Ok(Some(vdoc)) = insar_download::read_local_vault_map_readonly(&local_vault) {
            if let Some(pid) = fb_pid {
                if let Some(raw_secret) = vdoc
                    .get(format!("eostage:agent-model:gee-proj-{pid}"))
                    .and_then(|m| m.get(format!("gee-proj-{pid}")))
                    .and_then(Value::as_str)
                {
                    if let Some(found) = extract_gee_bundle_from_raw_entry(raw_secret, Some(pid)) {
                        return Some(found);
                    }
                }
            }

            if let Some(raw_secret) = vdoc
                .get("eostage:agent-model:gee-credentials")
                .and_then(|m| m.get("gee-credentials"))
                .and_then(Value::as_str)
            {
                if let Some(found) = extract_gee_bundle_from_raw_entry(raw_secret, fb_pid) {
                    return Some(found);
                }
            }

            if let Some(ee_obj) = vdoc.get("eostage:earthengine") {
                let v_pid = fb_pid.or_else(|| {
                    ee_obj
                        .get("project_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                });
                if let Some(raw_secret) = ee_obj.get("secret").and_then(Value::as_str) {
                    if let Some(found) = extract_gee_bundle_from_raw_entry(raw_secret, v_pid) {
                        return Some(found);
                    }
                }
            }
        }
    }

    #[cfg(not(test))]
    if let Ok(Some((ee_pid, raw_secret))) = insar_download::read_active_legacy_earthengine_entry() {
        if let Some(found) =
            extract_gee_bundle_from_raw_entry(&raw_secret, fb_pid.or(ee_pid.as_deref()))
        {
            return Some(found);
        }
    }

    None
}

pub(crate) fn scrub_gee_entries_from_local_vault(app_data_dir: &Path) {
    static LOCAL_VAULT_SCRUB_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    if is_isolated_empty_keychain_mode() {
        return;
    }
    let local_vault = app_data_dir.join("credentials-vault.v1.json");
    #[cfg(test)]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let ambient_vault = PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("EOStage")
                .join("v3")
                .join("credentials-vault.v1.json");
            if local_vault == ambient_vault
                || (local_vault.canonicalize().ok() == ambient_vault.canonicalize().ok()
                    && ambient_vault.exists())
            {
                return;
            }
        }
    }

    let _guard = LOCAL_VAULT_SCRUB_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let Ok(vault_meta) = fs::symlink_metadata(&local_vault) else {
        return;
    };
    if !vault_meta.file_type().is_file() {
        return;
    }
    let _ = insar_download::scrub_local_vault_services(&local_vault, |k| {
        k == "eostage:earthengine"
            || k == "eostage:agent-model:gee-credentials"
            || k.starts_with("eostage:agent-model:gee-proj-")
    });
}

#[cfg(test)]
fn clear_gee_keyring_bundle_for_project(_project_id_opt: Option<&str>) {
    if is_isolated_empty_keychain_mode() {
        return;
    }
    let mut map = gee_test_keyring_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.clear();
}

#[cfg(not(test))]
fn clear_gee_keyring_bundle_for_project(project_id_opt: Option<&str>) {
    if is_isolated_empty_keychain_mode() {
        return;
    }
    if let Some(pid) = project_id_opt {
        if let Some(scoped_ch) = gee_scoped_channel_id(pid) {
            let _ = insar_download::delete_desktop_agent_model_key(&scoped_ch);
        }
    }
    if let Some((existing_pid, _, _, _)) = resolve_gee_keyring_bundle() {
        if let Some(scoped_ch) = gee_scoped_channel_id(&existing_pid) {
            let _ = insar_download::delete_desktop_agent_model_key(&scoped_ch);
        }
    }
    let _ = insar_download::delete_desktop_agent_model_key("gee-credentials");
    let _ = insar_download::clear_active_legacy_earthengine_entry();
}

#[cfg(test)]
fn clear_gee_keyring_bundle() {
    clear_gee_keyring_bundle_for_project(None);
}

fn gee_expr_constant_one() -> Value {
    json!({
        "result": "0",
        "values": {
            "0": { "constantValue": 1 }
        }
    })
}

fn gee_expr_image_load(asset_id: &str) -> Value {
    json!({
        "result": "0",
        "values": {
            "0": {
                "functionInvocationValue": {
                    "functionName": "Image.load",
                    "arguments": {
                        "id": { "constantValue": asset_id }
                    }
                }
            }
        }
    })
}

fn gee_expr_collection_mosaic(asset_id: &str) -> Value {
    json!({
        "result": "0",
        "values": {
            "0": {
                "functionInvocationValue": {
                    "functionName": "ImageCollection.mosaic",
                    "arguments": {
                        "collection": { "valueReference": "1" }
                    }
                }
            },
            "1": {
                "functionInvocationValue": {
                    "functionName": "ImageCollection.load",
                    "arguments": {
                        "id": { "constantValue": asset_id }
                    }
                }
            }
        }
    })
}

fn is_static_gee_image_collection(asset_id: &str) -> bool {
    matches!(
        asset_id.trim(),
        "COPERNICUS/DEM/GLO30"
            | "COPERNICUS/DEM/GLO90"
            | "ESA/WorldCover/v100"
            | "ESA/WorldCover/v200"
            | "JRC/GSW1_4/GlobalSurfaceWater"
            | "JAXA/ALOS/AW3D30/V3_2"
            | "projects/climate-and-sustainability/assets/gem-15_v1"
            | "projects/sat-io/open-datasets/FABDEM"
            | "projects/sat-io/open-datasets/GHSL/GHSL_BUILT_H_AGBH_2018_100M"
            | "UMD/hansen/global_forest_change_2023_v1_11"
    )
}

fn normalize_gee_date_bound(raw: &str, is_end: bool) -> String {
    let trimmed = raw.trim();
    if trimmed.len() == 10 && !trimmed.contains('T') {
        if is_end {
            format!("{trimmed}T23:59:59.999Z")
        } else {
            format!("{trimmed}T00:00:00Z")
        }
    } else {
        trimmed.to_owned()
    }
}

fn sanitize_gee_route2_safe_name(gee_asset_id: &str) -> String {
    let mapped: String = gee_asset_id
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if mapped.trim_matches('_').is_empty() {
        "gee_asset".to_owned()
    } else {
        mapped
    }
}

fn gee_expr_collection_mosaic_filtered(
    asset_id: &str,
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Value {
    let eff_start = start_date.map(str::trim).filter(|s| !s.is_empty());
    let eff_end = end_date.map(str::trim).filter(|s| !s.is_empty());
    if is_static_gee_image_collection(asset_id) || (eff_start.is_none() && eff_end.is_none()) {
        return gee_expr_collection_mosaic(asset_id);
    }
    let raw_start = eff_start
        .map(|s| normalize_gee_date_bound(s, false))
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned());
    let raw_end = eff_end
        .map(|e| normalize_gee_date_bound(e, true))
        .unwrap_or_else(|| "2100-01-01T00:00:00Z".to_owned());
    let (norm_start, norm_end) = if raw_start <= raw_end {
        (raw_start, raw_end)
    } else if let (Some(s), Some(e)) = (eff_end, eff_start) {
        (
            normalize_gee_date_bound(s, false),
            normalize_gee_date_bound(e, true),
        )
    } else {
        (raw_end, raw_start)
    };
    // Canonical Google Earth Engine REST v1 Expression DAG for `ImageCollection.filterDate(start, end)`.
    // Note: `ImageCollection.filterDate` is client-side syntactic sugar in `earthengine-api` (`null` in
    // GEE's server `algorithms.json`); on REST v1 it compiles to `Collection.filter` +
    // `Filter.dateRangeContains` + `DateRange` against `"system:time_start"`.
    json!({
        "result": "0",
        "values": {
            "0": {
                "functionInvocationValue": {
                    "functionName": "ImageCollection.mosaic",
                    "arguments": {
                        "collection": { "valueReference": "2" }
                    }
                }
            },
            "1": {
                "functionInvocationValue": {
                    "functionName": "ImageCollection.load",
                    "arguments": {
                        "id": { "constantValue": asset_id }
                    }
                }
            },
            "2": {
                "functionInvocationValue": {
                    "functionName": "Collection.filter",
                    "arguments": {
                        "collection": { "valueReference": "1" },
                        "filter": { "valueReference": "3" }
                    }
                }
            },
            "3": {
                "functionInvocationValue": {
                    "functionName": "Filter.dateRangeContains",
                    "arguments": {
                        "leftValue": { "valueReference": "4" },
                        "rightField": { "constantValue": "system:time_start" }
                    }
                }
            },
            "4": {
                "functionInvocationValue": {
                    "functionName": "DateRange",
                    "arguments": {
                        "start": { "constantValue": norm_start },
                        "end": { "constantValue": norm_end }
                    }
                }
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn compute_gee_route2_scope_id(
    gee_asset_id: &str,
    safe_name: &str,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    start_date: Option<&str>,
    end_date: Option<&str>,
    explicit_bands: Option<&[String]>,
) -> (String, String) {
    use sha2::Digest as _;
    let norm = |v: f64| -> String {
        let clean = if v.abs() < 5e-7 { 0.0 } else { v };
        format!("{clean:.6}")
    };
    let s_raw = start_date.map(str::trim).unwrap_or("");
    let e_raw = end_date.map(str::trim).unwrap_or("");
    let (s_norm, e_norm) = if !s_raw.is_empty() && !e_raw.is_empty() && s_raw > e_raw {
        (e_raw, s_raw)
    } else {
        (s_raw, e_raw)
    };
    let b_norm = explicit_bands.map(|b| b.join(",")).unwrap_or_default();
    let canonical = format!(
        "v1|{}|{}|{}|{}|{}|{}|{}|{}",
        gee_asset_id.trim(),
        norm(min_lon),
        norm(min_lat),
        norm(max_lon),
        norm(max_lat),
        s_norm,
        e_norm,
        b_norm
    );
    let digest = sha2::Sha256::digest(canonical.as_bytes());
    let hash8 = format!(
        "{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    );
    let scoped_id = format!("{safe_name}_{hash8}");
    let scoped_filename = format!("GEE_{scoped_id}.tif");
    (scoped_id, scoped_filename)
}

#[allow(clippy::too_many_arguments)]
fn build_gee_catalog_search_response(
    gee_asset_id: &str,
    bbox: [f64; 4],
    scoped_item_id: &str,
    scoped_tif_name: &str,
    explicit_bands: Option<&[String]>,
    output_path: Option<&str>,
    fallback_dt: &str,
    avail_json: &Value,
) -> Value {
    let min_lon = bbox[0].min(bbox[2]);
    let min_lat = bbox[1].min(bbox[3]);
    let max_lon = bbox[0].max(bbox[2]);
    let max_lat = bbox[1].max(bbox[3]);

    let cloud_filtered_scenes = avail_json["cloud_filtered_scenes"].as_u64().unwrap_or(0);
    let total_scenes = avail_json["total_scenes"]
        .as_u64()
        .unwrap_or(cloud_filtered_scenes);
    let asset_type = avail_json["asset_type"]
        .as_str()
        .unwrap_or("image_collection");
    let expr_mode = if asset_type == "image" {
        "Image.load"
    } else {
        "ImageCollection.mosaic"
    };
    let discovered_bands: Vec<String> = explicit_bands
        .map(|b| b.to_vec())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            avail_json["bands"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            v.as_str()
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default()
        });

    let explicit_existing = output_path
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|dir_str| PathBuf::from(dir_str).join(scoped_tif_name))
        .and_then(|p| existing_valid_science_file_bytes(&p).map(|sz| (p, sz)));
    let already_in_explicit_dir = explicit_existing.is_some();
    let local_path_val = explicit_existing
        .as_ref()
        .map(|(p, _)| Value::String(p.to_string_lossy().to_string()))
        .unwrap_or(Value::Null);

    if cloud_filtered_scenes == 0 {
        return json!({
            "ok": true,
            "dataset": "gee_catalog",
            "gee_asset_id": gee_asset_id,
            "route_mode": "gee_official_rest",
            "expression_mode": expr_mode,
            "search_only": true,
            "count": 0,
            "total_scenes": total_scenes,
            "total_matching_scenes": 0,
            "already_exists": already_in_explicit_dir,
            "target_item_id": scoped_item_id,
            "bands": discovered_bands,
            "mosaic_bbox": [min_lon, min_lat, max_lon, max_lat],
            "items": [],
            "empty_reason_zh": avail_json
                .get("empty_reason_zh")
                .and_then(Value::as_str)
                .unwrap_or("当前研究区与筛选条件下暂无可用 GEE 影像（0 景）"),
        });
    }

    let poly_geom = json!({
        "type": "Polygon",
        "coordinates": [[
            [min_lon, min_lat],
            [max_lon, min_lat],
            [max_lon, max_lat],
            [min_lon, max_lat],
            [min_lon, min_lat]
        ]]
    });

    let raw_scenes = avail_json["scenes"].as_array().cloned().unwrap_or_default();
    let mut mapped_items: Vec<Value> = Vec::new();
    for sc in raw_scenes.into_iter().take(50) {
        let sc_id = sc["image_id"]
            .as_str()
            .or_else(|| sc["id"].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(scoped_item_id);
        let sc_dt = sc["acquisition_date"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback_dt);
        let cloud_cov = sc.get("cloud_cover").cloned().unwrap_or(Value::Null);
        mapped_items.push(json!({
            "id": sc_id,
            "title": sc["id"].as_str().unwrap_or(gee_asset_id),
            "collection": gee_asset_id,
            "datetime": sc_dt,
            "cloud_cover": cloud_cov,
            "bbox": [min_lon, min_lat, max_lon, max_lat],
            "bands": discovered_bands,
            "already_downloaded": already_in_explicit_dir,
            "local_file_path": local_path_val,
            "geometry": poly_geom,
        }));
    }

    if mapped_items.is_empty() {
        mapped_items.push(json!({
            "id": scoped_item_id,
            "title": gee_asset_id,
            "collection": gee_asset_id,
            "datetime": fallback_dt,
            "bbox": [min_lon, min_lat, max_lon, max_lat],
            "bands": discovered_bands,
            "already_downloaded": already_in_explicit_dir,
            "local_file_path": local_path_val,
            "geometry": poly_geom,
        }));
    }

    json!({
        "ok": true,
        "dataset": "gee_catalog",
        "gee_asset_id": gee_asset_id,
        "route_mode": "gee_official_rest",
        "expression_mode": expr_mode,
        "search_only": true,
        "count": mapped_items.len(),
        "total_scenes": total_scenes,
        "total_matching_scenes": cloud_filtered_scenes,
        "already_exists": already_in_explicit_dir,
        "target_item_id": scoped_item_id,
        "bands": discovered_bands,
        "mosaic_bbox": [min_lon, min_lat, max_lon, max_lat],
        "items": mapped_items,
    })
}

fn gee_expr_band_names(image_expr: &Value) -> Value {
    let mut values = image_expr["values"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let root_ref = image_expr["result"].as_str().unwrap_or("0").to_owned();
    values.insert(
        "bands_out".to_owned(),
        json!({
            "functionInvocationValue": {
                "functionName": "Image.bandNames",
                "arguments": {
                    "image": { "valueReference": root_ref }
                }
            }
        }),
    );
    json!({
        "result": "bands_out",
        "values": values
    })
}

fn extract_gee_http_error_summary(raw_body: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<Value>(raw_body) {
        if let Some(msg) = parsed["error"]["message"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let status_tag = parsed["error"]["status"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let clean_msg: String = msg
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .take(180)
                .collect();
            return match status_tag {
                Some(st) => format!("{st} - {clean_msg}"),
                None => clean_msg,
            };
        }
    }
    raw_body
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '{' | '}') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}

fn select_homogeneous_gee_bands(bands: &[String]) -> Option<Vec<String>> {
    if bands.is_empty() {
        return None;
    }
    let has = |b: &str| bands.iter().any(|x| x == b);
    for rgb in [
        ["B4", "B3", "B2"],
        ["SR_B4", "SR_B3", "SR_B2"],
        ["B04", "B03", "B02"],
    ] {
        if rgb.iter().all(|b| has(b)) {
            return Some(rgb.iter().map(|s| (*s).to_owned()).collect());
        }
    }
    for single in [
        "DEM",
        "elevation",
        "AVE_DSM",
        "DSM",
        "dtm",
        "dsm",
        "b1",
        "NDVI",
        "Map",
        "VV",
        "HH",
        "temperature_2m",
    ] {
        if has(single) {
            return Some(vec![single.to_owned()]);
        }
    }
    Some(vec![bands[0].clone()])
}

fn gee_fetch_band_names_for_expr(
    client: &reqwest::blocking::Client,
    api_base: &str,
    project_id: &str,
    bearer_token: &str,
    image_expr: &Value,
) -> Result<Vec<String>, (u16, String)> {
    let url = format!("{api_base}/v1/projects/{project_id}/value:compute");
    let payload = json!({
        "expression": gee_expr_band_names(image_expr)
    });
    let resp = client
        .post(&url)
        .bearer_auth(bearer_token)
        .json(&payload)
        .send()
        .map_err(|e| (0u16, format!("{e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let code = status.as_u16();
        let body = resp.text().unwrap_or_default();
        return Err((code, extract_gee_http_error_summary(&body)));
    }
    let val: Value = resp
        .json()
        .map_err(|e| (200u16, format!("解析波段元数据失败: {e}")))?;
    let bands = val["result"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(bands)
}

fn build_gee_compute_pixels_payload(
    expr: &Value,
    grid_spec: &Value,
    band_ids: Option<&[String]>,
) -> Value {
    let mut payload = json!({
        "expression": expr,
        "fileFormat": "GEO_TIFF",
        "grid": grid_spec
    });
    if let Some(bands) = band_ids {
        if !bands.is_empty() {
            payload["bandIds"] = json!(bands);
        }
    }
    payload
}

fn build_gee_map_payload(
    expr: &Value,
    band_ids: Option<&[String]>,
    min_val: f64,
    max_val: f64,
) -> Value {
    let mut payload = json!({
        "expression": expr,
        "fileFormat": "AUTO_JPEG_PNG",
        "visualizationOptions": {
            "ranges": [{ "min": min_val, "max": max_val }]
        }
    });
    if let Some(bands) = band_ids {
        if !bands.is_empty() {
            payload["bandIds"] = json!(bands);
        }
    }
    payload
}

pub(crate) fn resolve_gee_bearer_token(
    _app_data_dir: &Path,
    client: &reqwest::blocking::Client,
    secret: &str,
) -> Result<String, String> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return Err("GEE_CREDENTIAL_REQUIRED: GEE 访问凭据为空".to_owned());
    }
    if !trimmed.starts_with('{') {
        return Ok(trimmed.to_owned());
    }
    let sa: Value =
        serde_json::from_str(trimmed).map_err(|e| format!("INVALID_SERVICE_ACCOUNT_JSON: {e}"))?;
    if let Some(tok) = sa["access_token"]
        .as_str()
        .or_else(|| sa["token"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(tok.to_owned());
    }
    let token_uri = sa["token_uri"]
        .as_str()
        .unwrap_or("https://oauth2.googleapis.com/token")
        .trim();
    // Support Google ADC / Earth Engine CLI authorized_user JSON (refresh_token exchange)
    if let Some(refresh_token) = sa["refresh_token"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let client_id = sa["client_id"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                "INVALID_SA_JSON: authorized_user OAuth 凭据缺少 client_id".to_owned()
            })?;
        let client_secret = sa["client_secret"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                "INVALID_SA_JSON: authorized_user OAuth 凭据缺少 client_secret".to_owned()
            })?;
        let resp = client
            .post(token_uri)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", client_id),
                ("client_secret", client_secret),
            ])
            .send()
            .map_err(|err| format!("GEE_OAUTH_REFRESH_FAILED ({token_uri}): {err}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "GEE_OAUTH_REFRESH_FAILED: OAuth2 Refresh Token 端点返回 HTTP {}",
                resp.status()
            ));
        }
        let tok_json: Value = resp
            .json()
            .map_err(|err| format!("GEE_OAUTH_REFRESH_FAILED: 解析 Token 响应失败 ({err})"))?;
        return tok_json["access_token"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                "GEE_OAUTH_REFRESH_FAILED: OAuth2 响应中未包含 access_token".to_owned()
            });
    }
    let client_email = sa["client_email"].as_str().unwrap_or("").trim();
    let private_key = sa["private_key"].as_str().unwrap_or("").trim();
    if client_email.is_empty() || private_key.is_empty() {
        return Err(
            "GEE_SA_TOKEN_EXCHANGE_FAILED: Service Account JSON 缺少 client_email 或 private_key"
                .to_owned(),
        );
    }
    let now_dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let now = now_dur.as_secs().max(1_700_000_000);
    let header_b64 = base64url_encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = json!({
        "iss": client_email,
        "scope": "https://www.googleapis.com/auth/earthengine",
        "aud": token_uri,
        "iat": now,
        "exp": now + 3600,
    });
    let claims_b64 = base64url_encode(&serde_json::to_vec(&claims).unwrap_or_default());
    let signing_input = format!("{header_b64}.{claims_b64}");

    let sig_bytes = sign_rs256_jwt_pure_rust(private_key, signing_input.as_bytes()).map_err(|e| {
        format!("GEE_SA_TOKEN_EXCHANGE_FAILED: 服务账号 ({client_email}) 纯内存 RS256 签名失败 ({e})")
    })?;
    let sig_b64 = base64url_encode(&sig_bytes);
    let assertion = format!("{signing_input}.{sig_b64}");
    let resp = client
        .post(token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ])
        .send()
        .map_err(|err| format!("GEE_SA_TOKEN_EXCHANGE_FAILED ({token_uri}): {err}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "GEE_SA_TOKEN_EXCHANGE_FAILED: OAuth2 Token 端点返回 HTTP {}",
            resp.status()
        ));
    }
    let tok_json: Value = resp
        .json()
        .map_err(|err| format!("GEE_SA_TOKEN_EXCHANGE_FAILED: 解析 Token 响应失败 ({err})"))?;
    tok_json["access_token"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "GEE_SA_TOKEN_EXCHANGE_FAILED: OAuth2 响应中未包含 access_token".to_owned())
}

const MAX_SCIENCE_DOWNLOAD_BYTES: u64 = 20 * 1024 * 1024 * 1024;

fn stream_http_to_file_verified(
    mut resp: reqwest::blocking::Response,
    out_file: &Path,
    max_bytes: u64,
    context_tag: &str,
) -> Result<u64, String> {
    use std::io::{Read, Write};
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("{context_tag}: HTTP {status}"));
    }
    let expected_len = resp.content_length();
    if let Some(cl) = expected_len {
        if cl == 0 {
            return Err(format!("{context_tag}: 远端响应长度为 0 字节"));
        }
        if cl > max_bytes {
            return Err(format!(
                "{context_tag}: PAYLOAD_TOO_LARGE (Content-Length {cl} bytes 超过上限 {max_bytes} bytes)"
            ));
        }
    }
    if let Some(parent) = out_file.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{context_tag}: 创建目标目录失败 ({e})"))?;
    }
    let mut file =
        fs::File::create(out_file).map_err(|e| format!("{context_tag}: 创建文件失败 ({e})"))?;
    let mut buf = [0u8; 65536];
    let mut written: u64 = 0;
    loop {
        let n = resp.read(&mut buf).map_err(|e| {
            let _ = fs::remove_file(out_file);
            format!("{context_tag}: 网络流读取中断 ({e})")
        })?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > max_bytes {
            drop(file);
            let _ = fs::remove_file(out_file);
            return Err(format!(
                "{context_tag}: PAYLOAD_TOO_LARGE (传输字节数 {written} 超过单文件上限 {max_bytes} bytes)"
            ));
        }
        file.write_all(&buf[..n]).map_err(|e| {
            let _ = fs::remove_file(out_file);
            format!("{context_tag}: 写入磁盘失败 ({e})")
        })?;
    }
    file.flush()
        .map_err(|e| format!("{context_tag}: 刷新文件缓冲失败 ({e})"))?;
    drop(file);
    if written == 0 {
        let _ = fs::remove_file(out_file);
        return Err(format!("{context_tag}: 下载内容为空"));
    }
    if let Some(cl) = expected_len {
        if written != cl {
            let _ = fs::remove_file(out_file);
            return Err(format!(
                "{context_tag}: INCOMPLETE_TRANSFER (仅接收 {written}/{cl} bytes)"
            ));
        }
    }
    Ok(written)
}

fn attach_raster_preview_fields(target: &mut Value, raster_path: &Path) {
    if !raster_path.is_file() {
        return;
    }
    let engine = discover_native_gis_engine();
    if !engine.is_discovered() {
        return;
    }
    if let Ok((png_bytes, wgs84_bbox)) = engine.render_raster_preview_png(raster_path, 512) {
        if !png_bytes.is_empty() {
            let data_url = format!(
                "data:image/png;base64,{}",
                base64_standard_encode(&png_bytes)
            );
            target["preview_data_url"] = json!(data_url);
            target["mosaic_bbox"] = json!(wgs84_bbox);
        }
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn calculate_day_of_year(year: i32, month: u32, day: u32) -> Result<u32, String> {
    if !(1..=12).contains(&month) {
        return Err(format!("INVALID_DATE_MONTH: 月份 {month} 无效"));
    }
    let days_in_months = [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let max_day = days_in_months[(month - 1) as usize];
    if day == 0 || day > max_day {
        return Err(format!(
            "INVALID_DATE_DAY: 日期 {year}-{month:02}-{day:02} 无效"
        ));
    }
    let mut doy = day;
    for m in 1..month {
        doy += days_in_months[(m - 1) as usize];
    }
    Ok(doy)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = month as u64;
    let d = day as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + (doe as i64) - 719468
}

fn gps_week_and_dow(year: i32, month: u32, day: u32) -> Result<(u32, u32), String> {
    let _ = calculate_day_of_year(year, month, day)?;
    // GPS Epoch is 1980-01-06 00:00:00 GPST (3657 days after 1970-01-01)
    let days_since_gps_epoch = days_from_civil(year, month, day) - 3657;
    if days_since_gps_epoch < 0 {
        return Err(format!(
            "IONEX_DATE_BEFORE_GPS_EPOCH: {year}-{month:02}-{day:02} 早于 GPS 起始历元 (1980-01-06)"
        ));
    }
    Ok((
        (days_since_gps_epoch / 7) as u32,
        (days_since_gps_epoch % 7) as u32,
    ))
}

/// Real RFC / Unix `compress` (`.Z`) LZW decompressor:
/// - Validates magic header `0x1f 0x9d`
/// - Supports `max_bits` `9..=16` and block-mode flag `0x80` (clear code `256`)
/// - Unpacks LSB-first variable-width codes in `n_bits`-byte aligned chunks per BSD/ncompress spec
pub fn decompress_unix_lzw_z(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 3 || bytes[0] != 0x1f || bytes[1] != 0x9d {
        return Err("LZW_INVALID_MAGIC: 输入非标准 Unix compress (.Z) 头部 (0x1f 0x9d)".to_owned());
    }
    let flags = bytes[2];
    let max_bits = (flags & 0x1f) as usize;
    let block_mode = (flags & 0x80) != 0;
    if !(9..=16).contains(&max_bits) {
        return Err(format!(
            "LZW_UNSUPPORTED_MAX_BITS: max_bits={max_bits} 超出标准范围 9..=16"
        ));
    }

    const CLEAR: usize = 256;
    const MAX_OUT_BYTES: usize = 64 * 1024 * 1024;
    let max_maxcode: usize = 1usize << max_bits;
    let maxcode_for = |nb: usize| -> usize {
        if nb >= max_bits {
            max_maxcode
        } else {
            (1usize << nb) - 1
        }
    };

    let mut tab_prefix = vec![0u16; max_maxcode];
    let mut tab_suffix = vec![0u8; max_maxcode];
    for (c, slot) in tab_suffix.iter_mut().enumerate().take(256usize) {
        *slot = c as u8;
    }

    let mut n_bits: usize = 9;
    let mut maxcode = maxcode_for(n_bits);
    let mut free_ent: usize = if block_mode { 257 } else { 256 };
    let mut clear_flg = false;

    let data = &bytes[3..];
    if data.is_empty() {
        return Ok(Vec::new());
    }

    let mut pos: usize = 0;
    let mut gbuf = [0u8; 16];
    let mut bit_offset: usize = 0;
    let mut bit_size: usize = 0;

    let mut next_code = |n_bits: &mut usize,
                         maxcode: &mut usize,
                         clear_flg: &mut bool,
                         free_ent: usize|
     -> Option<usize> {
        if *clear_flg || bit_offset >= bit_size || free_ent > *maxcode {
            if free_ent > *maxcode {
                *n_bits += 1;
                *maxcode = maxcode_for(*n_bits);
            }
            if *clear_flg {
                *n_bits = 9;
                *maxcode = maxcode_for(9);
                *clear_flg = false;
            }
            if pos >= data.len() {
                return None;
            }
            let take = (*n_bits).min(data.len() - pos);
            gbuf[..take].copy_from_slice(&data[pos..pos + take]);
            for slot in &mut gbuf[take..(*n_bits).min(16)] {
                *slot = 0;
            }
            pos += take;
            bit_offset = 0;
            let total_bits = take * 8;
            bit_size = total_bits - (total_bits % *n_bits);
            if bit_size == 0 {
                return None;
            }
        }
        let r_off = bit_offset;
        let bp = r_off >> 3;
        let r_shift = r_off & 7;
        let raw = (gbuf.get(bp).copied().unwrap_or(0) as u32)
            | ((gbuf.get(bp + 1).copied().unwrap_or(0) as u32) << 8)
            | ((gbuf.get(bp + 2).copied().unwrap_or(0) as u32) << 16);
        let mask = (1u32 << *n_bits) - 1;
        let code = ((raw >> r_shift) & mask) as usize;
        bit_offset += *n_bits;
        Some(code)
    };

    let mut out = Vec::with_capacity(data.len().saturating_mul(4).min(8 * 1024 * 1024));
    let mut stack = Vec::with_capacity(4096);

    let Some(first_code) = next_code(&mut n_bits, &mut maxcode, &mut clear_flg, free_ent) else {
        return Ok(out);
    };
    if first_code >= 256 {
        return Err(format!(
            "LZW_CORRUPT_FIRST_CODE: 首个符号 {first_code} 超出字面量范围 0..255"
        ));
    }
    let mut finchar = first_code as u8;
    let mut oldcode = first_code;
    out.push(finchar);

    while let Some(mut code) = next_code(&mut n_bits, &mut maxcode, &mut clear_flg, free_ent) {
        if code == CLEAR && block_mode {
            tab_prefix[..256].fill(0);
            clear_flg = true;
            free_ent = CLEAR;
            let Some(c_after_clear) =
                next_code(&mut n_bits, &mut maxcode, &mut clear_flg, free_ent)
            else {
                break;
            };
            if c_after_clear >= 256 {
                return Err(format!(
                    "LZW_CORRUPT_AFTER_CLEAR: CLEAR 后首个符号 {c_after_clear} 非字面量"
                ));
            }
            free_ent = CLEAR + 1;
            finchar = c_after_clear as u8;
            oldcode = c_after_clear;
            out.push(finchar);
            continue;
        }

        let incode = code;
        stack.clear();
        if code >= free_ent {
            if code > free_ent {
                return Err(format!(
                    "LZW_CORRUPT_STREAM: code={code} > free_ent={free_ent}"
                ));
            }
            stack.push(finchar);
            code = oldcode;
        }

        while code >= 256 {
            if stack.len() >= max_maxcode {
                return Err("LZW_CORRUPT_CYCLE: 字典前缀链出现环路".to_owned());
            }
            stack.push(tab_suffix[code]);
            code = tab_prefix[code] as usize;
        }
        finchar = tab_suffix[code];
        stack.push(finchar);

        if out.len() + stack.len() > MAX_OUT_BYTES {
            return Err("LZW_OUTPUT_TOO_LARGE: 解压后体积超过 64 MB 安全上限".to_owned());
        }
        for &b in stack.iter().rev() {
            out.push(b);
        }

        if free_ent < max_maxcode {
            tab_prefix[free_ent] = oldcode as u16;
            tab_suffix[free_ent] = finchar;
            free_ent += 1;
        }
        oldcode = incode;
    }

    Ok(out)
}

fn decompress_gzip_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Err("GZIP_INVALID_MAGIC: 输入非标准 GZIP 头部 (0x1f 0x8b)".to_owned());
    }
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("GZIP_DECOMPRESS_FAILED: {e}"))?;
    Ok(out)
}

fn extract_ionex_dcb_block(ionex_text: &str) -> Option<String> {
    let mut in_dcb_aux = false;
    let mut dcb_lines = Vec::new();
    for line in ionex_text.lines() {
        if line.contains("START OF AUX DATA") && line.contains("DIFFERENTIAL CODE BIASES") {
            in_dcb_aux = true;
            dcb_lines.push(line);
            continue;
        }
        if in_dcb_aux {
            dcb_lines.push(line);
            if line.contains("END OF AUX DATA") {
                break;
            }
        }
    }
    if dcb_lines.len() >= 3 {
        Some(dcb_lines.join("\n") + "\n")
    } else {
        None
    }
}

fn parse_ionex_maps_count(ionex_text: &str) -> usize {
    for line in ionex_text.lines() {
        if line.contains("# OF MAPS IN FILE") {
            let head = if line.len() > 60 { &line[..60] } else { line };
            if let Some(n) = head
                .split_whitespace()
                .next()
                .and_then(|t| t.parse::<usize>().ok())
            {
                if n > 0 {
                    return n;
                }
            }
        }
        if line.contains("END OF HEADER") {
            break;
        }
    }
    ionex_text.matches("START OF TEC MAP").count()
}

fn filter_gem15_live_tiles_with_probe(
    raw_tiles: &[Value],
    band_label: &str,
    probe: &mut dyn FnMut(&str) -> Result<bool, String>,
) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    for tile in raw_tiles {
        let Some(url) = tile.get("primary_url").and_then(Value::as_str) else {
            continue;
        };
        if !probe(url)? {
            continue;
        }
        let tile_id = tile["tile_id"].as_str().unwrap_or("gem15_tile");
        let tile_bb = tile["tile_bbox"].clone();
        let vsicurl = tile["vsicurl_path"].as_str().unwrap_or("");
        let fname = format!("{tile_id}.tif");
        let geom = if let Some(arr) = tile_bb.as_array().filter(|a| a.len() >= 4) {
            let w = arr[0].as_f64().unwrap_or(0.0);
            let s = arr[1].as_f64().unwrap_or(0.0);
            let e = arr[2].as_f64().unwrap_or(0.0);
            let n = arr[3].as_f64().unwrap_or(0.0);
            json!({
                "type": "Polygon",
                "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]
            })
        } else {
            Value::Null
        };
        items.push(json!({
            "id": tile_id,
            "title": format!("Google GEM-15 v1 ({band_label} · {tile_id})"),
            "filename": fname,
            "band": band_label,
            "datetime": "2024-01-01T00:00:00Z",
            "provider": "Google Cloud Public COG (GEM-15 v1)",
            "resolution_m": 15,
            "license": "CC BY 4.0",
            "bbox": tile_bb,
            "geometry": geom,
            "href": url,
            "download_url": url,
            "vsicurl_path": vsicurl,
        }));
    }
    Ok(items)
}

fn execute_gem15_public_gcs_query(
    app_data_dir: &Path,
    req: &OpenGeodataQueryRequest,
    bbox: [f64; 4],
    kw_download: bool,
    kw_band: Option<&str>,
) -> Result<Value, String> {
    let band_str = kw_band.unwrap_or("dtm").trim();
    let is_dsm = band_str.to_ascii_lowercase().contains("dsm");
    let product = if is_dsm { "gem15_dsm" } else { "gem15_dtm" };
    let band_label = if is_dsm { "DSM" } else { "DTM" };
    let resolved = resolve_open_dem_tiles(product, bbox)?;
    let raw_tiles = resolved["tiles"].as_array().cloned().unwrap_or_default();
    let probe_client = build_http_client(app_data_dir)?;
    let items = filter_gem15_live_tiles_with_probe(&raw_tiles, band_label, &mut |url| {
        let resp = probe_client
            .head(url)
            .send()
            .map_err(|e| format!("GEM15_GCS_PROBE_FAILED: {e}"))?;
        let st = resp.status().as_u16();
        if resp.status().is_success() {
            Ok(true)
        } else if st == 404 {
            Ok(false)
        } else {
            Err(format!("GEM15_GCS_HTTP_ERROR: HTTP {st}"))
        }
    })?;

    let should_download = (kw_download || req.tool_name.as_deref() == Some("download"))
        && !has_search_only_token(req.keyword.as_deref());
    let mut downloaded_file: Option<String> = None;
    let mut downloaded_bytes: u64 = 0;
    let mut target_item_id_out: Option<String> = None;
    let mut already_exists = false;

    if should_download {
        let target_item = if let Some(tid) = req
            .osm_tag
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            items
                .iter()
                .find(|it| it["id"].as_str() == Some(tid))
                .or_else(|| items.first())
                .ok_or_else(|| {
                    "GEM15_OUT_OF_COVERAGE: Google GEM-15 v1 公开目录在当前研究区无可用瓦片（v1 官方范围不含中国大陆、俄罗斯、蒙古、格陵兰、南极及纯海域）。".to_owned()
                })?
        } else {
            items.first().ok_or_else(|| {
                "GEM15_OUT_OF_COVERAGE: Google GEM-15 v1 公开目录在当前研究区无可用瓦片（v1 官方范围不含中国大陆、俄罗斯、蒙古、格陵兰、南极及纯海域）。".to_owned()
            })?
        };
        let item_id = target_item["id"]
            .as_str()
            .unwrap_or("gem15_tile")
            .to_owned();
        let fname = target_item["filename"]
            .as_str()
            .unwrap_or("gem15_tile.tif")
            .to_owned();
        let dl_url = target_item["download_url"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        target_item_id_out = Some(item_id.clone());

        let out_dir = req
            .output_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join("GEM15"));
        fs::create_dir_all(&out_dir).map_err(|e| {
            format!(
                "CREATE_OUTPUT_DIR_FAILED ({}): {e}",
                out_dir.to_string_lossy()
            )
        })?;
        let dest = out_dir.join(&fname);
        if dest.is_file() && dest.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            already_exists = true;
            downloaded_bytes = dest.metadata().map(|m| m.len()).unwrap_or(0);
            downloaded_file = Some(dest.to_string_lossy().to_string());
        } else {
            let dl_client = build_download_http_client(app_data_dir)?;
            let resp = http_get_with_retry(&dl_client, &dl_url, None, 3)
                .map_err(|e| format!("GEM15_DOWNLOAD_FAILED ({dl_url}): {e}"))?
                .error_for_status()
                .map_err(|e| format!("GEM15_HTTP_ERROR ({dl_url}): {e}"))?;
            let tmp_path = out_dir.join(format!("{fname}.download.tmp"));
            let _ = fs::remove_file(&tmp_path);
            let written = stream_http_to_file_verified(
                resp,
                &tmp_path,
                2 * 1024 * 1024 * 1024,
                &format!("GEM15_DOWNLOAD ({item_id})"),
            )?;
            let head = read_prefix_bytes(&tmp_path, 8)?;
            if !is_valid_tiff_magic(&head) {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!(
                    "GEM15_INVALID_TIFF: 下载的 {item_id} 非有效 GeoTIFF 文件"
                ));
            }
            fs::rename(&tmp_path, &dest).map_err(|e| {
                let _ = fs::remove_file(&tmp_path);
                format!("RENAME_FAILED: {e}")
            })?;
            downloaded_bytes = written;
            downloaded_file = Some(dest.to_string_lossy().to_string());
        }
    }

    let empty_reason_zh = if items.is_empty() {
        Some("Google GEM-15 v1 公开目录在当前研究区无可用瓦片（v1 官方范围不含中国大陆、俄罗斯、蒙古、格陵兰、南极及纯海域）。".to_owned())
    } else {
        None
    };

    Ok(json!({
        "ok": true,
        "dataset": "gee_catalog",
        "gee_asset_id": "projects/climate-and-sustainability/assets/gem-15_v1",
        "route_mode": "cloud_native_direct",
        "provider": "Google Cloud Public COG (GEM-15 v1)",
        "count": items.len(),
        "empty_reason_zh": empty_reason_zh,
        "already_exists": already_exists,
        "target_item_id": target_item_id_out,
        "downloaded_file": downloaded_file,
        "downloaded_path": downloaded_file,
        "downloaded_bytes": downloaded_bytes,
        "items": items,
    }))
}

fn map_canonical_gee_asset(
    gee_asset_id: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    match gee_asset_id {
        "ESA/WorldCover/v200" => Some(("esa_worldcover", "esa-worldcover", "v200")),
        "ESA/WorldCover/v100" => Some(("esa_worldcover", "esa-worldcover", "v100")),
        "JRC/GSW1_4/GlobalSurfaceWater" => Some(("jrc_gsw", "jrc-gsw", "occurrence")),
        "JAXA/ALOS/PALSAR/YEARLY/SAR_EPOCH" | "JAXA/ALOS/PALSAR/YEARLY/SAR" => {
            Some(("alos_palsar_mosaic", "alos-palsar-mosaic", "HH"))
        }
        "MODIS/061/MOD13Q1" => Some(("modis_ndvi", "modis-13Q1-061", "250m_16_days_NDVI")),
        "COPERNICUS/S5P/OFFL/L3_NO2" | "COPERNICUS/S5P/NRTI/L3_NO2" => {
            Some(("sentinel_5p", "sentinel-5p-l2-netcdf", "no2"))
        }
        "COPERNICUS/S2_SR_HARMONIZED" => Some(("mpc_stac_science", "sentinel-2-l2a", "visual")),
        "COPERNICUS/S2_HARMONIZED" => Some(("mpc_stac_science", "sentinel-2-l2a", "visual")),
        "GOOGLE/DYNAMICWORLD/V1" => Some(("mpc_stac_science", "io-lulc-annual-v02", "data")),
        "COPERNICUS/S1_GRD" => Some(("mpc_stac_science", "sentinel-1-grd", "vv")),
        "LANDSAT/LC08/C02/T1_L2" | "LANDSAT/LC09/C02/T1_L2" => {
            Some(("mpc_stac_science", "landsat-c2-l2", "red"))
        }
        "COPERNICUS/DEM/GLO30" => Some(("mpc_stac_science", "cop-dem-glo-30", "data")),
        "NASA/NASADEM_HGT/001" => Some(("mpc_stac_science", "nasadem", "elevation")),
        "ECMWF/ERA5_LAND/HOURLY" => Some(("open_meteo_era5", "era5-land", "hourly_insar")),
        _ => None,
    }
}

fn resolve_mpc_tilejson_template(
    app_data_dir: &Path,
    client: &reqwest::blocking::Client,
    collection: &str,
    preferred_asset: &str,
    bbox_opt: Option<[f64; 4]>,
) -> Result<(String, String), String> {
    let mpc_base = read_loopback_mpc_base_url(app_data_dir)
        .unwrap_or_else(|| "https://planetarycomputer.microsoft.com".to_owned());
    let search_url = format!("{mpc_base}/api/stac/v1/search");
    let bbox = bbox_opt.unwrap_or([117.15, 31.75, 117.35, 31.95]);
    let search_body = json!({
        "collections": [collection],
        "bbox": [
            bbox[0].min(bbox[2]),
            bbox[1].min(bbox[3]),
            bbox[0].max(bbox[2]),
            bbox[1].max(bbox[3])
        ],
        "limit": 1
    });
    let mut stac_resp_opt: Option<Value> = None;
    let mut last_stac_err = String::new();
    for attempt in 0..3 {
        match client.post(&search_url).json(&search_body).send() {
            Ok(resp) if resp.status().is_success() => match resp.json::<Value>() {
                Ok(v) => {
                    stac_resp_opt = Some(v);
                    break;
                }
                Err(e) => {
                    last_stac_err =
                        format!("MPC_STAC_TILEJSON_SEARCH_PARSE_FAILED ({collection}): {e}")
                }
            },
            Ok(resp) => {
                last_stac_err = format!(
                    "MPC_STAC_TILEJSON_SEARCH_HTTP_ERROR ({collection}): HTTP {}",
                    resp.status()
                );
                if !resp.status().is_server_error() && resp.status().as_u16() != 429 {
                    break;
                }
            }
            Err(e) => {
                last_stac_err = format!("MPC_STAC_TILEJSON_SEARCH_FAILED ({collection}): {e}")
            }
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(800));
        }
    }
    let stac_resp = stac_resp_opt.ok_or(last_stac_err)?;

    let feat = stac_resp["features"]
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| format!("MPC_TILEJSON_NO_SCENES: 区域内未检索到 {collection} 瓦片景"))?;
    let item_id = feat["id"].as_str().unwrap_or("").trim();
    if item_id.is_empty() {
        return Err(format!(
            "MPC_TILEJSON_INVALID_ITEM: {collection} STAC Item ID 为空"
        ));
    }

    let tilejson_url = if let Some(href) = feat["assets"]["tilejson"]["href"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        href.to_owned()
    } else {
        let mut chosen_asset = preferred_asset;
        if let Some(assets_obj) = feat["assets"].as_object() {
            if !assets_obj.contains_key(chosen_asset) {
                for candidate in [
                    "visual",
                    "map",
                    "data",
                    "elevation",
                    "occurrence",
                    "HH",
                    "vv",
                    "red",
                    "250m_16_days_NDVI",
                ] {
                    if assets_obj.contains_key(candidate) {
                        chosen_asset = candidate;
                        break;
                    }
                }
            }
        }
        format!(
            "{mpc_base}/api/data/v1/item/tilejson.json?collection={collection}&item={item_id}&assets={chosen_asset}&format=png"
        )
    };

    let tj_json: Value = client
        .get(&tilejson_url)
        .send()
        .map_err(|e| format!("MPC_TILEJSON_FETCH_FAILED ({tilejson_url}): {e}"))?
        .error_for_status()
        .map_err(|e| format!("MPC_TILEJSON_HTTP_ERROR ({tilejson_url}): {e}"))?
        .json()
        .map_err(|e| format!("MPC_TILEJSON_PARSE_FAILED ({tilejson_url}): {e}"))?;

    let tile_tpl = tj_json["tiles"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| s.contains("{z}/{x}/{y}"))
        .ok_or_else(|| format!("MPC_TILEJSON_MISSING_TILES_ARRAY ({tilejson_url})"))?
        .to_owned();

    Ok((tile_tpl, item_id.to_owned()))
}

fn select_open_eo_target_item<'a>(
    items: &'a [Value],
    requested_id: Option<&str>,
) -> Result<&'a Value, String> {
    if let Some(target_id) = requested_id.map(str::trim).filter(|s| !s.is_empty()) {
        items
            .iter()
            .find(|it| {
                it["id"].as_str() == Some(target_id)
                    || it["granule_id"].as_str() == Some(target_id)
                    || it["producer_granule_id"].as_str() == Some(target_id)
            })
            .ok_or_else(|| {
                format!(
                    "TARGET_GRANULE_NOT_FOUND: 未在当前检索结果中找到指定景 {target_id}（已拒绝自动替换为其他影像），请重新检索后再下载"
                )
            })
    } else {
        items
            .iter()
            .find(|it| !it["href"].as_str().unwrap_or("").is_empty())
            .or_else(|| items.first())
            .ok_or_else(|| "OPEN_EO_NO_GRANULES: 未找到可下载的目标景".to_owned())
    }
}

fn has_search_only_token(keyword: Option<&str>) -> bool {
    let Some(raw) = keyword.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let mut saw_search = false;
    for token in raw.split('|').map(str::trim).filter(|s| !s.is_empty()) {
        if token.eq_ignore_ascii_case("download") {
            return false;
        }
        if token.eq_ignore_ascii_case("search") || token.eq_ignore_ascii_case("preview") {
            saw_search = true;
        }
    }
    saw_search
}

fn parse_orbit_filter_token(keyword: Option<&str>) -> Option<&'static str> {
    let raw = keyword.map(str::trim).filter(|s| !s.is_empty())?;
    for token in raw.split('|').map(str::trim) {
        if token.eq_ignore_ascii_case("ASCENDING") || token.eq_ignore_ascii_case("ASC") {
            return Some("ASCENDING");
        }
        if token.eq_ignore_ascii_case("DESCENDING") || token.eq_ignore_ascii_case("DESC") {
            return Some("DESCENDING");
        }
    }
    None
}

pub(crate) fn infer_orbit_direction_from_granule_id(granule_id: &str) -> Option<&'static str> {
    let upper = granule_id.trim().to_ascii_uppercase();
    if upper.is_empty() {
        return None;
    }
    if upper.starts_with("NISAR_") {
        let parts: Vec<&str> = upper.split('_').collect();
        if let Some(tok) = parts.get(6).copied() {
            if tok == "A" || tok == "ASC" || tok == "ASCENDING" {
                return Some("ASCENDING");
            }
            if tok == "D" || tok == "DESC" || tok == "DESCENDING" {
                return Some("DESCENDING");
            }
        }
    }
    if upper.starts_with("S1-GUNW-") {
        let parts: Vec<&str> = upper.split('-').collect();
        if let Some(tok) = parts.get(2).copied() {
            if tok == "A" {
                return Some("ASCENDING");
            }
            if tok == "D" {
                return Some("DESCENDING");
            }
        }
    }
    if upper.contains("_D_") {
        return Some("DESCENDING");
    }
    if upper.contains("_A_") {
        return Some("ASCENDING");
    }
    None
}

pub(crate) fn parse_bhoonidhi_orbit_direction(
    props: &Value,
    granule_id: &str,
) -> Option<&'static str> {
    for key in [
        "Node",
        "node",
        "sat:orbit_state",
        "flight_direction",
        "orbit_direction",
        "AscendingDescending",
        "PASSDIRECTION",
    ] {
        if let Some(val) = props.get(key).and_then(Value::as_str) {
            let trimmed = val.trim().to_ascii_uppercase();
            if trimmed == "A" || trimmed.starts_with("ASC") {
                return Some("ASCENDING");
            }
            if trimmed == "D" || trimmed.starts_with("DESC") {
                return Some("DESCENDING");
            }
        }
    }
    infer_orbit_direction_from_granule_id(granule_id)
}

fn existing_valid_science_file_bytes(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() <= 64 {
        return None;
    }
    let head = {
        use std::io::Read;
        let mut f = fs::File::open(path).ok()?;
        let mut buf = [0u8; 256];
        let n = f.read(&mut buf).ok()?;
        buf[..n].to_vec()
    };
    let sample = String::from_utf8_lossy(&head)
        .trim_start()
        .to_ascii_lowercase();
    if sample.starts_with("<!doctype") || sample.starts_with("<html") {
        return None;
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "tif" || ext == "tiff" {
        let valid_tiff = head.starts_with(b"II*\0")
            || head.starts_with(b"MM\0*")
            || head.starts_with(b"II+\0")
            || head.starts_with(b"MM\0+");
        if !valid_tiff {
            return None;
        }
    }
    Some(meta.len())
}

fn fetch_preview_png_data_url(client: &reqwest::blocking::Client, url: &str) -> Option<String> {
    if url.trim().is_empty() {
        return None;
    }
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(6))
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().ok()?;
    if bytes.len() < 16 || bytes.len() > 2_500_000 {
        return None;
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(format!(
            "data:image/png;base64,{}",
            base64_standard_encode(&bytes)
        ))
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(format!(
            "data:image/jpeg;base64,{}",
            base64_standard_encode(&bytes)
        ))
    } else {
        None
    }
}

const GEODATA_PREVIEW_ALLOWED_HOSTS: &[&str] = &[
    "sentinel-s1-l1c.s3.eu-central-1.amazonaws.com",
    "sentinel-s1-l1c.s3.amazonaws.com",
];
const GEODATA_PREVIEW_CACHE_MAX_ENTRIES: usize = 64;
const GEODATA_PREVIEW_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
struct GeodataPreviewCache {
    entries: HashMap<String, String>,
    order: VecDeque<String>,
    total_bytes: usize,
}

impl GeodataPreviewCache {
    fn get(&self, key: &str) -> Option<String> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: String, data_url: String) {
        let new_bytes = data_url.len();
        if new_bytes > GEODATA_PREVIEW_CACHE_MAX_BYTES {
            return;
        }
        if let Some(prev) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(prev.len());
            self.order.retain(|existing| existing != &key);
        }
        while self.entries.len() >= GEODATA_PREVIEW_CACHE_MAX_ENTRIES
            || self.total_bytes.saturating_add(new_bytes) > GEODATA_PREVIEW_CACHE_MAX_BYTES
        {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(removed.len());
            }
        }
        self.total_bytes = self.total_bytes.saturating_add(new_bytes);
        self.order.push_back(key.clone());
        self.entries.insert(key, data_url);
    }
}

static GEODATA_PREVIEW_CACHE: OnceLock<Mutex<GeodataPreviewCache>> = OnceLock::new();

fn geodata_preview_cache() -> &'static Mutex<GeodataPreviewCache> {
    GEODATA_PREVIEW_CACHE.get_or_init(|| Mutex::new(GeodataPreviewCache::default()))
}

pub fn validate_allowlisted_geodata_preview_url(raw: &str) -> Result<reqwest::Url, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("PREVIEW_URL_EMPTY".to_owned());
    }
    let parsed =
        reqwest::Url::parse(trimmed).map_err(|err| format!("PREVIEW_URL_INVALID: {err}"))?;
    if parsed.scheme() != "https" {
        return Err("PREVIEW_URL_SCHEME_REJECTED".to_owned());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("PREVIEW_URL_CREDENTIALS_REJECTED".to_owned());
    }
    if parsed.port().is_some() {
        return Err("PREVIEW_URL_PORT_REJECTED".to_owned());
    }
    let host = parsed
        .host_str()
        .map(|h| h.to_ascii_lowercase())
        .unwrap_or_default();
    if !GEODATA_PREVIEW_ALLOWED_HOSTS.contains(&host.as_str()) {
        return Err(format!("PREVIEW_URL_HOST_NOT_ALLOWLISTED: {host}"));
    }
    Ok(parsed)
}

pub fn fetch_allowlisted_preview_data_url(
    app_data_dir: &Path,
    raw_url: &str,
) -> Result<String, String> {
    let validated = validate_allowlisted_geodata_preview_url(raw_url)?;
    let key = validated.as_str().to_owned();
    if let Ok(guard) = geodata_preview_cache().lock() {
        if let Some(hit) = guard.get(&key) {
            return Ok(hit);
        }
    }
    let builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(10))
        // Reject redirects at the transport boundary so an allowlisted host
        // cannot bounce to an internal or untrusted destination.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("EOStage-GeoDataTools/1.0");
    let (builder, _) = apply_geodata_client_proxy(builder, app_data_dir, false)?;
    let client = builder
        .build()
        .map_err(|err| format!("PREVIEW_HTTP_CLIENT_FAILED: {err}"))?;
    let data_url = fetch_preview_png_data_url(&client, &key)
        .ok_or_else(|| "PREVIEW_FETCH_FAILED".to_owned())?;
    if let Ok(mut guard) = geodata_preview_cache().lock() {
        guard.insert(key, data_url.clone());
    }
    Ok(data_url)
}

struct EarthdataAuthMaterials {
    bearer: Option<String>,
    basic: Option<(String, String)>,
}

fn resolve_earthdata_auth_materials(auth_err_msg: &str) -> Result<EarthdataAuthMaterials, String> {
    let force_no_auth = {
        #[cfg(test)]
        {
            std::env::var("EOSTAGE_FORCE_NO_EARTHDATA_AUTH")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
        }
        #[cfg(not(test))]
        {
            false
        }
    };
    if force_no_auth {
        return Err(auth_err_msg.to_owned());
    }

    let bearer = insar_download::resolve_earthdata_search_token()
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            insar_download::reveal_desktop_credential_field(
                insar_download::DesktopCredentialField::EarthdataToken,
            )
            .ok()
            .flatten()
            .map(|z| z.trim().to_owned())
            .filter(|s| !s.is_empty())
        });
    let basic = match (
        insar_download::reveal_desktop_credential_field(
            insar_download::DesktopCredentialField::EarthdataUsername,
        ),
        insar_download::reveal_desktop_credential_field(
            insar_download::DesktopCredentialField::EarthdataPassword,
        ),
    ) {
        (Ok(Some(u)), Ok(Some(p))) if !u.trim().is_empty() && !p.is_empty() => {
            Some((u.trim().to_owned(), p.to_string()))
        }
        _ => match (
            std::env::var("INSAR_EARTHDATA_USERNAME"),
            std::env::var("INSAR_EARTHDATA_PASSWORD"),
        ) {
            (Ok(u), Ok(p)) if !u.trim().is_empty() && !p.is_empty() => {
                Some((u.trim().to_owned(), p))
            }
            _ => None,
        },
    };
    if bearer.is_none() && basic.is_none() {
        return Err(auth_err_msg.to_owned());
    }
    Ok(EarthdataAuthMaterials { bearer, basic })
}

fn build_edl_manual_redirect_client(
    app_data_dir: &Path,
) -> Result<reqwest::blocking::Client, String> {
    let edl_builder = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(1800))
        .user_agent("EOStage-GeoDataTools/1.0");
    let (edl_builder, _) = apply_geodata_client_proxy(edl_builder, app_data_dir, false)?;
    edl_builder
        .build()
        .map_err(|e| format!("EDL_HTTP_CLIENT_INIT_FAILED: {e}"))
}

fn execute_edl_manual_get(
    edl_client: &reqwest::blocking::Client,
    data_url: &str,
    auth: &EarthdataAuthMaterials,
    auth_err_msg: &str,
) -> Result<reqwest::blocking::Response, String> {
    let mut current_url = data_url.to_owned();
    for _hop in 0..10 {
        let parsed_url = reqwest::Url::parse(&current_url)
            .map_err(|e| format!("INVALID_DATA_URL ({current_url}): {e}"))?;
        let host = parsed_url.host_str().unwrap_or("").to_ascii_lowercase();
        let is_presigned_s3 = current_url.contains("X-Amz-Signature=")
            || current_url.contains("x-amz-signature=")
            || host.ends_with(".amazonaws.com");
        let is_edl_host = host == "urs.earthdata.nasa.gov";
        let is_nasa_or_asf = host.ends_with(".nasa.gov") || host.ends_with(".alaska.edu");

        let mut rb = edl_client.get(parsed_url.clone());
        if !is_presigned_s3 {
            if is_edl_host {
                if let Some((ref u, ref p)) = auth.basic {
                    rb = rb.basic_auth(u, Some(p));
                } else if let Some(ref tok) = auth.bearer {
                    rb = rb.bearer_auth(tok);
                }
            } else if is_nasa_or_asf {
                if let Some(ref tok) = auth.bearer {
                    rb = rb.bearer_auth(tok);
                } else if let Some((ref u, ref p)) = auth.basic {
                    rb = rb.basic_auth(u, Some(p));
                }
            }
        }

        let resp = rb
            .send()
            .map_err(|e| format!("EARTHDATA_DOWNLOAD_REQUEST_FAILED ({current_url}): {e}"))?;
        let st = resp.status();
        if st.is_redirection() {
            let Some(loc) = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            else {
                return Err(auth_err_msg.to_owned());
            };
            current_url = parsed_url
                .join(loc)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| loc.to_owned());
            continue;
        }
        if st.as_u16() == 401 || st.as_u16() == 403 {
            return Err(auth_err_msg.to_owned());
        }
        return Ok(resp);
    }
    Err(auth_err_msg.to_owned())
}

fn resolve_cdse_credentials(auth_err_msg: &str) -> Result<(String, String), String> {
    let force_no_auth = {
        #[cfg(test)]
        {
            std::env::var("EOSTAGE_FORCE_NO_CDSE_AUTH")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
        }
        #[cfg(not(test))]
        {
            false
        }
    };
    if force_no_auth {
        return Err(auth_err_msg.to_owned());
    }
    let creds = match (
        insar_download::reveal_desktop_credential_field(
            insar_download::DesktopCredentialField::CdseUsername,
        ),
        insar_download::reveal_desktop_credential_field(
            insar_download::DesktopCredentialField::CdsePassword,
        ),
    ) {
        (Ok(Some(u)), Ok(Some(p))) if !u.trim().is_empty() && !p.is_empty() => {
            Some((u.trim().to_owned(), p.to_string()))
        }
        _ => None,
    }
    .or_else(|| {
        if is_isolated_empty_keychain_mode() {
            None
        } else {
            match (
                std::env::var("INSAR_CDSE_USERNAME"),
                std::env::var("INSAR_CDSE_PASSWORD"),
            ) {
                (Ok(u), Ok(p)) if !u.trim().is_empty() && !p.is_empty() => {
                    Some((u.trim().to_owned(), p))
                }
                _ => None,
            }
        }
    });
    creds.ok_or_else(|| auth_err_msg.to_owned())
}

fn parse_geodata_window_and_sub(
    req: &OpenGeodataQueryRequest,
    default_past_days: i64,
    default_future_days: i64,
    default_sub: &str,
) -> (String, String, String, bool) {
    let mut date_range_opt: Option<(String, String)> = None;
    let mut sub_opt: Option<String> = None;
    let mut kw_download = false;
    if let Some(raw_kw) = req
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        for token in raw_kw.split('|').map(str::trim).filter(|s| !s.is_empty()) {
            if let Some((sd, ed)) = token.split_once("..") {
                let s = sd.trim();
                let e = ed.trim();
                if !s.is_empty() || !e.is_empty() {
                    date_range_opt = Some((s.to_owned(), e.to_owned()));
                }
            } else if token.eq_ignore_ascii_case("download") {
                kw_download = true;
            } else if !token.eq_ignore_ascii_case("search")
                && !token.eq_ignore_ascii_case("preview")
            {
                sub_opt = Some(token.to_owned());
            }
        }
    }
    let now = chrono::Utc::now();
    let default_start = (now - chrono::Duration::days(default_past_days))
        .format("%Y-%m-%d")
        .to_string();
    let default_end = (now + chrono::Duration::days(default_future_days))
        .format("%Y-%m-%d")
        .to_string();
    let kw_start = date_range_opt
        .as_ref()
        .map(|(s, _)| s.as_str())
        .filter(|s| !s.is_empty());
    let kw_end = date_range_opt
        .as_ref()
        .map(|(_, e)| e.as_str())
        .filter(|s| !s.is_empty());
    let raw_start = req
        .start_date
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or(kw_start)
        .unwrap_or(&default_start)
        .to_owned();
    let raw_end = req
        .end_date
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or(kw_end)
        .unwrap_or(&default_end)
        .to_owned();
    let sub_token = sub_opt.unwrap_or_else(|| default_sub.to_owned());
    let should_download = !has_search_only_token(req.keyword.as_deref())
        && (kw_download || req.tool_name.as_deref() == Some("download"));
    (raw_start, raw_end, sub_token, should_download)
}

fn parse_nisar_oe_cmr_entries(entries: &[Value], sub_filter: &str) -> Vec<Value> {
    let want = sub_filter.trim().to_ascii_uppercase();
    let mut items = Vec::new();
    for entry in entries {
        let title = entry["title"]
            .as_str()
            .or_else(|| entry["id"].as_str())
            .unwrap_or("")
            .trim()
            .to_owned();
        if title.is_empty() {
            continue;
        }
        let upper = title.to_ascii_uppercase();
        let orbit_level = if upper.contains("_POE_") {
            "POE"
        } else if upper.contains("_MOE_") {
            "MOE"
        } else if upper.contains("_NOE_") {
            "NOE"
        } else if upper.contains("_FOE_") {
            "FOE"
        } else {
            "OE"
        };
        if matches!(want.as_str(), "POE" | "MOE" | "NOE" | "FOE") && orbit_level != want {
            continue;
        }
        let href = entry["links"]
            .as_array()
            .and_then(|links| {
                links.iter().find_map(|link| {
                    let rel = link["rel"].as_str().unwrap_or("");
                    let url = link["href"].as_str().unwrap_or("").trim();
                    let clean = url.split('?').next().unwrap_or("");
                    if rel.contains("data#")
                        && url.starts_with("https://")
                        && clean.to_ascii_lowercase().ends_with(".xml")
                    {
                        Some(url.to_owned())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();
        let dt = entry["time_start"].as_str().unwrap_or("").to_owned();
        let time_end = entry["time_end"].as_str().unwrap_or("").to_owned();
        items.push(json!({
            "id": title,
            "title": title,
            "datetime": dt,
            "time_end": time_end,
            "platform": "NISAR",
            "product_type": format!("NISAR_{orbit_level}"),
            "orbit_level": orbit_level,
            "bbox": [-180.0, -90.0, 180.0, 90.0],
            "href": href,
        }));
    }
    items
}

fn parse_copernicus_pod_odata_entries(entries: &[Value]) -> Vec<Value> {
    let mut items = Vec::new();
    for entry in entries {
        let raw_name = entry["Name"].as_str().unwrap_or("").trim();
        let id_uuid = entry["Id"].as_str().unwrap_or("").trim();
        if raw_name.is_empty() || id_uuid.is_empty() {
            continue;
        }
        let clean_id = raw_name
            .trim_end_matches(".EOF")
            .trim_end_matches(".SEN3")
            .to_owned();
        let dt = entry["ContentDate"]["Start"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let time_end = entry["ContentDate"]["End"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let size_bytes = entry["ContentLength"].as_u64().unwrap_or(0);
        let platform = if raw_name.starts_with("S2A") {
            "Sentinel-2A"
        } else if raw_name.starts_with("S2B") {
            "Sentinel-2B"
        } else if raw_name.starts_with("S2C") {
            "Sentinel-2C"
        } else if raw_name.starts_with("S3A") {
            "Sentinel-3A"
        } else if raw_name.starts_with("S3B") {
            "Sentinel-3B"
        } else if raw_name.starts_with("S3") {
            "Sentinel-3"
        } else {
            "Sentinel-2"
        };
        let prod_type = if raw_name.contains("AUX_POEORB") {
            "AUX_POEORB"
        } else if raw_name.contains("SR___POE_AX") {
            "SR___POE_AX"
        } else {
            "CDSE_POD"
        };
        let href =
            format!("https://download.dataspace.copernicus.eu/odata/v1/Products({id_uuid})/$value");
        items.push(json!({
            "id": clean_id,
            "title": raw_name,
            "filename": raw_name,
            "product_uuid": id_uuid,
            "datetime": dt,
            "time_end": time_end,
            "platform": platform,
            "product_type": prod_type,
            "size_bytes": size_bytes,
            "bbox": [-180.0, -90.0, 180.0, 90.0],
            "href": href,
        }));
    }
    items
}

pub fn execute_open_geodata_query(
    app_data_dir: &Path,
    req: &OpenGeodataQueryRequest,
) -> Result<Value, String> {
    execute_open_geodata_query_with_progress(app_data_dir, req, &|_| {})
}

pub fn execute_open_geodata_query_with_progress(
    app_data_dir: &Path,
    req: &OpenGeodataQueryRequest,
    on_progress: &(dyn Fn(Value) + Send + Sync),
) -> Result<Value, String> {
    let dataset = req.dataset.trim().to_ascii_lowercase();
    let is_cloud_suite_dispatch = matches!(
        dataset.as_str(),
        "gee_quota_status"
            | "gee_catalog_search"
            | "gee_recommend_datasets"
            | "gee_compare_datasets"
            | "gee_verify_dataset"
            | "gee_dataset_advisor"
            | "gee_export_preflight"
            | "gee_dataset_bands"
            | "gee_check_availability"
            | "gee_cloud_analyze"
            | "gee_chunked_download"
            | "gee_download"
            | "gee_artifact_get"
            | "gee_script_env_status"
            | "gee_script_inspect"
            | "gee_script_preview"
            | "gee_script_execute"
    ) || (dataset == "gee_map_tiles" && req.gee_params.is_some())
        || (dataset == "gee_catalog" && req.bbox.is_none() && req.gee_params.is_some());

    if is_cloud_suite_dispatch {
        return crate::gee_cloud_suite::dispatch_gee_geodata_query(
            app_data_dir,
            &dataset,
            req.keyword.as_deref(),
            req.bbox,
            req.start_date.as_deref(),
            req.end_date.as_deref(),
            req.output_path.as_deref(),
            req.gee_params.as_ref(),
        );
    }
    let default_out_dir = app_data_dir.join("open_geodata");
    let _ = fs::create_dir_all(&default_out_dir);

    match dataset.as_str() {
        "open_dem_tiles" | "dem_tiles" => {
            let bbox = req.bbox.unwrap_or([117.15, 31.75, 117.35, 31.90]);
            let product = req.dem_product.as_deref().unwrap_or("cop30");
            resolve_open_dem_tiles(product, bbox)
        }
        "mca_xzqh" => {
            let code = req.code.as_deref().unwrap_or("00").trim();
            let url = format!("https://dmfw.mca.gov.cn/9095/xzqh/getList?code={code}&maxLevel=2");
            let client = build_http_client(app_data_dir)?;
            let body: Value = client
                .get(&url)
                .send()
                .map_err(|err| format!("MCA_XZQH_REQUEST_FAILED: {err}"))?
                .json()
                .map_err(|err| format!("MCA_XZQH_PARSE_FAILED: {err}"))?;
            let out_path = req
                .output_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| default_out_dir.join(format!("mca_xzqh_{code}.json")));
            if let Some(parent) = out_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let bytes = serde_json::to_vec_pretty(&body).unwrap_or_default();
            let _ = fs::write(&out_path, &bytes);
            Ok(json!({
                "ok": true,
                "dataset": "mca_xzqh",
                "endpoint": url,
                "output_path": out_path.to_string_lossy(),
                "downloaded_file": out_path.to_string_lossy(),
                "downloaded_path": out_path.to_string_lossy(),
                "bytes_written": bytes.len(),
                "response": body,
            }))
        }
        "mca_placename" => {
            let kw = req.keyword.as_deref().unwrap_or("合肥").trim();
            let code = req.code.as_deref().unwrap_or("").trim();
            let client = build_http_client(app_data_dir)?;
            let resp = client
                .get("https://dmfw.mca.gov.cn/9095/stname/listPub")
                .query(&[
                    ("stName", kw),
                    ("searchType", "模糊"),
                    ("code", code),
                    ("page", "1"),
                    ("size", "20"),
                ])
                .send()
                .map_err(|err| format!("MCA_PLACENAME_REQUEST_FAILED: {err}"))?;
            let body: Value = resp
                .json()
                .map_err(|err| format!("MCA_PLACENAME_PARSE_FAILED: {err}"))?;
            Ok(json!({
                "ok": true,
                "dataset": "mca_placename",
                "keyword": kw,
                "response": body,
            }))
        }
        "datav_bound" => {
            let adcode = req.code.as_deref().unwrap_or("340100").trim();
            let suffix = if req.full_children.unwrap_or(true) {
                "_full"
            } else {
                ""
            };
            let url =
                format!("https://geo.datav.aliyun.com/areas_v3/bound/{adcode}{suffix}.json");
            let client = build_http_client(app_data_dir)?;
            let body: Value = client
                .get(&url)
                .send()
                .map_err(|err| format!("DATAV_BOUND_REQUEST_FAILED: {err}"))?
                .json()
                .map_err(|err| format!("DATAV_BOUND_PARSE_FAILED: {err}"))?;
            let feature_count = body
                .get("features")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let out_path = req
                .output_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    default_out_dir.join(format!("datav_{adcode}{suffix}.geojson"))
                });
            if let Some(parent) = out_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let serialized = serde_json::to_vec_pretty(&body).unwrap_or_default();
            fs::write(&out_path, &serialized)
                .map_err(|err| format!("DATAV_WRITE_FAILED: {err}"))?;
            Ok(json!({
                "ok": true,
                "dataset": "datav_bound",
                "adcode": adcode,
                "endpoint": url,
                "feature_count": feature_count,
                "output_path": out_path.to_string_lossy(),
                "downloaded_file": out_path.to_string_lossy(),
                "downloaded_path": out_path.to_string_lossy(),
                "bytes_written": serialized.len(),
            }))
        }
        "open_meteo_era5" => {
            // 1. Resolve date range and variable set ("hourly_insar" / "hourly_full" / "daily_summary")
            let mut start_date = req
                .start_date
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let mut end_date = req
                .end_date
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let mut var_set = String::from("hourly_insar");

            if let Some(kw) = req.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                for raw_token in kw.split('|') {
                    let token = raw_token.trim();
                    if token.is_empty() {
                        continue;
                    }
                    if token.contains("..") {
                        let mut dt_parts = token.split("..");
                        let s = dt_parts.next().unwrap_or("").trim();
                        let e = dt_parts.next().unwrap_or("").trim();
                        if !s.is_empty() && start_date.is_none() {
                            start_date = Some(s.to_string());
                        }
                        if !e.is_empty() && end_date.is_none() {
                            end_date = Some(e.to_string());
                        }
                    } else if token.eq_ignore_ascii_case("hourly_insar")
                        || token.eq_ignore_ascii_case("hourly_full")
                        || token.eq_ignore_ascii_case("daily_summary")
                    {
                        var_set = token.to_ascii_lowercase();
                    }
                }
            }
            if let Some(dp) = req
                .dem_product
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if dp.eq_ignore_ascii_case("hourly_insar")
                    || dp.eq_ignore_ascii_case("hourly_full")
                    || dp.eq_ignore_ascii_case("daily_summary")
                {
                    var_set = dp.to_ascii_lowercase();
                }
            }
            if start_date.is_none() {
                if let Some(d) = req
                    .code
                    .as_deref()
                    .or(req.osm_tag.as_deref())
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && s.contains('-'))
                {
                    start_date = Some(d.to_string());
                    end_date = Some(d.to_string());
                }
            }
            let s_date = start_date.unwrap_or_else(|| "2024-01-01".to_string());
            let e_date = end_date.unwrap_or_else(|| s_date.clone());

            // 2. Resolve coordinates / grid sampling points from bbox or lat/lng
            let (center_lat, center_lng, grid_points) = if let Some(bbox) = req.bbox {
                let clat = (bbox[1] + bbox[3]) * 0.5;
                let clng = (bbox[0] + bbox[2]) * 0.5;
                let pts = vec![
                    (clat, clng, "center"),
                    (bbox[3], bbox[0], "NW"),
                    (bbox[3], bbox[2], "NE"),
                    (bbox[1], bbox[0], "SW"),
                    (bbox[1], bbox[2], "SE"),
                ];
                (clat, clng, pts)
            } else {
                let clat = req.lat.unwrap_or(31.8206);
                let clng = req.lng.unwrap_or(117.2272);
                let pts = vec![(clat, clng, "point")];
                (clat, clng, pts)
            };

            let lats_str = grid_points
                .iter()
                .map(|p| format!("{:.4}", p.0))
                .collect::<Vec<_>>()
                .join(",");
            let lngs_str = grid_points
                .iter()
                .map(|p| format!("{:.4}", p.1))
                .collect::<Vec<_>>()
                .join(",");

            let client = build_http_client(app_data_dir)?;
            let query_params: Vec<(&str, &str)> = match var_set.as_str() {
                "daily_summary" => vec![
                    ("latitude", lats_str.as_str()),
                    ("longitude", lngs_str.as_str()),
                    ("start_date", s_date.as_str()),
                    ("end_date", e_date.as_str()),
                    (
                        "daily",
                        "temperature_2m_max,temperature_2m_min,temperature_2m_mean,precipitation_sum,rain_sum,wind_speed_10m_max,shortwave_radiation_sum",
                    ),
                    ("timezone", "UTC"),
                ],
                "hourly_full" => vec![
                    ("latitude", lats_str.as_str()),
                    ("longitude", lngs_str.as_str()),
                    ("start_date", s_date.as_str()),
                    ("end_date", e_date.as_str()),
                    (
                        "hourly",
                        "temperature_2m,relative_humidity_2m,dew_point_2m,precipitation,rain,snowfall,surface_pressure,pressure_msl,cloud_cover,wind_speed_10m,wind_direction_10m,shortwave_radiation",
                    ),
                    ("timezone", "UTC"),
                ],
                _ => vec![
                    ("latitude", lats_str.as_str()),
                    ("longitude", lngs_str.as_str()),
                    ("start_date", s_date.as_str()),
                    ("end_date", e_date.as_str()),
                    (
                        "hourly",
                        "temperature_2m,relative_humidity_2m,dew_point_2m,surface_pressure,pressure_msl,cloud_cover,wind_speed_10m",
                    ),
                    ("timezone", "UTC"),
                ],
            };

            let resp = http_get_with_retry(
                &client,
                "https://archive-api.open-meteo.com/v1/archive",
                Some(&query_params),
                3,
            )
            .map_err(|err| format!("OPEN_METEO_ERA5_REQUEST_FAILED: {err}"))?;
            let http_status = resp.status();
            let body: Value = resp
                .json()
                .map_err(|err| format!("OPEN_METEO_ERA5_PARSE_FAILED (HTTP {http_status}): {err}"))?;

            if !http_status.is_success() || body.get("error") == Some(&Value::Bool(true)) {
                let reason = body["reason"]
                    .as_str()
                    .unwrap_or("Open-Meteo ERA5 API returned error for requested date window");
                return Err(format!(
                    "OPEN_METEO_ERA5_API_ERROR (HTTP {http_status}, {s_date}..{e_date}): {reason}"
                ));
            }

            let raw_results: Vec<Value> = if body.is_array() {
                body.as_array().cloned().unwrap_or_default()
            } else {
                vec![body.clone()]
            };

            let era5_search_only = has_search_only_token(req.keyword.as_deref())
                && req.tool_name.as_deref() != Some("download");
            let out_dir = req
                .output_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| default_out_dir.join("era5_reanalysis"));
            let json_file = out_dir.join(format!("era5_{s_date}_{e_date}_grid.json"));
            let csv_file = out_dir.join(format!("era5_{s_date}_{e_date}_grid.csv"));
            let existing_csv_bytes = existing_valid_science_file_bytes(&csv_file);

            let mut csv_buf = match var_set.as_str() {
                "daily_summary" => String::from(
                    "point_tag,latitude,longitude,elevation_m,date,temperature_2m_max_degC,temperature_2m_min_degC,temperature_2m_mean_degC,precipitation_sum_mm,rain_sum_mm,wind_speed_10m_max_kmh,shortwave_radiation_sum_MJm2\n",
                ),
                "hourly_full" => String::from(
                    "point_tag,latitude,longitude,elevation_m,timestamp,temperature_2m_degC,relative_humidity_2m_percent,dew_point_2m_degC,precipitation_mm,rain_mm,snowfall_cm,surface_pressure_hPa,pressure_msl_hPa,geopotential_height_m,cloud_cover_percent,wind_speed_10m_kmh,wind_direction_10m_deg,shortwave_radiation_Wm2\n",
                ),
                _ => String::from(
                    "point_tag,latitude,longitude,elevation_m,timestamp,temperature_2m_degC,relative_humidity_2m_percent,dew_point_2m_degC,surface_pressure_hPa,pressure_msl_hPa,geopotential_height_m,cloud_cover_percent,wind_speed_10m_kmh\n",
                ),
            };

            let mut items = Vec::new();
            let mut total_hours = 0usize;
            let mut total_days = 0usize;

            for (idx, res) in raw_results.iter().enumerate() {
                let tag = grid_points.get(idx).map(|p| p.2).unwrap_or("grid");
                let pt_lat = res["latitude"].as_f64().unwrap_or(grid_points[idx].0);
                let pt_lng = res["longitude"].as_f64().unwrap_or(grid_points[idx].1);
                let elev = res["elevation"].as_f64().unwrap_or(0.0);

                if var_set == "daily_summary" {
                    if let Some(daily) = res.get("daily") {
                        let times = daily["time"].as_array().cloned().unwrap_or_default();
                        let tmax = daily["temperature_2m_max"].as_array().cloned().unwrap_or_default();
                        let tmin = daily["temperature_2m_min"].as_array().cloned().unwrap_or_default();
                        let tmean = daily["temperature_2m_mean"].as_array().cloned().unwrap_or_default();
                        let psum = daily["precipitation_sum"].as_array().cloned().unwrap_or_default();
                        let rsum = daily["rain_sum"].as_array().cloned().unwrap_or_default();
                        let wmax = daily["wind_speed_10m_max"].as_array().cloned().unwrap_or_default();
                        let swsum = daily["shortwave_radiation_sum"].as_array().cloned().unwrap_or_default();

                        total_days = total_days.max(times.len());
                        total_hours = total_hours.max(times.len() * 24);
                        for (t_idx, time_val) in times.iter().enumerate() {
                            let dt = time_val.as_str().unwrap_or("");
                            let v_tmax = tmax.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let v_tmin = tmin.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let v_tmean = tmean.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let v_psum = psum.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let v_rsum = rsum.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let v_wmax = wmax.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let v_swsum = swsum.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            csv_buf.push_str(&format!(
                                "{tag},{pt_lat:.4},{pt_lng:.4},{elev:.1},{dt},{v_tmax:.1},{v_tmin:.1},{v_tmean:.1},{v_psum:.2},{v_rsum:.2},{v_wmax:.1},{v_swsum:.2}\n"
                            ));
                        }
                    }
                } else if let Some(hourly) = res.get("hourly") {
                    let times = hourly["time"].as_array().cloned().unwrap_or_default();
                    let t2m = hourly["temperature_2m"].as_array().cloned().unwrap_or_default();
                    let rh = hourly["relative_humidity_2m"].as_array().cloned().unwrap_or_default();
                    let td = hourly["dew_point_2m"].as_array().cloned().unwrap_or_default();
                    let psfc = hourly["surface_pressure"].as_array().cloned().unwrap_or_default();
                    let pmsl = hourly["pressure_msl"].as_array().cloned().unwrap_or_default();
                    let cc = hourly["cloud_cover"].as_array().cloned().unwrap_or_default();
                    let ws = hourly["wind_speed_10m"].as_array().cloned().unwrap_or_default();
                    let precip = hourly["precipitation"].as_array().cloned().unwrap_or_default();
                    let rain = hourly["rain"].as_array().cloned().unwrap_or_default();
                    let snow = hourly["snowfall"].as_array().cloned().unwrap_or_default();
                    let wdir = hourly["wind_direction_10m"].as_array().cloned().unwrap_or_default();
                    let swrad = hourly["shortwave_radiation"].as_array().cloned().unwrap_or_default();

                    total_hours = total_hours.max(times.len());
                    for (t_idx, time_val) in times.iter().enumerate() {
                        let tm = time_val.as_str().unwrap_or("");
                        let t_val = t2m.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                        let rh_val = rh.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                        let td_val = td.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                        let ps_val = psfc.get(t_idx).and_then(Value::as_f64).unwrap_or(1013.25);
                        let pm_val = pmsl.get(t_idx).and_then(Value::as_f64).unwrap_or(1013.25);
                        let cc_val = cc.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                        let ws_val = ws.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);

                        let r_d = 287.058;
                        let g_0 = 9.80665;
                        let t_k = t_val + 273.15;
                        let z_m = if ps_val > 0.0 && pm_val > 0.0 {
                            elev + (r_d * t_k / g_0) * (pm_val / ps_val).ln()
                        } else {
                            elev
                        };

                        if var_set == "hourly_full" {
                            let pr_val = precip.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let rn_val = rain.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let sn_val = snow.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let wd_val = wdir.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            let sw_val = swrad.get(t_idx).and_then(Value::as_f64).unwrap_or(0.0);
                            csv_buf.push_str(&format!(
                                "{tag},{pt_lat:.4},{pt_lng:.4},{elev:.1},{tm},{t_val:.1},{rh_val:.0},{td_val:.1},{pr_val:.2},{rn_val:.2},{sn_val:.2},{ps_val:.1},{pm_val:.1},{z_m:.1},{cc_val:.0},{ws_val:.1},{wd_val:.0},{sw_val:.1}\n"
                            ));
                        } else {
                            csv_buf.push_str(&format!(
                                "{tag},{pt_lat:.4},{pt_lng:.4},{elev:.1},{tm},{t_val:.1},{rh_val:.0},{td_val:.1},{ps_val:.1},{pm_val:.1},{z_m:.1},{cc_val:.0},{ws_val:.1}\n"
                            ));
                        }
                    }
                }

                let prod_type = match var_set.as_str() {
                    "daily_summary" => "DAILY_SUMMARY_GRID",
                    "hourly_full" => "HOURLY_FULL_METEO_GRID",
                    _ => "HOURLY_ATMOSPHERIC_GRID",
                };
                items.push(json!({
                    "id": format!("ERA5_{s_date}_{e_date}_{tag}"),
                    "title": format!("ERA5 Reanalysis {tag} ({pt_lat:.2}, {pt_lng:.2})"),
                    "datetime": format!("{s_date}T00:00:00Z"),
                    "end_date": format!("{e_date}T23:00:00Z"),
                    "lat": pt_lat,
                    "lon": pt_lng,
                    "bbox": [pt_lng - 0.08, pt_lat - 0.08, pt_lng + 0.08, pt_lat + 0.08],
                    "product_type": prod_type,
                    "variable_set": var_set,
                    "already_downloaded": existing_csv_bytes.is_some(),
                    "local_file_path": existing_csv_bytes.map(|_| csv_file.to_string_lossy().to_string()),
                    "download_url": csv_file.to_string_lossy(),
                }));
            }

            if era5_search_only {
                return Ok(json!({
                    "ok": true,
                    "dataset": "open_meteo_era5",
                    "variable_set": var_set,
                    "bbox": req.bbox,
                    "center_lat": center_lat,
                    "center_lng": center_lng,
                    "start_date": s_date,
                    "end_date": e_date,
                    "grid_points_count": grid_points.len(),
                    "total_hours": total_hours,
                    "total_days": total_days,
                    "downloaded_file": Value::Null,
                    "downloaded_path": Value::Null,
                    "downloaded_bytes": 0,
                    "items": items,
                }));
            }

            let mut already_exists = false;
            let downloaded_bytes = if let Some(existing_len) = existing_csv_bytes {
                already_exists = true;
                existing_len as usize
            } else {
                let _ = fs::create_dir_all(&out_dir);
                let serialized = serde_json::to_vec_pretty(&body).unwrap_or_default();
                fs::write(&json_file, &serialized)
                    .map_err(|err| format!("OPEN_METEO_WRITE_FAILED: {err}"))?;
                fs::write(&csv_file, csv_buf.as_bytes())
                    .map_err(|err| format!("ERA5_CSV_WRITE_FAILED: {err}"))?;
                serialized.len() + csv_buf.len()
            };

            Ok(json!({
                "ok": true,
                "dataset": "open_meteo_era5",
                "variable_set": var_set,
                "bbox": req.bbox,
                "center_lat": center_lat,
                "center_lng": center_lng,
                "start_date": s_date,
                "end_date": e_date,
                "grid_points_count": grid_points.len(),
                "total_hours": total_hours,
                "total_days": total_days,
                "already_exists": already_exists,
                "output_path": json_file.to_string_lossy(),
                "csv_path": csv_file.to_string_lossy(),
                "downloaded_file": csv_file.to_string_lossy(),
                "downloaded_path": csv_file.to_string_lossy(),
                "downloaded_bytes": downloaded_bytes,
                "items": items,
            }))
        }
        "overpass_osm" => {
            let [west, south, east, north] = req.bbox.unwrap_or([117.20, 31.80, 117.26, 31.85]);
            let tag = req.osm_tag.as_deref().unwrap_or("waterway").trim();
            let ql = format!(
                "[out:json][timeout:25];(way[\"{tag}\"]({south},{west},{north},{east});relation[\"{tag}\"]({south},{west},{north},{east}););out body;>;out skel qt;"
            );
            let client = build_http_client(app_data_dir)?;
            let endpoints = [
                "https://overpass.kumi.systems/api/interpreter",
                "https://overpass-api.de/api/interpreter",
            ];
            let mut last_err = String::new();
            let mut parsed_body = None;
            let mut used_ep = "";
            for ep in endpoints {
                match client.post(ep).form(&[("data", ql.as_str())]).send() {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(val) = r.json::<Value>() {
                            parsed_body = Some(val);
                            used_ep = ep;
                            break;
                        }
                    }
                    Ok(r) => {
                        last_err = format!("HTTP {}", r.status());
                    }
                    Err(e) => {
                        last_err = e.to_string();
                    }
                }
            }
            let body = parsed_body
                .ok_or_else(|| format!("OVERPASS_OSM_QUERY_FAILED: {last_err}"))?;
            let element_count = body
                .get("elements")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let out_path = req
                .output_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| default_out_dir.join(format!("osm_{tag}.json")));
            if let Some(parent) = out_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let serialized = serde_json::to_vec_pretty(&body).unwrap_or_default();
            fs::write(&out_path, &serialized)
                .map_err(|err| format!("OVERPASS_WRITE_FAILED: {err}"))?;
            Ok(json!({
                "ok": true,
                "dataset": "overpass_osm",
                "osm_tag": tag,
                "bbox": [west, south, east, north],
                "endpoint_used": used_ep,
                "element_count": element_count,
                "output_path": out_path.to_string_lossy(),
                "downloaded_file": out_path.to_string_lossy(),
                "downloaded_path": out_path.to_string_lossy(),
                "bytes_written": serialized.len(),
            }))
        }
        "worldpop" => {
            let iso3_upper = req.iso3.as_deref().unwrap_or("CHN").trim().to_ascii_uppercase();
            let iso3_lower = iso3_upper.to_ascii_lowercase();
            let year = req.year.unwrap_or(2020).clamp(2000, 2020);
            let cog_url = format!(
                "https://data.worldpop.org/GIS/Population/Global_2000_2020_1km/{year}/{iso3_upper}/{iso3_lower}_ppp_{year}_1km_Aggregated.tif"
            );
            let rest_url = format!("https://hub.worldpop.org/rest/data/pop/wpic1km?iso3={iso3_upper}");
            Ok(json!({
                "ok": true,
                "dataset": "worldpop",
                "iso3": iso3_upper,
                "year": year,
                "cog_url": cog_url,
                "vsicurl_path": format!("/vsicurl/{cog_url}"),
                "catalog_rest_url": rest_url,
            }))
        }
        "whitebox_run" | "whitebox" => {
            let rt_root = crate::application::discover_runtime_root();
            let exe = crate::application::resolve_component_binary(
                &rt_root.join("runtime/whitebox-cli"),
                &["whitebox_tools"],
            )
            .ok_or_else(|| {
                format!(
                    "WHITEBOX_NOT_INSTALLED: 未找到 {}",
                    rt_root.join("runtime/whitebox-cli/whitebox_tools").display()
                )
            })?;
            let mut cmd = std::process::Command::new(&exe);
            if let Some(tool) = req.tool_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                if tool.eq_ignore_ascii_case("version") || tool == "--version" {
                    cmd.arg("--version");
                } else if tool.eq_ignore_ascii_case("listtools") || tool == "--listtools" {
                    cmd.arg("--listtools");
                } else {
                    cmd.arg(format!("--run={tool}"));
                }
            } else {
                cmd.arg("--version");
            }
            if let Some(args) = &req.tool_args {
                for a in args {
                    cmd.arg(a);
                }
            }
            let out = cmd
                .output()
                .map_err(|err| format!("WHITEBOX_EXEC_FAILED: {err}"))?;
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            Ok(json!({
                "ok": out.status.success(),
                "dataset": "whitebox_run",
                "executable": exe.to_string_lossy(),
                "exit_code": out.status.code(),
                "stdout": stdout,
                "stderr": stderr,
            }))
        }
        "duckdb_sql" | "duckdb" => {
            let rt_root = crate::application::discover_runtime_root();
            let exe = crate::application::resolve_component_binary(
                &rt_root.join("runtime/duckdb-cli"),
                &["duckdb"],
            )
            .ok_or_else(|| {
                format!(
                    "DUCKDB_NOT_INSTALLED: 未找到 {}",
                    rt_root.join("runtime/duckdb-cli/duckdb").display()
                )
            })?;
            let spatial_ext = rt_root.join("runtime/duckdb-cli/ext/spatial.duckdb_extension");
            let user_sql = req
                .sql
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("SELECT ST_AsText(ST_Point(117.28, 31.86)) AS geometry_wkt;");
            let full_sql = if spatial_ext.is_file() {
                let ext_slash = spatial_ext.to_string_lossy().replace('\\', "/");
                format!("LOAD '{ext_slash}'; {user_sql}")
            } else {
                user_sql.to_owned()
            };
            let out = std::process::Command::new(&exe)
                .arg("-unsigned")
                .arg("-json")
                .arg("-c")
                .arg(&full_sql)
                .output()
                .map_err(|err| format!("DUCKDB_EXEC_FAILED: {err}"))?;
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            let rows: Value = serde_json::from_str(&stdout).unwrap_or_else(|_| Value::String(stdout.clone()));
            Ok(json!({
                "ok": out.status.success(),
                "dataset": "duckdb_sql",
                "executable": exe.to_string_lossy(),
                "spatial_loaded": spatial_ext.is_file(),
                "exit_code": out.status.code(),
                "rows": rows,
                "stderr": stderr,
            }))
        }
        "xyz_satellite_tiles" | "satellite_tiles" => {
            let bbox = req
                .bbox
                .ok_or_else(|| "MISSING_BBOX: xyz_satellite_tiles 需要 bbox=[west, south, east, north]".to_owned())?;
            let raw_spec = req
                .keyword
                .as_deref()
                .or(req.dem_product.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("arcgis_satellite:13");

            // Extract custom URL template:
            // 1. From req.code / req.sql / req.osm_tag
            // 2. Or fallback: extract embedded http(s) URL if present in raw_spec
            let mut extracted_url = None;
            let cleaned_spec: String;

            let explicit_code_url = req
                .code
                .as_deref()
                .or(req.sql.as_deref())
                .or(req.osm_tag.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            if let Some(code_url) = explicit_code_url {
                cleaned_spec = raw_spec.to_string();
                extracted_url = Some(code_url.to_string());
            } else if let Some(url_pos) = raw_spec.find("http://").or_else(|| raw_spec.find("https://")) {
                let tail = &raw_spec[url_pos..];
                let scheme_len = if tail.starts_with("https://") { 8 } else { 7 };
                // Locate the first '/' after host[:port] so host ports like `127.0.0.1:8080` are NEVER split!
                let path_start = tail[scheme_len..]
                    .find('/')
                    .map(|idx| scheme_len + idx)
                    .unwrap_or(tail.len());
                // Spec options (`:zoom:mode:...`) appear after `{z}/{x}/{y}` (last `}`) or after the URL path.
                let search_from = tail.rfind('}').map(|idx| idx + 1).unwrap_or(path_start);
                let raw_prefix = raw_spec[..url_pos].trim_end_matches('|').trim_end_matches(':').trim();
                let effective_prefix = if raw_prefix.is_empty() { "custom" } else { raw_prefix };
                if let Some(colon_offset) = tail[search_from..].find(':') {
                    let split_at = search_from + colon_offset;
                    extracted_url = Some(tail[..split_at].trim().to_string());
                    cleaned_spec = format!("{effective_prefix}:{}", &tail[split_at + 1..]);
                } else {
                    extracted_url = Some(tail.trim().to_string());
                    cleaned_spec = effective_prefix.to_string();
                }
            } else {
                cleaned_spec = raw_spec.to_string();
            }

            let mut parts = cleaned_spec.split(':');
            let first_token = parts.next().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("arcgis_satellite");

            let (source_key, inline_coord_sys) = if let Some((src, coord)) = first_token.split_once('|') {
                (src.trim().to_ascii_lowercase(), Some(coord.trim().to_ascii_lowercase()))
            } else {
                (first_token.to_ascii_lowercase(), None)
            };
            if source_key == "esri_wayback"
                || (source_key.starts_with("wayback_")
                    && wayback::parse_wayback_version_id(&source_key).is_none())
            {
                return Err(format!(
                    "INVALID_WAYBACK_VERSION_ID: Wayback 历史影像下载必须指定有效版本 ID (wayback_<digits>)，当前为 '{source_key}'。"
                ));
            }
            let source_max_zoom = xyz_source_max_zoom(&source_key);

            let zoom_token_opt = parts.next().map(str::trim).filter(|z| !z.is_empty());
            let requested_zoom: u32 = if let Some(tok) = zoom_token_opt {
                if let Ok(z_num) = tok.parse::<i64>() {
                    if z_num < 1 || z_num > source_max_zoom as i64 {
                        return Err(format!(
                            "XYZ_ZOOM_OUT_OF_RANGE: 数据源 {source_key} 仅支持 Z1–Z{source_max_zoom} 缩放级别，当前请求了 Z{z_num}。"
                        ));
                    }
                    z_num as u32
                } else {
                    let fb = req.year.unwrap_or(13);
                    if fb < 1 || fb > source_max_zoom {
                        return Err(format!(
                            "XYZ_ZOOM_OUT_OF_RANGE: 数据源 {source_key} 仅支持 Z1–Z{source_max_zoom} 缩放级别，当前请求了 Z{fb}。"
                        ));
                    }
                    fb
                }
            } else {
                let fb = req.year.unwrap_or(13);
                if fb < 1 || fb > source_max_zoom {
                    return Err(format!(
                        "XYZ_ZOOM_OUT_OF_RANGE: 数据源 {source_key} 仅支持 Z1–Z{source_max_zoom} 缩放级别，当前请求了 Z{fb}。"
                    ));
                }
                fb
            };
            let mode_flag = parts.next().unwrap_or("mosaic").trim().to_ascii_lowercase();
            let stitch_mosaic = mode_flag != "nomosaic" && mode_flag != "tiles_only";
            let export_format = parts.next().unwrap_or("gtiff").trim().to_ascii_lowercase();
            let compression = parts.next().unwrap_or("DEFLATE").trim().to_ascii_uppercase();
            let pyramids_flag = parts.next().unwrap_or("pyramids").trim().to_ascii_lowercase();
            let build_pyramids = pyramids_flag != "no_pyramids"
                && pyramids_flag != "nopyramids"
                && pyramids_flag != "false"
                && pyramids_flag != "0";
            let coord_sys_flag = inline_coord_sys
                .or_else(|| parts.next().map(|s| s.trim().to_ascii_lowercase()))
                .unwrap_or_else(|| "auto".to_owned());

            let custom_url_template = extracted_url
                .unwrap_or_else(|| "https://mt1.google.com/vt/lyrs=s&x={x}&y={y}&z={z}".to_string());

            let is_gcj02 = if coord_sys_flag == "gcj02" {
                true
            } else if coord_sys_flag == "wgs84" {
                false
            } else {
                source_key.contains("gaode")
                    || source_key.contains("amap")
                    || (matches!(source_key.as_str(), "custom" | "custom_xyz" | "custom_tms")
                        && (custom_url_template.contains("autonavi.com")
                            || custom_url_template.contains("amap.com")))
            };
            let query_bbox = if is_gcj02 {
                if let Ok(transformed) = transform_lbs_coordinates(
                    "wgs84",
                    "gcj02",
                    &[[bbox[0], bbox[1]], [bbox[2], bbox[3]]],
                ) {
                    let w = transformed["results"][0]["target_lng"].as_f64().unwrap_or(bbox[0]);
                    let s = transformed["results"][0]["target_lat"].as_f64().unwrap_or(bbox[1]);
                    let e = transformed["results"][1]["target_lng"].as_f64().unwrap_or(bbox[2]);
                    let n = transformed["results"][1]["target_lat"].as_f64().unwrap_or(bbox[3]);
                    [w.min(e), s.min(n), w.max(e), s.max(n)]
                } else {
                    [
                        bbox[0].min(bbox[2]),
                        bbox[1].min(bbox[3]),
                        bbox[0].max(bbox[2]),
                        bbox[1].max(bbox[3]),
                    ]
                }
            } else {
                [
                    bbox[0].min(bbox[2]),
                    bbox[1].min(bbox[3]),
                    bbox[0].max(bbox[2]),
                    bbox[1].max(bbox[3]),
                ]
            };

            let lon_to_tile_x = |lon: f64, z: u32| -> u32 {
                let n = (1u64 << z) as f64;
                let x = ((lon.clamp(-179.9999, 179.9999) + 180.0) / 360.0 * n).floor();
                (x as i64).clamp(0, (1i64 << z) - 1) as u32
            };
            let lat_to_tile_y = |lat: f64, z: u32| -> u32 {
                let n = (1u64 << z) as f64;
                let lat_rad = lat.clamp(-85.0511, 85.0511).to_radians();
                let y = ((1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n).floor();
                (y as i64).clamp(0, (1i64 << z) - 1) as u32
            };
            let tile_to_mercator_bounds = |x: u32, y: u32, z: u32| -> [f64; 4] {
                let origin_shift = 20037508.342789244_f64;
                let total = (1u64 << z) as f64;
                let tile_span = (2.0 * origin_shift) / total;
                let min_x = -origin_shift + (x as f64) * tile_span;
                let max_x = -origin_shift + ((x + 1) as f64) * tile_span;
                let max_y = origin_shift - (y as f64) * tile_span;
                let min_y = origin_shift - ((y + 1) as f64) * tile_span;
                [min_x, min_y, max_x, max_y]
            };

            let mut target_zooms: Vec<u32> = if let Some(zs) = req.zooms.as_ref().filter(|v| !v.is_empty()) {
                let mut validated = Vec::with_capacity(zs.len());
                for &z_raw in zs {
                    let z = u32::from(z_raw);
                    if z < 1 || z > source_max_zoom {
                        return Err(format!(
                            "XYZ_ZOOM_OUT_OF_RANGE: 数据源 {source_key} 仅支持 Z1–Z{source_max_zoom} 缩放级别，当前请求了 Z{z}。"
                        ));
                    }
                    validated.push(z);
                }
                validated
            } else if let Some(zs) = req
                .gee_params
                .as_ref()
                .and_then(|g| g.get("xyz_zooms").or_else(|| g.get("zooms")))
                .and_then(Value::as_array)
                .filter(|arr| !arr.is_empty())
            {
                let mut validated = Vec::with_capacity(zs.len());
                for item in zs {
                    let Some(z_num) = item.as_i64() else {
                        continue;
                    };
                    if z_num < 1 || z_num > source_max_zoom as i64 {
                        return Err(format!(
                            "XYZ_ZOOM_OUT_OF_RANGE: 数据源 {source_key} 仅支持 Z1–Z{source_max_zoom} 缩放级别，当前请求了 Z{z_num}。"
                        ));
                    }
                    validated.push(z_num as u32);
                }
                validated
            } else {
                vec![requested_zoom]
            };
            target_zooms.sort_unstable();
            target_zooms.dedup();
            if target_zooms.is_empty() {
                target_zooms.push(requested_zoom);
            }
            let primary_zoom = *target_zooms.iter().max().unwrap_or(&requested_zoom);
            let zoom_tag = if target_zooms.len() == 1 {
                format!("z{}", primary_zoom)
            } else {
                format!("z{}-z{}", target_zooms[0], primary_zoom)
            };

            let will_stitch_raster = stitch_mosaic && export_format != "mbtiles" && export_format != "vrt";
            let requires_raster_stitch_or_vrt = (stitch_mosaic && export_format != "mbtiles") || export_format == "vrt";
            let max_tiles_limit = xyz_max_tiles_limit(requires_raster_stitch_or_vrt);
            let mut zoom_grids: Vec<(u32, u32, u32, u32, u32)> = Vec::with_capacity(target_zooms.len());
            let mut total_planned_tiles_u64: u64 = 0;
            for &z in &target_zooms {
                let zx_min = lon_to_tile_x(query_bbox[0], z);
                let zx_max = lon_to_tile_x(query_bbox[2], z);
                let zy_min = lat_to_tile_y(query_bbox[3], z);
                let zy_max = lat_to_tile_y(query_bbox[1], z);
                let nx = u64::from(zx_max.saturating_sub(zx_min)).saturating_add(1);
                let ny = u64::from(zy_max.saturating_sub(zy_min)).saturating_add(1);
                if will_stitch_raster && (export_format == "jpg" || export_format == "jpeg") {
                    let px_w = nx.saturating_mul(256);
                    let px_h = ny.saturating_mul(256);
                    if px_w > XYZ_JPEG_MAX_DIMENSION || px_h > XYZ_JPEG_MAX_DIMENSION {
                        return Err(format!(
                            "XYZ_JPEG_DIMENSION_EXCEEDED: JPEG 拼接导出单边最大支持 {XYZ_JPEG_MAX_DIMENSION} 像素（约 255 块瓦片），当前 Z{z} 预计生成 {px_w}×{px_h} 像素。请缩小区域或改用 GeoTIFF / COG / PNG。"
                        ));
                    }
                }
                total_planned_tiles_u64 = total_planned_tiles_u64.saturating_add(nx.saturating_mul(ny));
                zoom_grids.push((z, zx_min, zx_max, zy_min, zy_max));
            }
            if total_planned_tiles_u64 > max_tiles_limit as u64 {
                return Err(format!(
                    "XYZ_TILES_LIMIT_EXCEEDED: 当前范围与缩放层级预计需要 {total_planned_tiles_u64} 张瓦片，超出当前导出模式上限 {max_tiles_limit} 张。请缩小区域、降低缩放级别，或切换至 MBTiles / ZIP 瓦片归档模式。"
                ));
            }
            let tiles_clamped = false;
            let mut tile_jobs: Vec<(u32, u32, u32)> = Vec::with_capacity(total_planned_tiles_u64 as usize);
            for &(z, zx_min, zx_max, zy_min, zy_max) in &zoom_grids {
                for x in zx_min..=zx_max {
                    for y in zy_min..=zy_max {
                        tile_jobs.push((z, x, y));
                    }
                }
            }
            let (_, x_min, x_max, y_min, y_max) = zoom_grids
                .iter()
                .copied()
                .find(|(z, ..)| *z == primary_zoom)
                .unwrap_or((primary_zoom, 0, 0, 0, 0));

            let out_dir = req
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    app_data_dir
                        .join("geodata_downloads")
                        .join(format!("satellite_tiles_{}_{}", source_key, zoom_tag))
                });
            fs::create_dir_all(&out_dir)
                .map_err(|err| format!("CREATE_TILE_DIR_FAILED: {err}"))?;

            let ext = if source_key.contains("osm")
                || source_key.contains("carto")
                || source_key.contains("opentopo")
                || source_key.contains("gaode_map")
                || source_key.contains("google_map")
                || source_key.contains("tianditu_vec")
                || source_key == "tiandituvector"
                || (matches!(source_key.as_str(), "custom" | "custom_xyz" | "custom_tms")
                    && custom_url_template.to_ascii_lowercase().contains(".png"))
            {
                "png"
            } else {
                "jpg"
            };
            let world_ext = if ext == "png" { "pgw" } else { "jgw" };
            let write_sidecars = export_format != "mbtiles" && (stitch_mosaic || export_format != "zip");
            let epsg3857_prj = r#"PROJCS["WGS 84 / Pseudo-Mercator",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Mercator_1SP"],PARAMETER["central_meridian",0],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1]]"#;

            let tianditu_token = resolve_tianditu_token(app_data_dir);
            if source_key.starts_with("tianditu") && tianditu_token.trim().is_empty() {
                return Err(
                    "TIANDITU_TOKEN_REQUIRED: 批量下载天地图瓦片需要配置您自己的天地图 Key，请在「设置 -> 网络与代理」中填写后重试。".to_owned(),
                );
            }
            let build_tile_url = |x: u32, y: u32, z: u32| -> String {
                build_xyz_tile_url(&source_key, &custom_url_template, z, x, y, &tianditu_token)
            };

            let client = build_http_client(app_data_dir)?;
            let total_planned_tiles = tile_jobs.len();
            let configured_tile_workers = insar_download::load_transfer_engine_prefs_in_dir(app_data_dir)
                .map(|p| usize::from(p.tile_chunk_concurrency))
                .unwrap_or(16)
                .clamp(4, 64);
            let worker_count = configured_tile_workers.min(total_planned_tiles.max(1));

            on_progress(json!({
                "phase": "planning",
                "source": source_key,
                "zooms": target_zooms,
                "total_tiles": total_planned_tiles,
                "completed_tiles": 0,
                "failed_tiles": 0,
                "total_bytes": 0,
                "workers": worker_count,
                "tiles_clamped": tiles_clamped,
            }));

            struct DownloadedXyzTile {
                idx: usize,
                z: u32,
                x: u32,
                y: u32,
                img_path: PathBuf,
                src_rect: Option<[u32; 4]>,
                bytes: u64,
                manifest_entry: Value,
            }

            let next_job = std::sync::atomic::AtomicUsize::new(0);
            let completed_atomic = std::sync::atomic::AtomicUsize::new(0);
            let failed_atomic = std::sync::atomic::AtomicUsize::new(0);
            let consecutive_failures = std::sync::atomic::AtomicUsize::new(0);
            let total_bytes_atomic = std::sync::atomic::AtomicU64::new(0);
            let progress_step = (total_planned_tiles / 50).max(1);
            type FallbackParentTileMap =
                std::collections::HashMap<(u32, u32, u32), (String, Vec<u8>)>;
            let fallback_parent_cache: std::sync::Mutex<FallbackParentTileMap> =
                std::sync::Mutex::new(std::collections::HashMap::new());
            let collected_tiles: std::sync::Mutex<Vec<DownloadedXyzTile>> =
                std::sync::Mutex::new(Vec::new());
            let write_error: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
            let abort_flag = std::sync::atomic::AtomicBool::new(false);
            let abort_reason: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
            let record_abort = |reason: String| {
                if let Ok(mut guard) = abort_reason.lock() {
                    if guard.is_none() {
                        *guard = Some(reason);
                    }
                }
                abort_flag.store(true, std::sync::atomic::Ordering::Release);
            };
            let early_failure_threshold = worker_count.clamp(6, 12);

            std::thread::scope(|scope| {
                for _ in 0..worker_count {
                    scope.spawn(|| loop {
                        if abort_flag.load(std::sync::atomic::Ordering::Acquire)
                            || write_error.lock().map(|g| g.is_some()).unwrap_or(true)
                        {
                            break;
                        }
                        let idx = next_job.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if idx >= total_planned_tiles {
                            break;
                        }
                        let (z, x, y) = tile_jobs[idx];
                        let mut final_url = build_tile_url(x, y, z);
                        let mut final_bytes: Option<Vec<u8>> = None;
                        let mut fallback_zoom_used: Option<u32> = None;

                        match fetch_xyz_tile_bytes(&client, &final_url, &abort_flag) {
                            XyzTileFetchOutcome::Ok(bytes) => {
                                final_bytes = Some(bytes);
                            }
                            XyzTileFetchOutcome::RateLimited(code) => {
                                record_abort(format!(
                                    "XYZ_UPSTREAM_RATE_LIMITED: 上游瓦片服务返回 HTTP {code} 限流（{source_key}），已自动中止剩余并发请求。"
                                ));
                                break;
                            }
                            XyzTileFetchOutcome::Forbidden(code) => {
                                record_abort(format!(
                                    "XYZ_UPSTREAM_FORBIDDEN: 上游瓦片服务拒绝访问（HTTP {code}，{source_key}），已中止剩余并发请求。"
                                ));
                                break;
                            }
                            XyzTileFetchOutcome::NotFoundOrEmpty
                            | XyzTileFetchOutcome::TransientError => {}
                        }

                        // Super-zoom fallback: if provider returns 404/empty at high zoom,
                        // automatically fall back to nearest parent tile (bounded shift <= 4) and crop+upsample the exact child sub-quadrant.
                        if final_bytes.is_none()
                            && !abort_flag.load(std::sync::atomic::Ordering::Acquire)
                            && z > 18
                        {
                            let min_fb_z = z.saturating_sub(4).max(14);
                            for fb_z in (min_fb_z..z).rev() {
                                if abort_flag.load(std::sync::atomic::Ordering::Acquire) {
                                    break;
                                }
                                let shift = z - fb_z;
                                let fb_x = x >> shift;
                                let fb_y = y >> shift;
                                let cached = fallback_parent_cache
                                    .lock()
                                    .ok()
                                    .and_then(|c| c.get(&(fb_z, fb_x, fb_y)).cloned());
                                if let Some((cached_url, cached_bytes)) = cached {
                                    final_url = cached_url;
                                    final_bytes = Some(cached_bytes);
                                    fallback_zoom_used = Some(fb_z);
                                    break;
                                }
                                let fb_url = build_tile_url(fb_x, fb_y, fb_z);
                                match fetch_xyz_tile_bytes(&client, &fb_url, &abort_flag) {
                                    XyzTileFetchOutcome::Ok(vec_b) => {
                                        if let Ok(mut cache) = fallback_parent_cache.lock() {
                                            cache.insert(
                                                (fb_z, fb_x, fb_y),
                                                (fb_url.clone(), vec_b.clone()),
                                            );
                                        }
                                        final_url = fb_url;
                                        final_bytes = Some(vec_b);
                                        fallback_zoom_used = Some(fb_z);
                                        break;
                                    }
                                    XyzTileFetchOutcome::RateLimited(code) => {
                                        record_abort(format!(
                                            "XYZ_UPSTREAM_RATE_LIMITED: 上游瓦片服务返回 HTTP {code} 限流（{source_key}），已自动中止剩余并发请求。"
                                        ));
                                        break;
                                    }
                                    XyzTileFetchOutcome::Forbidden(code) => {
                                        record_abort(format!(
                                            "XYZ_UPSTREAM_FORBIDDEN: 上游瓦片服务拒绝访问（HTTP {code}，{source_key}），已中止剩余并发请求。"
                                        ));
                                        break;
                                    }
                                    XyzTileFetchOutcome::NotFoundOrEmpty
                                    | XyzTileFetchOutcome::TransientError => {}
                                }
                            }
                        }

                        if abort_flag.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }

                        let Some(bytes) = final_bytes else {
                            let failed = failed_atomic
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                + 1;
                            let streak = consecutive_failures
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                + 1;
                            let done = completed_atomic.load(std::sync::atomic::Ordering::Relaxed);
                            if done == 0 && streak >= early_failure_threshold {
                                record_abort(format!(
                                    "XYZ_UPSTREAM_UNREACHABLE: 连续 {streak} 次瓦片请求失败且无成功响应（{source_key}），已提前中止剩余请求。"
                                ));
                                break;
                            }
                            if (done + failed).is_multiple_of(progress_step)
                                || (done + failed) == total_planned_tiles
                            {
                                on_progress(json!({
                                    "phase": "downloading",
                                    "source": source_key,
                                    "zooms": target_zooms,
                                    "zoom": z,
                                    "x": x,
                                    "y": y,
                                    "completed_tiles": done,
                                    "failed_tiles": failed,
                                    "total_tiles": total_planned_tiles,
                                    "total_bytes": total_bytes_atomic.load(std::sync::atomic::Ordering::Relaxed),
                                }));
                            }
                            continue;
                        };
                        consecutive_failures.store(0, std::sync::atomic::Ordering::Relaxed);

                        let stem = format!("tile_z{z}_x{x}_y{y}");
                        let img_path = out_dir.join(format!("{stem}.{ext}"));
                        let world_path = out_dir.join(format!("{stem}.{world_ext}"));
                        let prj_path = out_dir.join(format!("{stem}.prj"));
                        if let Err(err) = fs::write(&img_path, &bytes) {
                            if let Ok(mut guard) = write_error.lock() {
                                if guard.is_none() {
                                    *guard = Some(format!("WRITE_TILE_FAILED: {err}"));
                                }
                            }
                            break;
                        }

                        let mut subtile_cropped = false;
                        let mut sub_quadrant_info = None;
                        let mut src_rect = None;
                        if let Some(fb_z) = fallback_zoom_used {
                            let shift = z - fb_z;
                            let sub_div = 1u32 << shift;
                            let sub_ix = x & (sub_div - 1);
                            let sub_iy = y & (sub_div - 1);
                            sub_quadrant_info = Some(json!({
                                "parent_zoom": fb_z,
                                "shift": shift,
                                "sub_ix": sub_ix,
                                "sub_iy": sub_iy,
                            }));
                            let engine = discover_native_gis_engine();
                            if engine.is_discovered()
                                && matches!(
                                    engine.crop_and_upsample_subtile(&img_path, sub_ix, sub_iy, shift),
                                    Ok(true)
                                )
                            {
                                subtile_cropped = true;
                            }
                            if !subtile_cropped {
                                let sub_w = (256 / sub_div).max(1);
                                let sub_h = (256 / sub_div).max(1);
                                src_rect = Some([
                                    (sub_ix * sub_w).min(255),
                                    (sub_iy * sub_h).min(255),
                                    sub_w,
                                    sub_h,
                                ]);
                            }
                        }

                        let actual_file_bytes =
                            fs::metadata(&img_path).map(|m| m.len()).unwrap_or(bytes.len() as u64);
                        let [min_x, min_y, max_x, max_y] = tile_to_mercator_bounds(x, y, z);
                        if write_sidecars {
                            let pixel_size_x = (max_x - min_x) / 256.0;
                            let pixel_size_y = -((max_y - min_y) / 256.0);
                            let upper_left_x = min_x + pixel_size_x * 0.5;
                            let upper_left_y = max_y + pixel_size_y * 0.5;
                            let world_content = format!(
                                "{pixel_size_x:.8}\n0.00000000\n0.00000000\n{pixel_size_y:.8}\n{upper_left_x:.6}\n{upper_left_y:.6}\n"
                            );
                            let _ = fs::write(&world_path, world_content);
                            let _ = fs::write(&prj_path, epsg3857_prj);
                        }

                        let running_bytes = total_bytes_atomic
                            .fetch_add(actual_file_bytes, std::sync::atomic::Ordering::Relaxed)
                            + actual_file_bytes;
                        let done = completed_atomic
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                            + 1;
                        let failed = failed_atomic.load(std::sync::atomic::Ordering::Relaxed);
                        if (done + failed).is_multiple_of(progress_step)
                            || (done + failed) == total_planned_tiles
                        {
                            on_progress(json!({
                                "phase": "downloading",
                                "source": source_key,
                                "zooms": target_zooms,
                                "zoom": z,
                                "x": x,
                                "y": y,
                                "completed_tiles": done,
                                "failed_tiles": failed,
                                "total_tiles": total_planned_tiles,
                                "total_bytes": running_bytes,
                            }));
                        }

                        let redacted_url = redact_tile_url_secrets(&final_url);
                        let entry = json!({
                            "z": z,
                            "x": x,
                            "y": y,
                            "url": redacted_url,
                            "fallback_zoom": fallback_zoom_used,
                            "subtile_quadrant": sub_quadrant_info,
                            "subtile_cropped": subtile_cropped,
                            "file_path": img_path.to_string_lossy(),
                            "world_file": if write_sidecars {
                                json!(world_path.to_string_lossy())
                            } else {
                                Value::Null
                            },
                            "prj_file": if write_sidecars {
                                json!(prj_path.to_string_lossy())
                            } else {
                                Value::Null
                            },
                            "bytes": actual_file_bytes,
                            "mercator_bounds": [min_x, min_y, max_x, max_y],
                        });
                        if let Ok(mut guard) = collected_tiles.lock() {
                            guard.push(DownloadedXyzTile {
                                idx,
                                z,
                                x,
                                y,
                                img_path,
                                src_rect,
                                bytes: actual_file_bytes,
                                manifest_entry: entry,
                            });
                        }
                    });
                }
            });

            if let Ok(mut err_guard) = write_error.lock() {
                if let Some(err) = err_guard.take() {
                    return Err(err);
                }
            }
            let aborted_reason = abort_reason.lock().ok().and_then(|mut g| g.take());

            let mut tiles_sorted = collected_tiles
                .into_inner()
                .map_err(|_| "XYZ_TILE_COLLECTION_POISONED".to_owned())?;
            if tiles_sorted.is_empty() {
                return Err(aborted_reason.unwrap_or_else(|| {
                    format!(
                        "XYZ_TILES_ALL_FAILED: 计划下载 {total_planned_tiles} 张瓦片但全部失败（{source_key}）"
                    )
                }));
            }
            tiles_sorted.sort_by_key(|t| t.idx);

            let stitch_zoom = tiles_sorted
                .iter()
                .map(|t| t.z)
                .max()
                .unwrap_or(primary_zoom);
            let mut downloaded_tiles = Vec::with_capacity(tiles_sorted.len());
            let mut all_tile_files: Vec<(u32, u32, u32, PathBuf)> =
                Vec::with_capacity(tiles_sorted.len());
            let mut tile_files_for_stitch: Vec<(u32, u32, PathBuf)> = Vec::new();
            let mut tile_src_rects: std::collections::HashMap<(u32, u32), [u32; 4]> =
                std::collections::HashMap::new();
            let mut total_bytes: u64 = 0;

            for item in tiles_sorted {
                total_bytes += item.bytes;
                all_tile_files.push((item.z, item.x, item.y, item.img_path.clone()));
                if item.z == stitch_zoom {
                    if let Some(rect) = item.src_rect {
                        tile_src_rects.insert((item.x, item.y), rect);
                    }
                    tile_files_for_stitch.push((item.x, item.y, item.img_path));
                }
                downloaded_tiles.push(item.manifest_entry);
            }

            let failed_tiles_count = failed_atomic.load(std::sync::atomic::Ordering::Relaxed);
            on_progress(json!({
                "phase": "done",
                "source": source_key,
                "zooms": target_zooms,
                "completed_tiles": downloaded_tiles.len(),
                "failed_tiles": failed_tiles_count,
                "total_tiles": total_planned_tiles,
                "total_bytes": total_bytes,
            }));

            let mut mosaic_tif_path_str: Option<String> = None;
            let mut exported_mosaic_path_str: Option<String> = None;
            let mut mosaic_vrt_path_str: Option<String> = None;
            let mut mbtiles_path_str: Option<String> = None;
            let mut zip_archive_path_str: Option<String> = None;
            let mut computed_grid_wgs84_bbox: Option<[f64; 4]> = None;

            if !tile_files_for_stitch.is_empty() {
                let actual_x_min = tile_files_for_stitch.iter().map(|(x, _, _)| *x).min().unwrap_or(x_min);
                let actual_x_max = tile_files_for_stitch.iter().map(|(x, _, _)| *x).max().unwrap_or(x_max);
                let actual_y_min = tile_files_for_stitch.iter().map(|(_, y, _)| *y).min().unwrap_or(y_min);
                let actual_y_max = tile_files_for_stitch.iter().map(|(_, y, _)| *y).max().unwrap_or(y_max);

                let n_tiles = (1u64 << stitch_zoom) as f64;
                let tile_lon = |tx: u32| -> f64 { (tx as f64) / n_tiles * 360.0 - 180.0 };
                let tile_lat = |ty: u32| -> f64 {
                    (std::f64::consts::PI * (1.0 - 2.0 * (ty as f64) / n_tiles))
                        .sinh()
                        .atan()
                        .to_degrees()
                };
                let grid_wgs84 = [
                    tile_lon(actual_x_min),
                    tile_lat(actual_y_max + 1),
                    tile_lon(actual_x_max + 1),
                    tile_lat(actual_y_min),
                ];
                computed_grid_wgs84_bbox = Some(grid_wgs84);

                let [ul_min_x, _ul_min_y, ul_max_x, ul_max_y] =
                    tile_to_mercator_bounds(actual_x_min, actual_y_min, stitch_zoom);
                let px_x = (ul_max_x - ul_min_x) / 256.0;
                let px_y = -((ul_max_y - _ul_min_y) / 256.0);
                let geotransform = [ul_min_x, px_x, 0.0, ul_max_y, 0.0, px_y];
                let total_w = (actual_x_max - actual_x_min + 1) * 256;
                let total_h = (actual_y_max - actual_y_min + 1) * 256;

                // Emit standard GDAL VRT mosaic file unless raw/archive download > 50k tiles
                let should_write_vrt = will_stitch_raster
                    || export_format == "vrt"
                    || (!stitch_mosaic
                        && export_format != "mbtiles"
                        && export_format != "zip"
                        && tile_files_for_stitch.len() <= XYZ_MAX_TILES_MOSAIC);
                if should_write_vrt {
                    let vrt_path = out_dir.join(format!("mosaic_{source_key}_{zoom_tag}.vrt"));
                    let mut vrt_xml = format!(
                        "<VRTDataset rasterXSize=\"{total_w}\" rasterYSize=\"{total_h}\">\n  <SRS>{epsg3857_prj}</SRS>\n  <GeoTransform>{:.8}, {:.8}, 0.0, {:.8}, 0.0, {:.8}</GeoTransform>\n",
                        geotransform[0], geotransform[1], geotransform[3], geotransform[5]
                    );
                    let band_colors = [(1, "Red"), (2, "Green"), (3, "Blue")];
                    for (b, color) in band_colors {
                        vrt_xml.push_str(&format!(
                            "  <VRTRasterBand dataType=\"Byte\" band=\"{b}\">\n    <ColorInterp>{color}</ColorInterp>\n"
                        ));
                        for (tx, ty, tpath) in &tile_files_for_stitch {
                            let fname = tpath.file_name().and_then(|s| s.to_str()).unwrap_or("");
                            let dst_x = (tx - actual_x_min) * 256;
                            let dst_y = (ty - actual_y_min) * 256;
                            let [sx, sy, sw, sh] = tile_src_rects.get(&(*tx, *ty)).copied().unwrap_or([0, 0, 256, 256]);
                            vrt_xml.push_str(&format!(
                                "    <SimpleSource>\n      <SourceFilename relativeToVRT=\"1\">{fname}</SourceFilename>\n      <SourceBand>{b}</SourceBand>\n      <SrcRect xOff=\"{sx}\" yOff=\"{sy}\" xSize=\"{sw}\" ySize=\"{sh}\"/>\n      <DstRect xOff=\"{dst_x}\" yOff=\"{dst_y}\" xSize=\"256\" ySize=\"256\"/>\n    </SimpleSource>\n"
                            ));
                        }
                        vrt_xml.push_str("  </VRTRasterBand>\n");
                    }
                    vrt_xml.push_str("</VRTDataset>\n");
                    if fs::write(&vrt_path, vrt_xml).is_ok() {
                        mosaic_vrt_path_str = Some(vrt_path.to_string_lossy().to_string());
                    }
                }

                if will_stitch_raster {
                    let tif_path = out_dir.join(format!("mosaic_{source_key}_{zoom_tag}.tif"));
                    let stitch_fmt = match export_format.as_str() {
                        "mbtiles" | "zip" | "vrt" => "gtiff",
                        other => other,
                    };
                    let engine = discover_native_gis_engine();
                    if engine.is_discovered() {
                        let stitch_res = engine
                            .stitch_xyz_tiles_to_geotiff(
                                &tile_files_for_stitch,
                                actual_x_min,
                                actual_x_max,
                                actual_y_min,
                                actual_y_max,
                                geotransform,
                                epsg3857_prj,
                                &tif_path,
                                &compression,
                                build_pyramids,
                                stitch_fmt,
                            )
                            .map_err(|err| format!("{}: {err}", err.code().as_str()))?;
                        if stitch_res["ok"] == true && tif_path.is_file() {
                            mosaic_tif_path_str = Some(tif_path.to_string_lossy().to_string());
                        }
                        if let Some(exp) = stitch_res["exported_extra_path"]
                            .as_str()
                            .or_else(|| stitch_res["exported_path"].as_str())
                        {
                            exported_mosaic_path_str = Some(exp.to_owned());
                        } else if export_format == "vrt" {
                            exported_mosaic_path_str = mosaic_vrt_path_str.clone();
                        } else {
                            exported_mosaic_path_str = mosaic_tif_path_str.clone();
                        }
                    } else if export_format == "vrt" {
                        exported_mosaic_path_str = mosaic_vrt_path_str.clone();
                    }
                } else if export_format == "vrt" {
                    exported_mosaic_path_str = mosaic_vrt_path_str.clone();
                }
            }

            // Real MBTiles 1.3 SQLite export (`metadata` + `tiles` with TMS y-flip across all target zooms)
            if export_format == "mbtiles" && !all_tile_files.is_empty() {
                let grid_wgs84 = computed_grid_wgs84_bbox.unwrap_or(query_bbox);
                let mbt_path = out_dir.join(format!("mosaic_{source_key}_{zoom_tag}.mbtiles"));
                let _ = fs::remove_file(&mbt_path);
                let mut conn = rusqlite::Connection::open(&mbt_path)
                    .map_err(|e| format!("MBTILES_OPEN_FAILED: {e}"))?;
                conn.execute_batch(
                    "CREATE TABLE metadata (name text, value text);
                     CREATE UNIQUE INDEX name ON metadata (name);
                     CREATE TABLE tiles (zoom_level integer, tile_column integer, tile_row integer, tile_data blob);
                     CREATE UNIQUE INDEX tile_index ON tiles (zoom_level, tile_column, tile_row);",
                )
                .map_err(|e| format!("MBTILES_SCHEMA_FAILED: {e}"))?;

                let min_zoom = all_tile_files.iter().map(|(z, ..)| *z).min().unwrap_or(primary_zoom);
                let max_zoom = all_tile_files.iter().map(|(z, ..)| *z).max().unwrap_or(primary_zoom);
                let bounds_str = format!(
                    "{:.6},{:.6},{:.6},{:.6}",
                    grid_wgs84[0], grid_wgs84[1], grid_wgs84[2], grid_wgs84[3]
                );
                let center_str = format!(
                    "{:.6},{:.6},{}",
                    (grid_wgs84[0] + grid_wgs84[2]) * 0.5,
                    (grid_wgs84[1] + grid_wgs84[3]) * 0.5,
                    max_zoom
                );
                let mbt_format = if ext == "png" { "png" } else { "jpg" };
                let meta_rows = [
                    ("name", format!("EOStage_{source_key}_{}", zoom_tag.to_ascii_uppercase())),
                    ("type", "baselayer".to_owned()),
                    ("version", "1.3".to_owned()),
                    (
                        "description",
                        format!("XYZ satellite tiles ({source_key}) at zooms {target_zooms:?}"),
                    ),
                    ("format", mbt_format.to_owned()),
                    ("bounds", bounds_str),
                    ("center", center_str),
                    ("minzoom", min_zoom.to_string()),
                    ("maxzoom", max_zoom.to_string()),
                ];
                let tx = conn
                    .transaction()
                    .map_err(|e| format!("MBTILES_TX_BEGIN_FAILED: {e}"))?;
                for (k, v) in &meta_rows {
                    tx.execute(
                        "INSERT OR REPLACE INTO metadata (name, value) VALUES (?1, ?2)",
                        rusqlite::params![k, v],
                    )
                    .map_err(|e| format!("MBTILES_METADATA_INSERT_FAILED: {e}"))?;
                }
                for (tile_z, tile_x, tile_y, img_path) in &all_tile_files {
                    let tile_bytes = fs::read(img_path)
                        .map_err(|e| format!("MBTILES_READ_TILE_FAILED: {e}"))?;
                    // MBTiles 1.3 uses OSGeo TMS row numbering: tile_row = (2^z - 1) - y
                    let max_y_idx = (1u32 << tile_z) - 1;
                    let tms_row = max_y_idx - *tile_y;
                    tx.execute(
                        "INSERT OR REPLACE INTO tiles (zoom_level, tile_column, tile_row, tile_data) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![*tile_z as i64, *tile_x as i64, tms_row as i64, tile_bytes],
                    )
                    .map_err(|e| format!("MBTILES_TILE_INSERT_FAILED: {e}"))?;
                }
                tx.commit()
                    .map_err(|e| format!("MBTILES_TX_COMMIT_FAILED: {e}"))?;
                let mbt_str = mbt_path.to_string_lossy().to_string();
                mbtiles_path_str = Some(mbt_str.clone());
                exported_mosaic_path_str = Some(mbt_str);
            }

            if export_format == "zip" && !all_tile_files.is_empty() {
                let zip_path = out_dir.join(format!("tiles_{source_key}_{zoom_tag}.zip"));
                let zip_str = zip_path.to_string_lossy().to_string();
                zip_archive_path_str = Some(zip_str.clone());
                exported_mosaic_path_str = Some(zip_str);
            }

            let primary_file_path = exported_mosaic_path_str
                .clone()
                .or_else(|| mosaic_tif_path_str.clone())
                .or_else(|| mosaic_vrt_path_str.clone())
                .or_else(|| {
                    downloaded_tiles
                        .first()
                        .and_then(|t| t["file_path"].as_str().map(str::to_owned))
                });

            let manifest_path = out_dir.join("tiles_manifest.json");
            let mut manifest = json!({
                "ok": !downloaded_tiles.is_empty(),
                "dataset": "xyz_satellite_tiles",
                "source": source_key,
                "wayback_version_id": wayback::parse_wayback_version_id(&source_key),
                "tile_url_template": wayback::parse_wayback_version_id(&source_key).map(|wid| format!("{}/{wid}/{{z}}/{{y}}/{{x}}", wayback::WAYBACK_TILE_BASE)),
                "custom_url_template": if matches!(source_key.as_str(), "custom" | "custom_xyz" | "custom_tms") {
                    Some(redact_tile_url_secrets(&custom_url_template))
                } else {
                    None
                },
                "zoom": primary_zoom,
                "zooms": target_zooms,
                "source_max_zoom": source_max_zoom,
                "workers": worker_count,
                "max_tiles_limit": max_tiles_limit,
                "crs": "EPSG:3857",
                "gcj02_offset_corrected": is_gcj02,
                "export_format": export_format,
                "compression": compression,
                "pyramids_built": build_pyramids,
                "requested_bbox_wgs84": bbox,
                "tiles_clamped": tiles_clamped,
                "tile_grid": {
                    "x_min": x_min,
                    "x_max": x_max,
                    "y_min": y_min,
                    "y_max": y_max,
                },
                "planned_tile_count": total_planned_tiles,
                "failed_tile_count": failed_tiles_count,
                "aborted_reason": aborted_reason,
                "tile_count": downloaded_tiles.len(),
                "total_bytes": total_bytes,
                "output_dir": out_dir.to_string_lossy(),
                "downloaded_file": primary_file_path,
                "downloaded_path": primary_file_path,
                "mosaic_geotiff_path": mosaic_tif_path_str,
                "exported_mosaic_path": exported_mosaic_path_str,
                "mosaic_vrt_path": mosaic_vrt_path_str,
                "mbtiles_path": mbtiles_path_str,
                "zip_archive_path": zip_archive_path_str,
                "manifest_path": manifest_path.to_string_lossy(),
                "tiles": downloaded_tiles,
            });
            if let Some(ref tif_s) = mosaic_tif_path_str {
                attach_raster_preview_fields(&mut manifest, Path::new(tif_s));
            } else if let Some(ref vrt_s) = mosaic_vrt_path_str {
                attach_raster_preview_fields(&mut manifest, Path::new(vrt_s));
            }
            if manifest.get("mosaic_bbox").is_none() {
                if let Some(grid_bbox) = computed_grid_wgs84_bbox {
                    manifest["mosaic_bbox"] = json!(grid_bbox);
                }
            }
            let manifest_pretty = serde_json::to_string_pretty(&manifest).unwrap_or_default();
            let _ = fs::write(&manifest_path, &manifest_pretty);

            // Real ZIP archive packaging (`{z}/{x}/{y}.{ext}` + sidecars + stitched `.tif`/`.vrt` + `tiles_manifest.json`)
            if let Some(ref zip_s) = zip_archive_path_str {
                use std::io::Write;
                let zip_path = Path::new(zip_s);
                let _ = fs::remove_file(zip_path);
                let zfile = fs::File::create(zip_path)
                    .map_err(|e| format!("ZIP_CREATE_FAILED: {e}"))?;
                let mut zwriter = zip::ZipWriter::new(zfile);
                let zopts = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);

                for (tile_z, tile_x, tile_y, img_path) in &all_tile_files {
                    let tile_bytes = fs::read(img_path)
                        .map_err(|e| format!("ZIP_READ_TILE_FAILED: {e}"))?;
                    let entry_name = format!("{tile_z}/{tile_x}/{tile_y}.{ext}");
                    zwriter
                        .start_file(entry_name, zopts)
                        .map_err(|e| format!("ZIP_ENTRY_FAILED: {e}"))?;
                    zwriter
                        .write_all(&tile_bytes)
                        .map_err(|e| format!("ZIP_WRITE_FAILED: {e}"))?;

                    if write_sidecars {
                        let stem = format!("tile_z{tile_z}_x{tile_x}_y{tile_y}");
                        for side_ext in [world_ext, "prj"] {
                            let side_path = out_dir.join(format!("{stem}.{side_ext}"));
                            if let Ok(side_bytes) = fs::read(&side_path) {
                                let side_entry = format!("{tile_z}/{tile_x}/{tile_y}.{side_ext}");
                                zwriter
                                    .start_file(side_entry, zopts)
                                    .map_err(|e| format!("ZIP_SIDE_ENTRY_FAILED: {e}"))?;
                                zwriter
                                    .write_all(&side_bytes)
                                    .map_err(|e| format!("ZIP_SIDE_WRITE_FAILED: {e}"))?;
                            }
                        }
                    }
                }
                if let Some(ref tif_s) = mosaic_tif_path_str {
                    let tif_p = Path::new(tif_s);
                    if let (Some(fname), Ok(mut tif_file)) = (
                        tif_p.file_name().and_then(|n| n.to_str()),
                        fs::File::open(tif_p),
                    ) {
                        let tif_len = fs::metadata(tif_p).map(|m| m.len()).unwrap_or(0);
                        let tif_zopts = if tif_len >= 0xFFFF_FFFF {
                            zopts.large_file(true)
                        } else {
                            zopts
                        };
                        zwriter
                            .start_file(fname, tif_zopts)
                            .map_err(|e| format!("ZIP_TIF_ENTRY_FAILED: {e}"))?;
                        std::io::copy(&mut tif_file, &mut zwriter)
                            .map_err(|e| format!("ZIP_TIF_WRITE_FAILED: {e}"))?;
                    }
                }
                if let Some(ref vrt_s) = mosaic_vrt_path_str {
                    let vrt_p = Path::new(vrt_s);
                    if let (Some(fname), Ok(vrt_bytes)) = (
                        vrt_p.file_name().and_then(|n| n.to_str()),
                        fs::read(vrt_p),
                    ) {
                        zwriter
                            .start_file(fname, zopts)
                            .map_err(|e| format!("ZIP_VRT_ENTRY_FAILED: {e}"))?;
                        zwriter
                            .write_all(&vrt_bytes)
                            .map_err(|e| format!("ZIP_VRT_WRITE_FAILED: {e}"))?;
                    }
                }
                zwriter
                    .start_file("tiles_manifest.json", zopts)
                    .map_err(|e| format!("ZIP_MANIFEST_ENTRY_FAILED: {e}"))?;
                zwriter
                    .write_all(manifest_pretty.as_bytes())
                    .map_err(|e| format!("ZIP_MANIFEST_WRITE_FAILED: {e}"))?;
                zwriter
                    .finish()
                    .map_err(|e| format!("ZIP_FINISH_FAILED: {e}"))?;
            }

            const MAX_IPC_TILES: usize = 2_000;
            let total_tile_count = downloaded_tiles.len();
            manifest["tiles_total_count"] = json!(total_tile_count);
            if total_tile_count > MAX_IPC_TILES {
                manifest["tiles"] = Value::Array(downloaded_tiles[..MAX_IPC_TILES].to_vec());
                manifest["tiles_truncated"] = json!(true);
            } else {
                manifest["tiles_truncated"] = json!(false);
            }

            Ok(manifest)
        }
        "mpc_stac_science" | "esa_worldcover" | "jrc_gsw" | "alos_palsar_mosaic" | "modis_ndvi" | "sentinel_5p" => {
            let raw_bbox = req
                .bbox
                .ok_or_else(|| "MISSING_BBOX: MPC STAC 检索需要 bbox=[west, south, east, north]".to_owned())?;
            let bbox = [
                raw_bbox[0].min(raw_bbox[2]),
                raw_bbox[1].min(raw_bbox[3]),
                raw_bbox[0].max(raw_bbox[2]),
                raw_bbox[1].max(raw_bbox[3]),
            ];
            let collection = match dataset.as_str() {
                "esa_worldcover" => "esa-worldcover",
                "jrc_gsw" => "jrc-gsw",
                "alos_palsar_mosaic" => "alos-palsar-mosaic",
                "modis_ndvi" => req.dem_product.as_deref().filter(|s| !s.is_empty()).unwrap_or("modis-13Q1-061"),
                "sentinel_5p" => "sentinel-5p-l2-netcdf",
                _ => req.code.as_deref().or(req.dem_product.as_deref()).unwrap_or("esa-worldcover"),
            };

            // Parse pipe-delimited keyword `${startDate}..${endDate}|${subProduct}|download`
            let mut kw_start_date: Option<String> = None;
            let mut kw_end_date: Option<String> = None;
            let mut kw_sub_product: Option<String> = None;
            let mut kw_custom_stac: Option<String> = None;
            let mut kw_download = false;
            let mut kw_search_only = false;
            if let Some(raw_kw) = req.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                for token in raw_kw.split('|').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some((s, e)) = token.split_once("..") {
                        let s_trim = s.trim();
                        let e_trim = e.trim();
                        if !s_trim.is_empty() {
                            kw_start_date = Some(s_trim.to_owned());
                        }
                        if !e_trim.is_empty() {
                            kw_end_date = Some(e_trim.to_owned());
                        }
                    } else if token.starts_with("http://") || token.starts_with("https://") {
                        kw_custom_stac = Some(token.to_owned());
                    } else if token.eq_ignore_ascii_case("download") {
                        kw_download = true;
                    } else if token.eq_ignore_ascii_case("search")
                        || token.eq_ignore_ascii_case("preview")
                    {
                        kw_search_only = true;
                    } else if token.eq_ignore_ascii_case("ascending")
                        || token.eq_ignore_ascii_case("descending")
                    {
                        // orbit direction token handled by parse_orbit_filter_token
                    } else if kw_sub_product.is_none() {
                        kw_sub_product = Some(token.to_owned());
                    }
                }
            }
            if collection == "sentinel-2-l2a" {
                if let Some(ref sp) = kw_sub_product {
                    let normalized = match sp.to_ascii_uppercase().as_str() {
                        "B1" => Some("B01"),
                        "B2" => Some("B02"),
                        "B3" => Some("B03"),
                        "B4" => Some("B04"),
                        "B5" => Some("B05"),
                        "B6" => Some("B06"),
                        "B7" => Some("B07"),
                        "B8" => Some("B08"),
                        "B9" => Some("B09"),
                        _ => None,
                    };
                    if let Some(n_band) = normalized {
                        kw_sub_product = Some(n_band.to_owned());
                    }
                }
            }
            let asset_pref = kw_sub_product.as_deref();

            let client = build_http_client(app_data_dir)?;
            let mut search_body = json!({
                "collections": [collection],
                "bbox": bbox,
                "limit": if matches!(
                    collection,
                    "modis-13Q1-061"
                        | "modis-11A2-061"
                        | "esa-worldcover"
                        | "jrc-gsw"
                        | "alos-palsar-mosaic"
                        | "sentinel-5p-l2-netcdf"
                ) {
                    100
                } else {
                    50
                },
            });
            let explicit_start = req
                .start_date
                .as_deref()
                .filter(|v| !v.is_empty())
                .or(kw_start_date.as_deref());
            let explicit_end = req
                .end_date
                .as_deref()
                .filter(|v| !v.is_empty())
                .or(kw_end_date.as_deref());
            if matches!(
                collection,
                "modis-13Q1-061"
                    | "modis-11A2-061"
                    | "sentinel-5p-l2-netcdf"
                    | "sentinel-2-l2a"
                    | "landsat-c2-l2"
                    | "sentinel-1-grd"
            ) {
                let s = explicit_start.unwrap_or("2023-01-01");
                let e = explicit_end.unwrap_or("2026-12-31");
                search_body["datetime"] = json!(format!("{s}T00:00:00Z/{e}T23:59:59Z"));
            } else if collection == "io-lulc-annual-v02" {
                // Planetary Computer `io-lulc-annual-v02` publishes annual composites up to 2023.
                // Apply explicit year filtering when within published epochs; otherwise query latest epoch.
                if let (Some(s), Some(e)) = (explicit_start, explicit_end) {
                    if s <= "2023-12-31" {
                        search_body["datetime"] = json!(format!("{s}T00:00:00Z/{e}T23:59:59Z"));
                    }
                }
            }
            let mpc_base = read_loopback_mpc_base_url(app_data_dir)
                .unwrap_or_else(|| "https://planetarycomputer.microsoft.com".to_owned());
            let default_stac_endpoint = format!("{mpc_base}/api/stac/v1/search");
            let stac_endpoint = kw_custom_stac
                .as_deref()
                .unwrap_or(default_stac_endpoint.as_str());
            let mut stac_resp_opt: Option<Value> = None;
            let mut last_stac_err = String::new();
            for attempt in 0..4 {
                match client.post(stac_endpoint).json(&search_body).send() {
                    Ok(resp) if resp.status().is_success() => match resp.json::<Value>() {
                        Ok(v) => {
                            stac_resp_opt = Some(v);
                            break;
                        }
                        Err(err) => {
                            last_stac_err = format!("MPC_STAC_JSON_FAILED ({collection}): {err}");
                        }
                    },
                    Ok(resp) => {
                        last_stac_err = format!("MPC_STAC_HTTP_ERROR ({collection}): HTTP {}", resp.status());
                        if !resp.status().is_server_error() && resp.status().as_u16() != 429 {
                            break;
                        }
                    }
                    Err(err) => {
                        last_stac_err = format!("MPC_STAC_SEARCH_FAILED ({collection}): {err}");
                    }
                }
                if attempt < 3 {
                    std::thread::sleep(Duration::from_millis(1000 * (attempt as u64 + 1)));
                }
            }
            let stac_resp: Value = stac_resp_opt.ok_or(last_stac_err)?;

            let sas_token = if kw_custom_stac.is_none() {
                let token_url = format!("{mpc_base}/api/sas/v1/token/{collection}");
                http_get_with_retry(&client, &token_url, None, 3)
                    .ok()
                    .filter(|r| r.status().is_success())
                    .and_then(|r| r.json::<Value>().ok())
                    .and_then(|v| v["token"].as_str().map(str::to_owned))
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let mut features = stac_resp["features"].as_array().cloned().unwrap_or_default();

            // Sub-product feature filtering for esa-worldcover (v100 / 2020 vs v200 / 2021)
            // and sentinel-5p-l2-netcdf (each STAC item corresponds to a specific trace-gas product)
            if collection == "esa-worldcover" {
                if let Some(pref) = asset_pref {
                    let pref_lower = pref.to_ascii_lowercase();
                    if pref_lower.contains("v100") || pref_lower.contains("2020") {
                        features.retain(|f| {
                            let id_lower = f["id"].as_str().unwrap_or("").to_ascii_lowercase();
                            let ver_lower = f["properties"]["esa_worldcover:product_version"]
                                .as_str()
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            let dt = f["properties"]["datetime"]
                                .as_str()
                                .or_else(|| f["properties"]["start_datetime"].as_str())
                                .unwrap_or("");
                            id_lower.contains("2020")
                                || id_lower.contains("v100")
                                || ver_lower.contains("1.0")
                                || ver_lower.contains("v100")
                                || dt.starts_with("2020")
                        });
                    } else if pref_lower.contains("v200") || pref_lower.contains("2021") {
                        features.retain(|f| {
                            let id_lower = f["id"].as_str().unwrap_or("").to_ascii_lowercase();
                            let ver_lower = f["properties"]["esa_worldcover:product_version"]
                                .as_str()
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            let dt = f["properties"]["datetime"]
                                .as_str()
                                .or_else(|| f["properties"]["start_datetime"].as_str())
                                .unwrap_or("");
                            id_lower.contains("2021")
                                || id_lower.contains("v200")
                                || ver_lower.contains("2.0")
                                || ver_lower.contains("v200")
                                || dt.starts_with("2021")
                        });
                    }
                }
            } else if collection == "sentinel-5p-l2-netcdf" {
                if let Some(pref) = asset_pref {
                    let pref_lower = pref.to_ascii_lowercase();
                    let filtered: Vec<Value> = features
                        .iter()
                        .filter(|f| {
                            f["assets"].get(pref).is_some()
                                || f["assets"].get(&pref_lower).is_some()
                                || f["properties"]["s5p:product_name"]
                                    .as_str()
                                    .is_some_and(|p| p.eq_ignore_ascii_case(pref) || p.eq_ignore_ascii_case(&pref_lower))
                        })
                        .cloned()
                        .collect();
                    if !filtered.is_empty() {
                        features = filtered;
                    }
                }
            }

            let mut items = Vec::new();
            for feat in &features {
                let id = feat["id"].as_str().unwrap_or("unknown").to_owned();
                let dt = feat["properties"]["datetime"]
                    .as_str()
                    .or_else(|| feat["properties"]["start_datetime"].as_str())
                    .unwrap_or("2021-01-01T00:00:00Z")
                    .to_owned();
                let assets = &feat["assets"];
                let safe_pref = asset_pref.filter(|k| *k != "rendered_preview" && *k != "tilejson");
                let candidate_keys: Vec<&str> = match collection {
                    "esa-worldcover" => vec!["map"],
                    "io-lulc-annual-v02" => {
                        if let Some(k) = safe_pref {
                            vec![k, "data"]
                        } else {
                            vec!["data"]
                        }
                    }
                    "jrc-gsw" => {
                        if let Some(k) = safe_pref {
                            vec![k, "occurrence", "change", "seasonality", "recurrence", "transitions", "extent"]
                        } else {
                            vec!["occurrence", "change", "seasonality", "recurrence", "extent"]
                        }
                    }
                    "alos-palsar-mosaic" => {
                        if let Some(k) = safe_pref {
                            vec![k, "HH", "HV", "mask", "linci"]
                        } else {
                            vec!["HH", "HV", "mask"]
                        }
                    }
                    "modis-13Q1-061" => {
                        if let Some(k) = safe_pref {
                            vec![k, "250m_16_days_NDVI", "250m_16_days_EVI", "250m_16_days_VI_Quality", "hdf"]
                        } else {
                            vec!["250m_16_days_NDVI", "250m_16_days_EVI", "hdf"]
                        }
                    }
                    "sentinel-5p-l2-netcdf" => {
                        if let Some(k) = safe_pref {
                            vec![k, "netcdf", "so2", "no2", "ch4", "co", "o3", "aer_ai"]
                        } else {
                            vec!["netcdf", "so2", "no2", "ch4", "co", "o3"]
                        }
                    }
                    "sentinel-2-l2a" => {
                        if let Some(k) = safe_pref {
                            vec![k, "visual", "B04", "B08"]
                        } else {
                            vec!["visual", "B04", "B08"]
                        }
                    }
                    "landsat-c2-l2" => {
                        if let Some(k) = safe_pref {
                            vec![k, "red", "green", "blue", "nir08", "swir16"]
                        } else {
                            vec!["red", "green", "blue", "nir08", "swir16"]
                        }
                    }
                    "sentinel-1-grd" => {
                        if let Some(k) = safe_pref {
                            vec![k, "vv", "vh", "hh", "hv"]
                        } else {
                            vec!["vv", "vh", "hh", "hv"]
                        }
                    }
                    "cop-dem-glo-30" | "nasadem" => {
                        if let Some(k) = safe_pref {
                            vec![k, "data", "elevation"]
                        } else {
                            vec!["data", "elevation"]
                        }
                    }
                    _ => {
                        if let Some(k) = safe_pref {
                            vec![k, "map", "data", "visual", "HH", "occurrence"]
                        } else {
                            vec!["map", "data", "visual", "HH", "occurrence"]
                        }
                    }
                };
                let mut chosen_key = String::new();
                let mut raw_href = String::new();
                for k in candidate_keys {
                    if let Some(href) = assets[k]["href"].as_str() {
                        chosen_key = k.to_owned();
                        raw_href = href.to_owned();
                        break;
                    }
                }
                if raw_href.is_empty() {
                    if let Some(obj) = assets.as_object() {
                        for (k, v) in obj {
                            if k == "rendered_preview" || k == "tilejson" {
                                continue;
                            }
                            if let Some(href) = v["href"].as_str() {
                                let clean = href.split('?').next().unwrap_or("");
                                if clean.ends_with(".tif")
                                    || clean.ends_with(".tiff")
                                    || clean.ends_with(".nc")
                                    || clean.ends_with(".hdf")
                                    || clean.ends_with(".h5")
                                {
                                    chosen_key = k.clone();
                                    raw_href = href.to_owned();
                                    break;
                                }
                            }
                        }
                    }
                }
                let signed_href = if !raw_href.is_empty() && !sas_token.is_empty() && raw_href.contains("blob.core.windows.net") {
                    if raw_href.contains('?') {
                        format!("{raw_href}&{sas_token}")
                    } else {
                        format!("{raw_href}?{sas_token}")
                    }
                } else {
                    raw_href.clone()
                };
                let tilejson_href = assets["tilejson"]["href"].as_str().unwrap_or("").to_owned();
                let preview_href = assets["rendered_preview"]["href"]
                    .as_str()
                    .or_else(|| assets["tilejson"]["href"].as_str())
                    .unwrap_or("")
                    .to_owned();
                let clean_asset_path = raw_href.split('?').next().unwrap_or("").to_ascii_lowercase();
                let guess_ext = if clean_asset_path.ends_with(".tif") || clean_asset_path.ends_with(".tiff") {
                    "tif"
                } else if clean_asset_path.ends_with(".hdf") || clean_asset_path.ends_with(".h5") {
                    "hdf"
                } else if clean_asset_path.ends_with(".nc") {
                    "nc"
                } else {
                    "bin"
                };
                let existing_local = req
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .map(|dir| dir.join(format!("{id}_{chosen_key}.{guess_ext}")))
                    .and_then(|p| existing_valid_science_file_bytes(&p).map(|sz| (p, sz)));
                items.push(json!({
                    "id": id,
                    "collection": collection,
                    "datetime": dt,
                    "asset_key": chosen_key,
                    "href": raw_href,
                    "signed_href": signed_href,
                    "tilejson_href": tilejson_href,
                    "preview_url": preview_href,
                    "thumbnail_url": preview_href,
                    "already_downloaded": existing_local.is_some(),
                    "local_file_path": existing_local.as_ref().map(|(p, _)| p.to_string_lossy().to_string()),
                    "size_bytes": existing_local.as_ref().map(|(_, sz)| *sz),
                    "bbox": feat["bbox"],
                    "geometry": feat["geometry"],
                }));
            }
            if collection != "sentinel-5p-l2-netcdf" {
                filter_open_geodata_items_by_aoi(&mut items, req.aoi_geojson.as_ref());
            }

            let mut downloaded_file: Option<String> = None;
            let mut downloaded_bytes: u64 = 0;
            let mut already_exists = false;
            let mut target_item_id_out: Option<String> = None;
            let should_download = kw_download
                || req.tool_name.as_deref() == Some("download")
                || (!kw_search_only
                    && req.output_path.as_deref().is_some_and(|s| !s.trim().is_empty()));
            if should_download {
                if items.is_empty() {
                    return Err(format!(
                        "MPC_STAC_NO_SCENES: 在指定区域与时间窗口内未检索到 {collection} 科学影像"
                    ));
                }
                let out_dir = req
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join(collection));
                fs::create_dir_all(&out_dir).map_err(|e| format!("CREATE_DIR_FAILED: {e}"))?;

                let cx = (bbox[0] + bbox[2]) * 0.5;
                let cy = (bbox[1] + bbox[3]) * 0.5;
                let target_item = if let Some(target_id) = req.osm_tag.as_deref().filter(|s| !s.is_empty()) {
                    items.iter().find(|it| it["id"].as_str() == Some(target_id)).unwrap_or(&items[0])
                } else if collection == "sentinel-5p-l2-netcdf" {
                    &items[0]
                } else {
                    items
                        .iter()
                        .find(|it| {
                            if let Some(arr) = it["bbox"].as_array() {
                                if arr.len() >= 4 {
                                    let w = arr[0].as_f64().unwrap_or(0.0);
                                    let s = arr[1].as_f64().unwrap_or(0.0);
                                    let e = arr[2].as_f64().unwrap_or(0.0);
                                    let n = arr[3].as_f64().unwrap_or(0.0);
                                    return cx >= w && cx <= e && cy >= s && cy <= n;
                                }
                            }
                            false
                        })
                        .unwrap_or(&items[0])
                };
                let item_id = target_item["id"].as_str().unwrap_or("scene");
                target_item_id_out = Some(item_id.to_owned());
                let asset_key = target_item["asset_key"].as_str().unwrap_or("data");
                let raw_target_href = target_item["href"].as_str().unwrap_or("");
                let clean_raw_path = raw_target_href.split('?').next().unwrap_or("").to_ascii_lowercase();
                let pre_ext = if clean_raw_path.ends_with(".tif") || clean_raw_path.ends_with(".tiff") {
                    "tif"
                } else if clean_raw_path.ends_with(".hdf") || clean_raw_path.ends_with(".h5") {
                    "hdf"
                } else if clean_raw_path.ends_with(".nc") {
                    "nc"
                } else {
                    "bin"
                };
                let pre_out_file = out_dir.join(format!("{item_id}_{asset_key}.{pre_ext}"));
                if let Some(existing_len) = existing_valid_science_file_bytes(&pre_out_file) {
                    already_exists = true;
                    downloaded_bytes = existing_len;
                    downloaded_file = Some(pre_out_file.to_string_lossy().to_string());
                } else {
                let mut signed_url_owned = target_item["signed_href"].as_str().unwrap_or("").to_owned();
                if !raw_target_href.is_empty() && raw_target_href.contains("blob.core.windows.net") {
                    let sign_url = format!("{mpc_base}/api/sas/v1/sign");
                    if let Ok(sign_resp) = http_get_with_retry(
                        &client,
                        &sign_url,
                        Some(&[("href", raw_target_href)]),
                        3,
                    ) {
                        if sign_resp.status().is_success() {
                            if let Ok(sign_json) = sign_resp.json::<Value>() {
                                if let Some(s_href) = sign_json["href"].as_str() {
                                    if !s_href.is_empty() {
                                        signed_url_owned = s_href.to_owned();
                                    }
                                }
                            }
                        }
                    }
                    if !signed_url_owned.contains("sv=") && !signed_url_owned.contains("sig=") {
                        let token_url = format!("{mpc_base}/api/sas/v1/token/{collection}");
                        if let Ok(tok_resp) = http_get_with_retry(&client, &token_url, None, 3) {
                            if let Ok(tok_json) = tok_resp.json::<Value>() {
                                if let Some(tok) = tok_json["token"].as_str().filter(|s| !s.is_empty()) {
                                    signed_url_owned = if raw_target_href.contains('?') {
                                        format!("{raw_target_href}&{tok}")
                                    } else {
                                        format!("{raw_target_href}?{tok}")
                                    };
                                }
                            }
                        }
                    }
                }
                let signed_url = signed_url_owned.as_str();
                if signed_url.is_empty() {
                    return Err(format!(
                        "MPC_STAC_ASSET_UNAVAILABLE: {collection}/{item_id} 未找到可下载科学资产 ({asset_key})"
                    ));
                }
                if signed_url.contains("blob.core.windows.net")
                    && !signed_url.contains("sv=")
                    && !signed_url.contains("sig=")
                {
                    return Err(format!(
                        "MPC_STAC_SAS_SIGN_FAILED: 无法为 {collection}/{item_id} 获取 Planetary Computer SAS 签名令牌"
                    ));
                }
                let clean_path = signed_url.split('?').next().unwrap_or("").to_ascii_lowercase();
                let is_tif = clean_path.ends_with(".tif") || clean_path.ends_with(".tiff");
                let is_hdf = clean_path.ends_with(".hdf")
                    || clean_path.ends_with(".h5")
                    || signed_url.contains(".hdf");
                let is_nc = clean_path.ends_with(".nc") || signed_url.contains(".nc");
                let ext = if is_tif {
                    "tif"
                } else if is_hdf {
                    "hdf"
                } else if is_nc {
                    "nc"
                } else {
                    "bin"
                };
                let out_file = out_dir.join(format!("{item_id}_{asset_key}.{ext}"));

                // For COG GeoTIFFs, clip the user's FULL requested AOI bbox (no 0.08° silent shrinking!)
                let mut clipped_ok = false;
                let mut clip_err_detail = String::new();
                if is_tif {
                    let engine = discover_native_gis_engine();
                    if engine.is_discovered() {
                        let clip_req = RasterProcessRequest {
                            sources: vec![signed_url.to_owned()],
                            output_path: out_file.clone(),
                            operation: "clip".to_owned(),
                            crop_bbox: Some(bbox),
                            band_a: Some(1),
                            ..Default::default()
                        };
                        for clip_attempt in 0..3 {
                            if clip_attempt > 0 {
                                let _ = fs::remove_file(&out_file);
                                std::thread::sleep(Duration::from_millis(600 * clip_attempt as u64));
                            }
                            match engine.process_raster_window(&clip_req) {
                                Ok(clip_res) if clip_res["ok"] == true && out_file.is_file() => {
                                    downloaded_bytes = fs::metadata(&out_file).map(|m| m.len()).unwrap_or(0);
                                    if downloaded_bytes > 100 {
                                        downloaded_file = Some(out_file.to_string_lossy().to_string());
                                        clipped_ok = true;
                                        break;
                                    }
                                }
                                Ok(_) => {
                                    clip_err_detail = "GDAL clip returned empty file".to_owned();
                                }
                                Err(err) => {
                                    clip_err_detail = format!("{}: {err}", err.code().as_str());
                                }
                            }
                        }
                    }
                }
                if !clipped_ok {
                    let dl_client = build_http_client_with_timeout(app_data_dir, 120)?;
                    let resp = http_get_with_retry(&dl_client, signed_url, None, 3).map_err(|e| {
                        format!(
                            "MPC_STAC_DOWNLOAD_FAILED ({collection}/{item_id}): 网络请求失败 ({e}); clip={clip_err_detail}"
                        )
                    })?;
                    let written = stream_http_to_file_verified(
                        resp,
                        &out_file,
                        MAX_SCIENCE_DOWNLOAD_BYTES,
                        &format!("MPC_STAC_DOWNLOAD ({collection}/{item_id})"),
                    )?;
                    downloaded_bytes = written;
                    downloaded_file = Some(out_file.to_string_lossy().to_string());
                }
                }
            }

            let mut out_json = json!({
                "ok": true,
                "dataset": dataset,
                "collection": collection,
                "sub_product": asset_pref,
                "count": items.len(),
                "already_exists": already_exists,
                "target_item_id": target_item_id_out,
                "downloaded_file": downloaded_file,
                "downloaded_path": downloaded_file,
                "downloaded_bytes": downloaded_bytes,
                "items": items,
            });
            if let Some(ref dl_s) = downloaded_file {
                if dl_s.ends_with(".tif") || dl_s.ends_with(".tiff") {
                    attach_raster_preview_fields(&mut out_json, Path::new(dl_s));
                }
            }
            Ok(out_json)
        }
        "asf_cmr_science" | "opera_disp_s1" | "aria_s1_gunw" | "nisar_asf_cmr" | "nisar_products" => {
            // Parse tokenized keyword: `${startDate}..${endDate}|${subProduct}|tier:validated,provisional|download`
            let mut raw_sub_product = String::new();
            let mut date_range_opt: Option<(String, String)> = None;
            let mut kw_download = false;
            let mut kw_catalog = false;
            let mut tier_filter_raw: Option<String> = None;

            if let Some(kw) = req.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                let tokens: Vec<&str> = kw.split('|').map(str::trim).filter(|s| !s.is_empty()).collect();
                for token in tokens {
                    if token.contains("..") {
                        let mut dt_parts = token.split("..");
                        let s = dt_parts.next().unwrap_or("").trim();
                        let e = dt_parts.next().unwrap_or("").trim();
                        if !s.is_empty() && !e.is_empty() {
                            date_range_opt = Some((s.to_string(), e.to_string()));
                        }
                    } else if let Some(rest) = token
                        .strip_prefix("tier:")
                        .or_else(|| token.strip_prefix("tiers:"))
                        .or_else(|| token.strip_prefix("TIER:"))
                        .or_else(|| token.strip_prefix("TIERS:"))
                    {
                        tier_filter_raw = Some(rest.trim().to_ascii_lowercase());
                    } else if token.eq_ignore_ascii_case("download") {
                        kw_download = true;
                    } else if token.eq_ignore_ascii_case("catalog") {
                        kw_catalog = true;
                    } else if token.eq_ignore_ascii_case("search")
                        || token.eq_ignore_ascii_case("preview")
                        || token.eq_ignore_ascii_case("ascending")
                        || token.eq_ignore_ascii_case("descending")
                    {
                        // action or orbit filter flag, skip as sub-product token
                    } else if raw_sub_product.is_empty() {
                        raw_sub_product = token.to_string();
                    }
                }
            }
            if tier_filter_raw.is_none() {
                if let Some(args) = req.tool_args.as_ref() {
                    for arg in args {
                        let trimmed = arg.trim();
                        if let Some(rest) = trimmed
                            .strip_prefix("tier:")
                            .or_else(|| trimmed.strip_prefix("tiers:"))
                            .or_else(|| trimmed.strip_prefix("TIER:"))
                            .or_else(|| trimmed.strip_prefix("TIERS:"))
                        {
                            tier_filter_raw = Some(rest.trim().to_ascii_lowercase());
                            break;
                        }
                    }
                }
            }
            let (include_validated, include_provisional) = match tier_filter_raw.as_deref() {
                Some(spec) if !spec.is_empty() => {
                    let v = spec.contains("validated") || spec.contains("v1") || spec.contains("official");
                    let p = spec.contains("provisional");
                    if !v && !p {
                        (true, true)
                    } else {
                        (v, p)
                    }
                }
                _ => (true, true),
            };

            if kw_catalog && (dataset == "nisar_products" || dataset == "nisar_asf_cmr") {
                let catalog_entries: Vec<Value> = insar_provider::asf::nisar_catalog::nisar_product_catalog()
                    .iter()
                    .map(|entry| {
                        json!({
                            "band": entry.band.code(),
                            "product_type": entry.product_type.code(),
                            "science_level": entry.science_level,
                            "tier": entry.tier.label(),
                            "short_name": entry.short_name,
                            "collection_concept_id": entry.collection_concept_id,
                            "availability": entry.availability.label(),
                            "provider_endpoint": entry.provider_endpoint,
                            "supports_client_subset_v2": entry.supports_client_subset_v2,
                            "notes": entry.notes,
                        })
                    })
                    .collect();
                let sband_entries: Vec<Value> = insar_provider::bhoonidhi::BHOONIDHI_SBAND_PRODUCTS
                    .iter()
                    .map(|entry| {
                        json!({
                            "band": "S",
                            "product_type": entry.product_type,
                            "science_level": entry.science_level,
                            "collection_id": entry.collection_id,
                            "search_url": entry.search_url,
                            "requires_auth": true,
                            "notes": entry.notes,
                        })
                    })
                    .collect();
                return Ok(json!({
                    "ok": true,
                    "dataset": dataset,
                    "l_band_selectable_products": ["RSLC", "GSLC", "GCOV", "GUNW", "GOFF", "RIFG", "RUNW", "ROFF", "SME2"],
                    "catalog": catalog_entries,
                    "s_band_bhoonidhi": sband_entries,
                }));
            }

            let bbox = req
                .bbox
                .ok_or_else(|| "MISSING_BBOX: ASF CMR 检索需要 bbox=[west, south, east, north]".to_owned())?;

            let (start_d, end_d) = match date_range_opt {
                Some((s, e)) => (Some(s), Some(e)),
                None => (
                    req.start_date.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
                    req.end_date.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
                ),
            };

            if raw_sub_product.is_empty() {
                raw_sub_product = if dataset == "aria_s1_gunw" {
                    "ARIA_S1_GUNW".to_string()
                } else if dataset == "nisar_products" || dataset == "nisar_asf_cmr" {
                    "RSLC".to_string()
                } else {
                    "OPERA_L3_DISP-S1_V1".to_string()
                };
            }

            let upper_sub_product = raw_sub_product.to_ascii_uppercase();
            let nisar_resolved = if dataset == "aria_s1_gunw" {
                None
            } else if dataset == "nisar_products"
                || dataset == "nisar_asf_cmr"
                || upper_sub_product.starts_with("NISAR")
            {
                insar_provider::asf::nisar_catalog::resolve_nisar_l_band_cmr_short_names_for_tiers(
                    &upper_sub_product,
                    include_validated,
                    include_provisional,
                )
            } else {
                None
            };

            let mut requested_tiers: Vec<&'static str> = Vec::new();
            if include_validated {
                requested_tiers.push("validated");
            }
            if include_provisional {
                requested_tiers.push("provisional");
            }

            if let Some((pt, ref names)) = nisar_resolved {
                if names.is_empty() {
                    let (tier_desc, hint_desc) = if include_validated && !include_provisional {
                        ("Validated", "Provisional")
                    } else if include_provisional && !include_validated {
                        ("Provisional", "Validated")
                    } else {
                        ("所选发布版本", "其他发布版本")
                    };
                    return Ok(json!({
                        "ok": true,
                        "dataset": dataset,
                        "short_name": format!("NISAR_{}_{}", pt.science_level(), pt.code()),
                        "requested_tiers": requested_tiers,
                        "count": 0,
                        "empty_reason_zh": format!(
                            "当前产品（NISAR L 波段 {}）在 NASA ASF CMR 尚未发布 {} 版本，请勾选 {} 后重试。",
                            pt.code(),
                            tier_desc,
                            hint_desc
                        ),
                        "already_exists": false,
                        "target_item_id": Value::Null,
                        "downloaded_file": Value::Null,
                        "downloaded_path": Value::Null,
                        "downloaded_bytes": Value::Null,
                        "items": [],
                    }));
                }
            }

            let short_name_owned: String;
            let short_name: &str = if let Some((_pt, ref names)) = nisar_resolved {
                short_name_owned = names[0].to_owned();
                short_name_owned.as_str()
            } else {
                match upper_sub_product.as_str() {
                    "DISP-S1" | "OPERA_L3_DISP-S1_V1" | "DISP" => "OPERA_L3_DISP-S1_V1",
                    "CSLC-S1" | "OPERA_L2_CSLC-S1_V1" | "CSLC" => "OPERA_L2_CSLC-S1_V1",
                    "RTC-S1" | "OPERA_L2_RTC-S1_V1" | "RTC" => "OPERA_L2_RTC-S1_V1",
                    "ARIA_S1_GUNW" | "GUNW" | "ARIA" | "ARIA-S1-GUNW" | "S1-GUNW" => "ARIA_S1_GUNW",
                    other => {
                        if other.starts_with("OPERA_") || other.starts_with("ARIA_") {
                            other
                        } else if dataset == "aria_s1_gunw" {
                            "ARIA_S1_GUNW"
                        } else {
                            "OPERA_L3_DISP-S1_V1"
                        }
                    }
                }
            };

            let client = build_http_client(app_data_dir)?;
            let use_umm_json = short_name == "ARIA_S1_GUNW" || nisar_resolved.is_some();
            let cmr_url = if use_umm_json {
                let query_short_names: Vec<&str> = match nisar_resolved {
                    Some((_, ref names)) => names.clone(),
                    None => vec![short_name],
                };
                let temporal = match (start_d.as_deref(), end_d.as_deref()) {
                    (Some(s), Some(e)) => Some((s, e)),
                    _ => None,
                };
                insar_provider::asf::nisar_catalog::build_cmr_umm_search_url(
                    &query_short_names,
                    bbox,
                    temporal,
                    25,
                )
            } else {
                let bbox_str = format!("{},{},{},{}", bbox[0], bbox[1], bbox[2], bbox[3]);
                let mut u = format!(
                    "https://cmr.earthdata.nasa.gov/search/granules.json?short_name={short_name}&bounding_box={bbox_str}&page_size=25&sort_key=-start_date"
                );
                if let (Some(s), Some(e)) = (start_d.as_deref(), end_d.as_deref()) {
                    let s_temporal = if s.contains('T') { s.to_string() } else { format!("{s}T00:00:00Z") };
                    let e_temporal = if e.contains('T') { e.to_string() } else { format!("{e}T23:59:59Z") };
                    u.push_str(&format!("&temporal={s_temporal},{e_temporal}"));
                }
                u
            };
            let cmr_http = http_get_with_retry(&client, &cmr_url, None, 3)
                .map_err(|err| format!("CMR_SEARCH_FAILED ({short_name}): {err}"))?;
            if !cmr_http.status().is_success() {
                return Err(format!(
                    "CMR_SEARCH_HTTP_ERROR ({short_name}): HTTP {}",
                    cmr_http.status()
                ));
            }
            let cmr_resp: Value = cmr_http
                .json()
                .map_err(|err| format!("CMR_JSON_FAILED ({short_name}): {err}"))?;

            let check_out_dir = req
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            let mut items = if short_name == "ARIA_S1_GUNW" {
                let mut parsed = insar_provider::asf::nisar_catalog::parse_aria_s1_gunw_umm_items(&cmr_resp, bbox);
                for item in &mut parsed {
                    if let (Some(dir), Some(id)) = (check_out_dir.as_ref(), item["id"].as_str()) {
                        let cand = dir.join(format!("{id}.nc"));
                        if let Some(local_p) = existing_valid_science_file_bytes(&cand)
                            .map(|_| cand.to_string_lossy().to_string())
                        {
                            item["already_downloaded"] = json!(true);
                            item["local_file_path"] = json!(local_p);
                        } else {
                            item["already_downloaded"] = json!(false);
                            item["local_file_path"] = Value::Null;
                        }
                    }
                }
                parsed
            } else if nisar_resolved.is_some() {
                let mut parsed = insar_provider::asf::nisar_catalog::filter_nisar_items_by_release_tier(
                    insar_provider::asf::nisar_catalog::parse_nisar_l_band_umm_items(&cmr_resp, short_name, bbox),
                    include_validated,
                    include_provisional,
                );
                for item in &mut parsed {
                    if let (Some(dir), Some(id)) = (check_out_dir.as_ref(), item["id"].as_str()) {
                        let cand = dir.join(format!("{id}.h5"));
                        if let Some(local_p) = existing_valid_science_file_bytes(&cand)
                            .map(|_| cand.to_string_lossy().to_string())
                        {
                            item["already_downloaded"] = json!(true);
                            item["local_file_path"] = json!(local_p);
                        } else {
                            item["already_downloaded"] = json!(false);
                            item["local_file_path"] = Value::Null;
                        }
                    }
                }
                parsed
            } else {
                let entries = cmr_resp["feed"]["entry"].as_array().cloned().unwrap_or_default();
                let mut legacy_items = Vec::new();
                for (entry_idx, entry) in entries.iter().enumerate() {
                    let id = entry["title"].as_str().or_else(|| entry["id"].as_str()).unwrap_or("granule").to_owned();
                    let dt = entry["time_start"].as_str().unwrap_or("2024-01-01T00:00:00Z").to_owned();
                    let mut data_url = String::new();
                    let mut browse_url = String::new();
                    if let Some(links) = entry["links"].as_array() {
                        for lnk in links {
                            let href = lnk["href"].as_str().unwrap_or("");
                            let rel = lnk["rel"].as_str().unwrap_or("");
                            let clean_href = href.split('?').next().unwrap_or("").to_ascii_lowercase();
                            if href.starts_with("https://") {
                                if (clean_href.ends_with(".tif")
                                    || clean_href.ends_with(".tiff")
                                    || clean_href.ends_with(".h5")
                                    || clean_href.ends_with(".hdf5")
                                    || clean_href.ends_with(".nc"))
                                    && !rel.contains("browse")
                                    && data_url.is_empty()
                                {
                                    data_url = href.to_owned();
                                } else if (rel.contains("browse") || clean_href.ends_with(".png")) && browse_url.is_empty() {
                                    browse_url = href.to_owned();
                                }
                            }
                        }
                    }
                    let mut poly_coords = Vec::new();
                    let mut min_lon = f64::INFINITY;
                    let mut min_lat = f64::INFINITY;
                    let mut max_lon = f64::NEG_INFINITY;
                    let mut max_lat = f64::NEG_INFINITY;
                    if let Some(polys) = entry["polygons"].as_array() {
                        if let Some(first_poly) = polys.first().and_then(|p| p.as_array()).and_then(|a| a.first()).and_then(|s| s.as_str()) {
                            let nums: Vec<f64> = first_poly
                                .split_whitespace()
                                .filter_map(|t| t.parse::<f64>().ok())
                                .collect();
                            for pair in nums.chunks(2) {
                                if pair.len() == 2 {
                                    let lat = pair[0];
                                    let lon = pair[1];
                                    min_lon = min_lon.min(lon);
                                    max_lon = max_lon.max(lon);
                                    min_lat = min_lat.min(lat);
                                    max_lat = max_lat.max(lat);
                                    poly_coords.push(json!([lon, lat]));
                                }
                            }
                        }
                    }
                    if poly_coords.len() >= 3 && poly_coords.first() != poly_coords.last() {
                        if let Some(first) = poly_coords.first().cloned() {
                            poly_coords.push(first);
                        }
                    }
                    let item_bbox = if poly_coords.len() >= 4 && min_lon.is_finite() && min_lat.is_finite() {
                        [min_lon, min_lat, max_lon, max_lat]
                    } else {
                        bbox
                    };
                    let geometry = if poly_coords.len() >= 4 {
                        json!({ "type": "Polygon", "coordinates": [poly_coords] })
                    } else {
                        json!({
                            "type": "Polygon",
                            "coordinates": [[
                                [bbox[0], bbox[1]],
                                [bbox[2], bbox[1]],
                                [bbox[2], bbox[3]],
                                [bbox[0], bbox[3]],
                                [bbox[0], bbox[1]]
                            ]]
                        })
                    };
                    let resolved_thumb: Option<String> = if browse_url.starts_with("https://datapool.asf.alaska.edu/")
                        || browse_url.starts_with("https://cumulus.asf.earthdatacloud.nasa.gov/")
                        || browse_url.starts_with("https://nisar.asf.earthdatacloud.nasa.gov/")
                    {
                        Some(browse_url.clone())
                    } else if !browse_url.is_empty() && entry_idx < 8 {
                        fetch_preview_png_data_url(&client, &browse_url)
                    } else {
                        None
                    };
                    let clean_data_url_item = data_url.split('?').next().unwrap_or("").to_ascii_lowercase();
                    let item_ext = if clean_data_url_item.ends_with(".tif") || clean_data_url_item.ends_with(".tiff") {
                        "tif"
                    } else if clean_data_url_item.ends_with(".h5")
                        || clean_data_url_item.ends_with(".hdf")
                        || clean_data_url_item.ends_with(".hdf5")
                    {
                        "h5"
                    } else {
                        "nc"
                    };
                    let existing_local_path = check_out_dir.as_ref().and_then(|dir| {
                        let cand = dir.join(format!("{id}.{item_ext}"));
                        existing_valid_science_file_bytes(&cand).map(|_| cand.to_string_lossy().to_string())
                    });
                    let entry_orbit_dir = entry["orbit_direction"]
                        .as_str()
                        .or_else(|| entry["flight_direction"].as_str())
                        .and_then(|v| {
                            let u = v.trim().to_ascii_uppercase();
                            if u.starts_with("ASC") || u == "A" {
                                Some("ASCENDING")
                            } else if u.starts_with("DESC") || u == "D" {
                                Some("DESCENDING")
                            } else {
                                None
                            }
                        })
                        .or_else(|| infer_orbit_direction_from_granule_id(&id));
                    legacy_items.push(json!({
                        "id": id,
                        "short_name": short_name,
                        "datetime": dt,
                        "flight_direction": entry_orbit_dir,
                        "href": data_url,
                        "data_url": data_url,
                        "browse_url": browse_url,
                        "thumbnail_url": resolved_thumb,
                        "preview_url": resolved_thumb,
                        "bbox": item_bbox,
                        "already_downloaded": existing_local_path.is_some(),
                        "local_file_path": existing_local_path,
                        "geometry": geometry,
                    }));
                }
                legacy_items
            };
            filter_open_geodata_items_by_aoi(&mut items, req.aoi_geojson.as_ref());
            if let Some(req_orbit) = parse_orbit_filter_token(req.keyword.as_deref()) {
                items.retain(|it| {
                    it.get("flight_direction")
                        .and_then(Value::as_str)
                        .is_some_and(|dir| dir.eq_ignore_ascii_case(req_orbit))
                });
            }

            let mut downloaded_file: Option<String> = None;
            let mut downloaded_bytes: u64 = 0;
            let mut already_exists = false;
            let mut target_item_id_out: Option<String> = None;
            let should_download = !has_search_only_token(req.keyword.as_deref())
                && (kw_download || req.tool_name.as_deref() == Some("download"));
            if should_download {
                const AUTH_ERR_MSG: &str = "EARTHDATA_AUTH_REQUIRED: 下载 NASA OPERA/ARIA 原始科学影像需先在「设置 -> 数据服务账号」登录 NASA Earthdata 账号";

                if items.is_empty() {
                    return Err(format!(
                        "ASF_CMR_NO_GRANULES: 在指定时间与区域内未检索到 {short_name} 科学影像"
                    ));
                }

                let out_dir = req
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join(short_name));
                fs::create_dir_all(&out_dir).map_err(|e| format!("CREATE_DIR_FAILED: {e}"))?;

                let target_item = if let Some(target_id) = req.osm_tag.as_deref().filter(|s| !s.is_empty()) {
                    items.iter().find(|it| it["id"].as_str() == Some(target_id)).unwrap_or(&items[0])
                } else {
                    items
                        .iter()
                        .find(|it| !it["href"].as_str().unwrap_or("").is_empty())
                        .unwrap_or(&items[0])
                };
                let item_id = target_item["id"].as_str().unwrap_or("granule").to_owned();
                target_item_id_out = Some(item_id.clone());
                let data_url = target_item["href"].as_str().unwrap_or("").to_owned();
                if data_url.is_empty() {
                    return Err(format!(
                        "ASF_CMR_MISSING_DATA_URL: {item_id} 未提供原始科学影像链接 (.tif/.h5/.nc)"
                    ));
                }

                let clean_data_url = data_url.split('?').next().unwrap_or("").to_ascii_lowercase();
                let ext = if clean_data_url.ends_with(".tif") || clean_data_url.ends_with(".tiff") {
                    "tif"
                } else if clean_data_url.ends_with(".h5")
                    || clean_data_url.ends_with(".hdf")
                    || clean_data_url.ends_with(".hdf5")
                {
                    "h5"
                } else {
                    "nc"
                };
                let fpath = out_dir.join(format!("{item_id}.{ext}"));

                if let Some(existing_bytes) = existing_valid_science_file_bytes(&fpath) {
                    already_exists = true;
                    downloaded_bytes = existing_bytes;
                    downloaded_file = Some(fpath.to_string_lossy().to_string());
                } else {
                    let auth = resolve_earthdata_auth_materials(AUTH_ERR_MSG)?;
                    let edl_client = build_edl_manual_redirect_client(app_data_dir)?;
                    let resp = execute_edl_manual_get(&edl_client, &data_url, &auth, AUTH_ERR_MSG)?;
                    let tmp_path = fpath.with_extension(format!("{ext}.download.tmp"));
                    let _ = fs::remove_file(&tmp_path);
                    let written = stream_http_to_file_verified(
                        resp,
                        &tmp_path,
                        MAX_SCIENCE_DOWNLOAD_BYTES,
                        &format!("EARTHDATA_SCIENCE_DOWNLOAD ({short_name}/{item_id})"),
                    )?;

                    // Strictly inspect downloaded magic bytes: reject HTML OAuth login pages & non-science payloads!
                    let head = {
                        use std::io::Read;
                        let mut f = fs::File::open(&tmp_path)
                            .map_err(|e| format!("READ_DOWNLOADED_GRANULE_FAILED: {e}"))?;
                        let mut b = [0u8; 64];
                        let n = f.read(&mut b).unwrap_or(0);
                        b[..n].to_vec()
                    };
                    let head_ascii = String::from_utf8_lossy(&head).trim_start().to_ascii_lowercase();
                    if head_ascii.starts_with("<!doctype")
                        || head_ascii.starts_with("<html")
                        || head_ascii.contains("earthdata login")
                    {
                        let _ = fs::remove_file(&tmp_path);
                        return Err(AUTH_ERR_MSG.to_owned());
                    }
                    let valid_magic = head.starts_with(b"II*\0")
                        || head.starts_with(b"MM\0*")
                        || head.starts_with(b"II+\0")
                        || head.starts_with(b"MM\0+")
                        || head.starts_with(b"\x89HDF\r\n\x1a\n")
                        || head.starts_with(b"CDF\x01")
                        || head.starts_with(b"CDF\x02")
                        || head.starts_with(b"CDF\x05");
                    if !valid_magic {
                        let _ = fs::remove_file(&tmp_path);
                        return Err(format!(
                            "EARTHDATA_INVALID_SCIENCE_MAGIC: 下载的 {item_id} 非合法 GeoTIFF/HDF5/NetCDF 二进制文件"
                        ));
                    }
                    fs::rename(&tmp_path, &fpath).map_err(|e| {
                        let _ = fs::remove_file(&tmp_path);
                        format!("RENAME_FAILED: {e}")
                    })?;

                    downloaded_bytes = written;
                    downloaded_file = Some(fpath.to_string_lossy().to_string());
                }
            }

            let empty_reason_zh = if items.is_empty() {
                if short_name == "ARIA_S1_GUNW" {
                    Some(insar_provider::asf::nisar_catalog::ARIA_S1_GUNW_EMPTY_REASON_ZH.to_owned())
                } else if nisar_resolved.is_some() && include_validated && !include_provisional {
                    Some("当前研究区与时间窗口内暂无 NASA ASF Validated 正式版 NISAR 产品，可勾选 Provisional 后重试。".to_owned())
                } else {
                    Some(format!("当前研究区与时间窗口内暂无 NASA CMR {short_name} 公开科学产品。"))
                }
            } else {
                None
            };

            let mut out_json = json!({
                "ok": true,
                "dataset": dataset,
                "short_name": short_name,
                "requested_tiers": requested_tiers,
                "count": items.len(),
                "empty_reason_zh": empty_reason_zh,
                "already_exists": already_exists,
                "target_item_id": target_item_id_out,
                "downloaded_file": downloaded_file,
                "downloaded_path": downloaded_file,
                "downloaded_bytes": downloaded_bytes,
                "items": items,
            });
            if let Some(ref dl_s) = downloaded_file {
                if dl_s.ends_with(".tif") || dl_s.ends_with(".tiff") {
                    attach_raster_preview_fields(&mut out_json, Path::new(dl_s));
                }
            }
            Ok(out_json)
        }
        "nasa_black_marble"
        | "black_marble"
        | "viirs_black_marble"
        | "nasa_emit"
        | "emit"
        | "emit_l2a"
        | "nasa_ecostress"
        | "ecostress"
        | "ecostress_l2t"
        | "dlr_enmap"
        | "enmap"
        | "enmap_l2a" => {
            let eo_dataset = insar_provider::open_eo_catalog::OpenEoDataset::from_dataset_key(&dataset)
                .ok_or_else(|| format!("UNSUPPORTED_OPEN_EO_DATASET: {dataset}"))?;
            let parsed_kw =
                insar_provider::open_eo_catalog::parse_open_eo_keyword(req.keyword.as_deref());
            let spec = insar_provider::open_eo_catalog::resolve_open_eo_subproduct(
                eo_dataset,
                parsed_kw
                    .sub_product_token
                    .as_deref()
                    .or(req.dem_product.as_deref()),
            );

            let should_download = !parsed_kw.search_only_token
                && !has_search_only_token(req.keyword.as_deref())
                && (parsed_kw.download_requested || req.tool_name.as_deref() == Some("download"));
            if should_download && (!eo_dataset.download_supported() || !spec.download_supported) {
                return Err(insar_provider::open_eo_catalog::DLR_ENMAP_SSO_DOWNLOAD_ERROR.to_owned());
            }

            let bbox = req.bbox.ok_or_else(|| {
                format!(
                    "MISSING_BBOX: {} 公开目录检索需要 bbox=[west, south, east, north]",
                    spec.code
                )
            })?;
            let (start_d, end_d) = match parsed_kw.date_range {
                Some((s, e)) => (Some(s), Some(e)),
                None => (
                    req.start_date
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    req.end_date
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                ),
            };
            let temporal_pair = match (start_d.as_deref(), end_d.as_deref()) {
                (Some(s), Some(e)) => Some((s, e)),
                _ => None,
            };

            let client = build_http_client(app_data_dir)?;
            let mut items = insar_provider::open_eo_catalog::fetch_open_eo_catalog_items(
                &client,
                spec,
                bbox,
                temporal_pair,
                25,
            )?;

            let check_out_dir = req
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            for item in &mut items {
                if !spec.download_supported {
                    item["already_downloaded"] = json!(false);
                    item["local_file_path"] = Value::Null;
                    continue;
                }
                if let (Some(dir), Some(id)) = (check_out_dir.as_ref(), item["id"].as_str()) {
                    let ext = item["file_ext"].as_str().unwrap_or(spec.default_ext);
                    let cand = dir.join(format!("{id}.{ext}"));
                    if let Some(local_p) = existing_valid_science_file_bytes(&cand)
                        .map(|_| cand.to_string_lossy().to_string())
                    {
                        item["already_downloaded"] = json!(true);
                        item["local_file_path"] = json!(local_p);
                    } else {
                        item["already_downloaded"] = json!(false);
                        item["local_file_path"] = Value::Null;
                    }
                }
            }
            filter_open_geodata_items_by_aoi(&mut items, req.aoi_geojson.as_ref());

            let mut downloaded_file: Option<String> = None;
            let mut downloaded_bytes: u64 = 0;
            let mut already_exists = false;
            let mut target_item_id_out: Option<String> = None;

            if should_download {
                if items.is_empty() {
                    return Err(format!("OPEN_EO_NO_GRANULES: {}", spec.empty_reason_zh));
                }
                let out_dir = req
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join(spec.code));
                fs::create_dir_all(&out_dir).map_err(|e| format!("CREATE_DIR_FAILED: {e}"))?;

                let target_item =
                    select_open_eo_target_item(&items, req.osm_tag.as_deref())?;
                let item_id = target_item["id"].as_str().unwrap_or("granule").to_owned();
                target_item_id_out = Some(item_id.clone());
                let data_url = target_item["href"].as_str().unwrap_or("").to_owned();
                if data_url.is_empty() {
                    return Err(format!(
                        "OPEN_EO_MISSING_DATA_URL: {item_id} 未提供可直链下载的原始科学影像链接"
                    ));
                }

                let ext = target_item["file_ext"].as_str().unwrap_or(spec.default_ext);
                let fpath = out_dir.join(format!("{item_id}.{ext}"));

                if let Some(existing_bytes) = existing_valid_science_file_bytes(&fpath) {
                    already_exists = true;
                    downloaded_bytes = existing_bytes;
                    downloaded_file = Some(fpath.to_string_lossy().to_string());
                } else {
                    let auth_err = eo_dataset.earthdata_auth_error();
                    let auth = resolve_earthdata_auth_materials(auth_err)?;
                    let edl_client = build_edl_manual_redirect_client(app_data_dir)?;
                    let resp = execute_edl_manual_get(&edl_client, &data_url, &auth, auth_err)?;
                    let tmp_path = fpath.with_extension(format!("{ext}.download.tmp"));
                    let _ = fs::remove_file(&tmp_path);
                    let written = stream_http_to_file_verified(
                        resp,
                        &tmp_path,
                        MAX_SCIENCE_DOWNLOAD_BYTES,
                        &format!("EARTHDATA_OPEN_EO_DOWNLOAD ({}/{item_id})", spec.code),
                    )?;

                    let head = {
                        use std::io::Read;
                        let mut f = fs::File::open(&tmp_path)
                            .map_err(|e| format!("READ_DOWNLOADED_GRANULE_FAILED: {e}"))?;
                        let mut b = [0u8; 64];
                        let n = f.read(&mut b).unwrap_or(0);
                        b[..n].to_vec()
                    };
                    if let Err(magic_err) = insar_provider::open_eo_catalog::validate_science_binary_magic(
                        &head, &item_id, auth_err,
                    ) {
                        let _ = fs::remove_file(&tmp_path);
                        return Err(magic_err);
                    }
                    fs::rename(&tmp_path, &fpath).map_err(|e| {
                        let _ = fs::remove_file(&tmp_path);
                        format!("RENAME_FAILED: {e}")
                    })?;

                    downloaded_bytes = written;
                    downloaded_file = Some(fpath.to_string_lossy().to_string());
                }
            }

            let empty_reason_zh = if items.is_empty() {
                Some(spec.empty_reason_zh.to_owned())
            } else {
                None
            };

            let mut out_json = json!({
                "ok": true,
                "dataset": eo_dataset.dataset_key(),
                "resource_id": eo_dataset.resource_id(),
                "short_name": spec.short_name,
                "sub_product": spec.code,
                "search_only": !spec.download_supported,
                "count": items.len(),
                "empty_reason_zh": empty_reason_zh,
                "already_exists": already_exists,
                "target_item_id": target_item_id_out,
                "downloaded_file": downloaded_file,
                "downloaded_path": downloaded_file,
                "downloaded_bytes": downloaded_bytes,
                "items": items,
            });
            if let Some(ref dl_s) = downloaded_file {
                if dl_s.ends_with(".tif") || dl_s.ends_with(".tiff") {
                    attach_raster_preview_fields(&mut out_json, Path::new(dl_s));
                }
            }
            Ok(out_json)
        }
        "usgs_earthquake" => {
            let bbox = req
                .bbox
                .ok_or_else(|| "MISSING_BBOX: usgs_earthquake 需要 bbox=[west, south, east, north]".to_owned())?;
            let mut kw_start_date: Option<String> = None;
            let mut kw_end_date: Option<String> = None;
            let mut kw_min_mag: Option<f64> = None;
            if let Some(raw_kw) = req.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                for token in raw_kw.split('|').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some((s, e)) = token.split_once("..") {
                        let s_trim = s.trim();
                        let e_trim = e.trim();
                        if !s_trim.is_empty() {
                            kw_start_date = Some(s_trim.to_owned());
                        }
                        if !e_trim.is_empty() {
                            kw_end_date = Some(e_trim.to_owned());
                        }
                    } else if let Ok(mag) = token.trim_start_matches('M').trim_start_matches('m').parse::<f64>() {
                        kw_min_mag = Some(mag);
                    }
                }
            }
            let min_mag = kw_min_mag.unwrap_or(4.5);
            let start = req
                .start_date
                .as_deref()
                .filter(|s| !s.is_empty())
                .or(kw_start_date.as_deref())
                .unwrap_or("2020-01-01");
            let end = req
                .end_date
                .as_deref()
                .filter(|s| !s.is_empty())
                .or(kw_end_date.as_deref())
                .unwrap_or("2026-12-31");
            let url = format!(
                "https://earthquake.usgs.gov/fdsnws/event/1/query?format=geojson&starttime={start}&endtime={end}&minmagnitude={min_mag}&minlongitude={}&minlatitude={}&maxlongitude={}&maxlatitude={}&limit=50",
                bbox[0].min(bbox[2]),
                bbox[1].min(bbox[3]),
                bbox[0].max(bbox[2]),
                bbox[1].max(bbox[3]),
            );
            let client = build_http_client(app_data_dir)?;
            let mut last_err = String::new();
            let mut body_opt: Option<Value> = None;
            for attempt in 0..3 {
                if attempt > 0 {
                    std::thread::sleep(Duration::from_millis(450 * attempt as u64));
                }
                match client.get(&url).send() {
                    Ok(resp) if resp.status().is_success() => match resp.json::<Value>() {
                        Ok(parsed) => {
                            body_opt = Some(parsed);
                            break;
                        }
                        Err(err) => {
                            last_err = format!("USGS_EQ_JSON_FAILED: {err}");
                        }
                    },
                    Ok(resp) => {
                        last_err = format!("USGS_EQ_HTTP_{}", resp.status().as_u16());
                    }
                    Err(err) => {
                        last_err = format!("USGS_EQ_QUERY_FAILED: {err}");
                    }
                }
            }
            let body: Value = body_opt.ok_or(last_err)?;

            let features = body["features"].as_array().cloned().unwrap_or_default();
            let mut items = Vec::new();
            for feat in &features {
                let id = feat["id"].as_str().unwrap_or("eq").to_owned();
                let mag = feat["properties"]["mag"].as_f64().unwrap_or(0.0);
                let place = feat["properties"]["place"].as_str().unwrap_or("Unknown").to_owned();
                let time_ms = feat["properties"]["time"].as_i64().unwrap_or(0);
                let dt = if time_ms > 0 {
                    let secs = time_ms / 1000;
                    let days = secs / 86400;
                    // Approximate YYYY-MM-DD from Unix days
                    let z = days + 719468;
                    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
                    let doe = (z - era * 146097) as u64;
                    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
                    let y = yoe as i64 + era * 400;
                    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                    let mp = (5 * doy + 2) / 153;
                    let d = doy - (153 * mp + 2) / 5 + 1;
                    let m = if mp < 10 { mp + 3 } else { mp - 9 };
                    let y_final = if m <= 2 { y + 1 } else { y };
                    format!("{y_final:04}-{m:02}-{d:02}T00:00:00Z")
                } else {
                    "2024-01-01T00:00:00Z".to_owned()
                };
                let coords = feat["geometry"]["coordinates"].as_array().cloned().unwrap_or_default();
                let lon = coords.first().and_then(|v| v.as_f64()).unwrap_or((bbox[0] + bbox[2]) * 0.5);
                let lat = coords.get(1).and_then(|v| v.as_f64()).unwrap_or((bbox[1] + bbox[3]) * 0.5);
                let depth_km = coords.get(2).and_then(|v| v.as_f64()).unwrap_or(10.0);
                let buf = 0.25;
                items.push(json!({
                    "id": format!("USGS_{id}_M{mag:.1}"),
                    "title": format!("M{mag:.1} · {place} (深度 {depth_km:.1}km)"),
                    "datetime": dt,
                    "magnitude": mag,
                    "depth_km": depth_km,
                    "place": place,
                    "lon": lon,
                    "lat": lat,
                    "href": feat["properties"]["url"].as_str().unwrap_or(""),
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[
                            [lon - buf, lat - buf],
                            [lon + buf, lat - buf],
                            [lon + buf, lat + buf],
                            [lon - buf, lat + buf],
                            [lon - buf, lat - buf]
                        ]]
                    }
                }));
            }

            let eq_search_only = has_search_only_token(req.keyword.as_deref())
                && req.tool_name.as_deref() != Some("download");
            let out_dir = req
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join("usgs_earthquakes"));
            let out_file = out_dir.join("usgs_coseismic_events.geojson");
            if eq_search_only {
                return Ok(json!({
                    "ok": true,
                    "dataset": "usgs_earthquake",
                    "min_magnitude": min_mag,
                    "start_date": start,
                    "end_date": end,
                    "count": items.len(),
                    "downloaded_file": Value::Null,
                    "downloaded_path": Value::Null,
                    "downloaded_bytes": 0,
                    "items": items,
                }));
            }
            let mut already_exists = false;
            let downloaded_bytes = if let Some(existing_len) = existing_valid_science_file_bytes(&out_file) {
                already_exists = true;
                existing_len as usize
            } else {
                let _ = fs::create_dir_all(&out_dir);
                let pretty = serde_json::to_string_pretty(&body).unwrap_or_default();
                let _ = fs::write(&out_file, pretty.as_bytes());
                pretty.len()
            };
            let out_file_str = out_file.to_string_lossy().to_string();

            Ok(json!({
                "ok": true,
                "dataset": "usgs_earthquake",
                "min_magnitude": min_mag,
                "start_date": start,
                "end_date": end,
                "count": items.len(),
                "already_exists": already_exists,
                "downloaded_file": out_file_str,
                "downloaded_path": out_file_str,
                "downloaded_bytes": downloaded_bytes,
                "items": items,
            }))
        }
        "ngl_gnss" | "comet_licsar" => {
            let bbox = req
                .bbox
                .ok_or_else(|| format!("MISSING_BBOX: {dataset} 需要 bbox=[west, south, east, north]"))?;
            let has_explicit_out_dir = req
                .output_path
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty());
            let out_dir = req
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join(&dataset));

            let mut kw_start_date: Option<String> = None;
            let mut kw_end_date: Option<String> = None;
            let mut kw_download = false;
            let mut kw_search_only = false;
            let mut kw_sub_product: Option<String> = None;
            if let Some(raw_kw) = req.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                for token in raw_kw.split('|').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some((s, e)) = token.split_once("..") {
                        if !s.trim().is_empty() {
                            kw_start_date = Some(s.trim().to_owned());
                        }
                        if !e.trim().is_empty() {
                            kw_end_date = Some(e.trim().to_owned());
                        }
                    } else if token.eq_ignore_ascii_case("download") {
                        kw_download = true;
                    } else if token.eq_ignore_ascii_case("search")
                        || token.eq_ignore_ascii_case("preview")
                    {
                        kw_search_only = true;
                    } else if token.eq_ignore_ascii_case("ascending")
                        || token.eq_ignore_ascii_case("descending")
                    {
                        // orbit direction token handled by parse_orbit_filter_token
                    } else if kw_sub_product.is_none() {
                        kw_sub_product = Some(token.to_owned());
                    }
                }
            }
            let should_download = kw_download
                || req.tool_name.as_deref() == Some("download")
                || (!kw_search_only && has_explicit_out_dir);
            if should_download {
                fs::create_dir_all(&out_dir).map_err(|e| format!("CREATE_DIR_FAILED: {e}"))?;
            }

            let client = build_http_client(app_data_dir)?;
            if dataset == "ngl_gnss" {
                let list_url = "https://geodesy.unr.edu/NGLStationPages/DataHoldings.txt";
                let text = http_get_with_retry(&client, list_url, None, 3)
                    .map_err(|e| format!("NGL_GNSS_INDEX_FETCH_FAILED: {e}"))?
                    .error_for_status()
                    .map_err(|e| format!("NGL_GNSS_INDEX_HTTP_ERROR: {e}"))?
                    .text()
                    .map_err(|e| format!("NGL_GNSS_INDEX_READ_FAILED: {e}"))?;
                let mut items = Vec::new();
                let w = bbox[0].min(bbox[2]) - 1.5;
                let e = bbox[0].max(bbox[2]) + 1.5;
                let s = bbox[1].min(bbox[3]) - 1.5;
                let n = bbox[1].max(bbox[3]) + 1.5;
                let center_lon = (bbox[0] + bbox[2]) * 0.5;
                let center_lat = (bbox[1] + bbox[3]) * 0.5;
                let mut nearest_candidates: Vec<(f64, Value)> = Vec::new();
                for line in text.lines().skip(1) {
                    let cols: Vec<&str> = line.split_whitespace().collect();
                    if cols.len() < 10 {
                        continue;
                    }
                    let sta = cols[0];
                    let Ok(lat) = cols[1].parse::<f64>() else { continue };
                    let Ok(mut lon) = cols[2].parse::<f64>() else { continue };
                    if lon > 180.0 {
                        lon -= 360.0;
                    }
                    let dt_beg = cols[7];
                    let dt_end = cols[8];
                    let tenv3_url = format!("https://geodesy.unr.edu/gps_timeseries/IGS20/tenv3/IGS20/{sta}.tenv3");
                    let buf = 0.08;
                    let existing_local = has_explicit_out_dir
                        .then(|| out_dir.join(format!("{sta}.tenv3")))
                        .and_then(|p| existing_valid_science_file_bytes(&p).map(|sz| (p, sz)));
                    let station_json = json!({
                        "id": format!("NGL_{sta}_IGS20"),
                        "station": sta,
                        "datetime": format!("{dt_end}T00:00:00Z"),
                        "time_span": format!("{dt_beg} ~ {dt_end}"),
                        "lat": lat,
                        "lon": lon,
                        "href": tenv3_url,
                        "already_downloaded": existing_local.is_some(),
                        "local_file_path": existing_local.as_ref().map(|(p, _)| p.to_string_lossy().to_string()),
                        "size_bytes": existing_local.as_ref().map(|(_, sz)| *sz),
                        "bbox": [lon - buf, lat - buf, lon + buf, lat + buf],
                        "geometry": {
                            "type": "Polygon",
                            "coordinates": [[
                                [lon - buf, lat - buf],
                                [lon + buf, lat - buf],
                                [lon + buf, lat + buf],
                                [lon - buf, lat + buf],
                                [lon - buf, lat - buf]
                            ]]
                        }
                    });
                    if lon >= w && lon <= e && lat >= s && lat <= n {
                        items.push(station_json);
                        if items.len() >= 25 {
                            break;
                        }
                    } else if items.is_empty() {
                        let dist2 = (lon - center_lon).powi(2) + (lat - center_lat).powi(2);
                        if dist2 <= 100.0 {
                            nearest_candidates.push((dist2, station_json));
                        }
                    }
                }
                if items.is_empty() && !nearest_candidates.is_empty() {
                    nearest_candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    for (_, st_val) in nearest_candidates.into_iter().take(5) {
                        items.push(st_val);
                    }
                }

                let mut downloaded_file: Option<String> = None;
                let mut downloaded_bytes: u64 = 0;
                let mut already_exists = false;
                let mut target_item_id_out: Option<String> = None;
                if should_download {
                    let chosen = if let Some(target_id) = req.osm_tag.as_deref().filter(|s| !s.is_empty()) {
                        items
                            .iter()
                            .find(|it| it["id"].as_str() == Some(target_id) || it["station"].as_str() == Some(target_id))
                            .or_else(|| items.first())
                    } else {
                        items.first()
                    }
                    .ok_or_else(|| {
                        "NGL_GNSS_NO_STATIONS: 目标 AOI 附近未找到可用 NGL GNSS 连续基准站".to_owned()
                    })?;
                    target_item_id_out = chosen["id"].as_str().map(str::to_owned);
                    let sta = chosen["station"].as_str().unwrap_or("STA");
                    let fpath = out_dir.join(format!("{sta}.tenv3"));
                    if let Some(existing_len) = existing_valid_science_file_bytes(&fpath) {
                        already_exists = true;
                        downloaded_bytes = existing_len;
                        downloaded_file = Some(fpath.to_string_lossy().to_string());
                    } else {
                        let candidate_urls = [
                            format!("https://geodesy.unr.edu/gps_timeseries/IGS20/tenv3/IGS20/{sta}.tenv3"),
                            format!("http://geodesy.unr.edu/gps_timeseries/IGS20/tenv3/IGS20/{sta}.tenv3"),
                            format!("https://geodesy.unr.edu/gps_timeseries/tenv3/IGS14/{sta}.tenv3"),
                            format!("http://geodesy.unr.edu/gps_timeseries/tenv3/IGS14/{sta}.tenv3"),
                        ];
                        let mut last_err = String::from("所有 .tenv3 镜像均不可达");
                        for url in &candidate_urls {
                            match http_get_with_retry(&client, url, None, 2) {
                                Ok(resp) if resp.status().is_success() => {
                                    match stream_http_to_file_verified(
                                        resp,
                                        &fpath,
                                        MAX_SCIENCE_DOWNLOAD_BYTES,
                                        "NGL_TENV3_DOWNLOAD",
                                    ) {
                                        Ok(written) => {
                                            let head = fs::read(&fpath).unwrap_or_default();
                                            let sample = String::from_utf8_lossy(&head[..head.len().min(256)])
                                                .to_ascii_lowercase();
                                            if sample.contains("<!doctype") || sample.contains("<html") {
                                                let _ = fs::remove_file(&fpath);
                                                last_err = format!("{url} 返回 HTML 错误页面");
                                                continue;
                                            }
                                            downloaded_bytes = written;
                                            downloaded_file = Some(fpath.to_string_lossy().to_string());
                                            break;
                                        }
                                        Err(e) => last_err = e,
                                    }
                                }
                                Ok(resp) => last_err = format!("{url} HTTP {}", resp.status()),
                                Err(e) => last_err = format!("{url} {e}"),
                            }
                        }
                        if downloaded_file.is_none() {
                            return Err(format!(
                                "NGL_TENV3_DOWNLOAD_FAILED: 无法下载站点 {sta} 真实 .tenv3 时间序列文件 ({last_err})"
                            ));
                        }
                    }
                }
                Ok(json!({
                    "ok": true,
                    "dataset": "ngl_gnss",
                    "count": items.len(),
                    "already_exists": already_exists,
                    "target_item_id": target_item_id_out,
                    "downloaded_file": downloaded_file,
                    "downloaded_path": downloaded_file,
                    "downloaded_bytes": downloaded_bytes,
                    "items": items,
                }))
            } else {
                // Real COMET LiCSAR discovery, browse quicklook preview, & GeoTIFF download on JASMIN
                let min_lon = bbox[0].min(bbox[2]);
                let min_lat = bbox[1].min(bbox[3]);
                let max_lon = bbox[0].max(bbox[2]);
                let max_lat = bbox[1].max(bbox[3]);
                let cy = (min_lat + max_lat) * 0.5;
                let start_dt_opt = req
                    .start_date
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .or(kw_start_date.as_deref());
                let end_dt = req
                    .end_date
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .or(kw_end_date.as_deref())
                    .or(kw_start_date.as_deref())
                    .unwrap_or("2026-12-31");
                let start_ymd = start_dt_opt.map(|s| s.replace('-', ""));
                let end_ymd = end_dt.replace('-', "");
                let requested_item_tag = req
                    .osm_tag
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());

                let fetch_text_retry = |url: &str| -> Result<String, String> {
                    let mut last_e = String::new();
                    for _ in 0..3 {
                        match client.get(url).send() {
                            Ok(r) if r.status().is_success() => {
                                if let Ok(t) = r.text() {
                                    return Ok(t);
                                }
                            }
                            Ok(r) => last_e = format!("HTTP {}", r.status()),
                            Err(e) => last_e = e.to_string(),
                        }
                    }
                    Err(format!("{url} ({last_e})"))
                };

                // 1. Discover real Sentinel-1 relative orbit tracks intersecting the AOI via NASA ASF CMR
                let mut candidate_tracks: Vec<i32> = Vec::new();
                if let Some(code_track) = req
                    .code
                    .as_deref()
                    .and_then(|c| c.trim().parse::<i32>().ok())
                    .filter(|t| (1..=175).contains(t))
                {
                    candidate_tracks.push(code_track);
                }
                if let Some(tag) = requested_item_tag {
                    let stripped = tag.strip_prefix("LiCSAR_").unwrap_or(tag);
                    if stripped.len() >= 3 {
                        if let Ok(tag_track) = stripped[..3].parse::<i32>() {
                            if (1..=175).contains(&tag_track) && !candidate_tracks.contains(&tag_track) {
                                candidate_tracks.push(tag_track);
                            }
                        }
                    }
                }
                let cmr_url = format!(
                    "https://cmr.earthdata.nasa.gov/search/granules.umm_json?short_name=SENTINEL-1A_SLC&bounding_box={min_lon},{min_lat},{max_lon},{max_lat}&page_size=40"
                );
                if let Ok(cmr_resp) = http_get_with_retry(&client, &cmr_url, None, 3) {
                    if cmr_resp.status().is_success() {
                        if let Ok(cmr_json) = cmr_resp.json::<Value>() {
                            if let Some(arr) = cmr_json["items"].as_array() {
                                for it in arr {
                                    if let Some(attrs) = it["umm"]["AdditionalAttributes"].as_array() {
                                        for attr in attrs {
                                            if attr["Name"].as_str() == Some("PATH_NUMBER") {
                                                if let Some(v_str) = attr["Values"]
                                                    .as_array()
                                                    .and_then(|vs| vs.first())
                                                    .and_then(|v| v.as_str())
                                                {
                                                    if let Ok(t) = v_str.trim().parse::<i32>() {
                                                        if (1..=175).contains(&t) && !candidate_tracks.contains(&t) {
                                                            candidate_tracks.push(t);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if candidate_tracks.is_empty() {
                    return Err(
                        "LICSAR_TRACK_DISCOVERY_FAILED: 无法从 ASF CMR 检索覆盖当前 AOI 的 Sentinel-1 真实相对轨道号 (PATH_NUMBER)"
                            .to_owned(),
                    );
                }

                // 2. Query JASMIN LiCSAR track directories & verify real frame footprints + real browse quicklooks
                struct VerifiedLicsarScene {
                    item_id: String,
                    track: i32,
                    frame_id: String,
                    pair: Option<String>,
                    datetime_iso: String,
                    frame_bbox: [f64; 4],
                    polygon_ring: Vec<[f64; 2]>,
                    tif_url: String,
                    browse_png_url: Option<String>,
                    preview_data_url: Option<String>,
                    product_type: String,
                }
                let mut verified_scenes: Vec<VerifiedLicsarScene> = Vec::new();
                let mut primary_endpoint = String::new();
                let max_scenes = if should_download && requested_item_tag.is_some() {
                    3
                } else if should_download {
                    4
                } else {
                    6
                };

                for &track in candidate_tracks.iter().take(6) {
                    let public_track_url = format!(
                        "https://gws-access.jasmin.ac.uk/public/nceo_geohazards/LiCSAR_products/{track}/"
                    );
                    if primary_endpoint.is_empty() {
                        primary_endpoint = public_track_url.clone();
                    }
                    let Ok(track_html) = fetch_text_retry(&public_track_url) else {
                        continue;
                    };
                    let prefix = format!("{track:03}");
                    let mut frame_candidates: Vec<(f64, String)> = Vec::new();
                    for part in track_html.split("href=\"") {
                        if let Some(end_q) = part.find('"') {
                            let link = part[..end_q].trim_end_matches('/');
                            if link.starts_with(&prefix)
                                && link.len() >= 15
                                && (link.contains('A') || link.contains('D'))
                            {
                                let exact_requested_frame = requested_item_tag
                                    .is_some_and(|tag| tag.contains(link));
                                let approx_lat_diff = if exact_requested_frame {
                                    -1.0
                                } else {
                                    link.split('_')
                                        .nth(1)
                                        .and_then(|s| s.parse::<f64>().ok())
                                        .map(|colat100| ((90.0 - colat100 / 100.0) - cy).abs())
                                        .unwrap_or(90.0)
                                };
                                if !frame_candidates.iter().any(|(_, f)| f == link) {
                                    frame_candidates.push((approx_lat_diff, link.to_owned()));
                                }
                            }
                        }
                    }
                    frame_candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                    for (_, frame_id) in frame_candidates.into_iter().take(3) {
                        let poly_url = format!(
                            "https://gws-access.jasmin.ac.uk/public/nceo_geohazards/LiCSAR_products/{track}/{frame_id}/metadata/{frame_id}-poly.txt"
                        );
                        let Ok(poly_txt) = fetch_text_retry(&poly_url) else {
                            continue;
                        };
                        let mut ring: Vec<[f64; 2]> = Vec::new();
                        let mut f_min_lon = f64::INFINITY;
                        let mut f_max_lon = f64::NEG_INFINITY;
                        let mut f_min_lat = f64::INFINITY;
                        let mut f_max_lat = f64::NEG_INFINITY;
                        for line in poly_txt.lines() {
                            let mut nums = line.split_whitespace();
                            if let (Some(lon_s), Some(lat_s)) = (nums.next(), nums.next()) {
                                if let (Ok(lon), Ok(lat)) = (lon_s.parse::<f64>(), lat_s.parse::<f64>()) {
                                    f_min_lon = f_min_lon.min(lon);
                                    f_max_lon = f_max_lon.max(lon);
                                    f_min_lat = f_min_lat.min(lat);
                                    f_max_lat = f_max_lat.max(lat);
                                    ring.push([lon, lat]);
                                }
                            }
                        }
                        if ring.len() < 3 {
                            continue;
                        }
                        if ring.first() != ring.last() {
                            if let Some(&first_pt) = ring.first() {
                                ring.push(first_pt);
                            }
                        }
                        let intersects = f_max_lon >= min_lon
                            && f_min_lon <= max_lon
                            && f_max_lat >= min_lat
                            && f_min_lat <= max_lat;
                        if !intersects {
                            continue;
                        }
                        let frame_bbox = [f_min_lon, f_min_lat, f_max_lon, f_max_lat];
                        let meta_mli_png = format!(
                            "https://gws-access.jasmin.ac.uk/public/nceo_geohazards/LiCSAR_products/{track}/{frame_id}/metadata/{frame_id}.geo.mli.png"
                        );

                        let want_meta_dem = kw_sub_product
                            .as_deref()
                            .map(|s| s.to_ascii_uppercase().contains("DEM") || s.to_ascii_uppercase().contains("HGT"))
                            .unwrap_or(false);
                        let mut added_for_frame = 0usize;
                        if !want_meta_dem {
                            let ifg_index_url = format!(
                                "https://gws-access.jasmin.ac.uk/public/nceo_geohazards/LiCSAR_products.public/{track}/{frame_id}/interferograms/"
                            );
                            if let Ok(ifg_html) = fetch_text_retry(&ifg_index_url) {
                                let mut pairs: Vec<String> = Vec::new();
                                for p in ifg_html.split("href=\"") {
                                    if let Some(eq) = p.find('"') {
                                        let entry = p[..eq].trim_end_matches('/');
                                        if entry.len() == 17
                                            && entry.as_bytes().get(8) == Some(&b'_')
                                            && entry.chars().all(|c| c.is_ascii_digit() || c == '_')
                                        {
                                            pairs.push(entry.to_owned());
                                        }
                                    }
                                }
                                pairs.sort();
                                pairs.dedup();
                                let mut candidate_pairs: Vec<String> = Vec::new();
                                if let Some(tag) = requested_item_tag {
                                    for pr in &pairs {
                                        if tag.ends_with(pr) {
                                            candidate_pairs.push(pr.clone());
                                        }
                                    }
                                }
                                let filtered_by_date: Vec<String> = pairs
                                    .iter()
                                    .filter(|pr| {
                                        let sec = &pr[9..17];
                                        let after_start = start_ymd
                                            .as_deref()
                                            .map(|s| sec >= s)
                                            .unwrap_or(true);
                                        let before_end = sec <= end_ymd.as_str();
                                        after_start && before_end
                                    })
                                    .cloned()
                                    .collect();
                                let source_pool = if !filtered_by_date.is_empty() {
                                    &filtered_by_date
                                } else {
                                    &pairs
                                };
                                for pr in source_pool.iter().rev().take(2) {
                                    if !candidate_pairs.contains(pr) {
                                        candidate_pairs.push(pr.clone());
                                    }
                                }
                                let pref_upper = kw_sub_product
                                    .as_deref()
                                    .unwrap_or("")
                                    .to_ascii_uppercase();
                                let explicit_unw_only = pref_upper.contains("UNW")
                                    && !pref_upper.contains("COH")
                                    && !pref_upper.contains("CC");

                                for pair in candidate_pairs {
                                    let pair_dir = format!("{ifg_index_url}{pair}/");
                                    let Ok(pair_html) = fetch_text_retry(&pair_dir) else {
                                        continue;
                                    };
                                    let mut tifs: Vec<String> = Vec::new();
                                    let mut pngs: Vec<String> = Vec::new();
                                    for p in pair_html.split("href=\"") {
                                        if let Some(eq) = p.find('"') {
                                            let fn_entry = &p[..eq];
                                            if !fn_entry.contains('/') {
                                                if fn_entry.ends_with(".tif") {
                                                    tifs.push(fn_entry.to_owned());
                                                } else if fn_entry.ends_with(".png") {
                                                    pngs.push(fn_entry.to_owned());
                                                }
                                            }
                                        }
                                    }
                                    let preferred_tif = if explicit_unw_only {
                                        tifs.iter()
                                            .find(|f| f.ends_with(".geo.unw.tif"))
                                            .or_else(|| tifs.iter().find(|f| f.ends_with(".geo.cc.tif")))
                                            .or_else(|| tifs.first())
                                    } else {
                                        tifs.iter()
                                            .find(|f| f.ends_with(".geo.cc.tif"))
                                            .or_else(|| tifs.iter().find(|f| f.ends_with(".geo.unw.tif")))
                                            .or_else(|| tifs.first())
                                    };
                                    let Some(tif_fn) = preferred_tif else {
                                        continue;
                                    };
                                    let preferred_png = if tif_fn.contains("unw") {
                                        pngs.iter()
                                            .find(|f| f.ends_with(".geo.unw.png"))
                                            .or_else(|| pngs.iter().find(|f| f.ends_with(".geo.diff_pha.png")))
                                            .or_else(|| pngs.iter().find(|f| f.ends_with(".geo.cc.png")))
                                            .or_else(|| pngs.first())
                                    } else {
                                        pngs.iter()
                                            .find(|f| f.ends_with(".geo.cc.png"))
                                            .or_else(|| pngs.iter().find(|f| f.ends_with(".geo.unw.png")))
                                            .or_else(|| pngs.first())
                                    };
                                    let browse_png_url = preferred_png
                                        .map(|f| format!("{pair_dir}{f}"))
                                        .unwrap_or_else(|| meta_mli_png.clone());
                                    let preview_data_url = fetch_preview_png_data_url(&client, &browse_png_url)
                                        .or_else(|| {
                                            if browse_png_url != meta_mli_png {
                                                fetch_preview_png_data_url(&client, &meta_mli_png)
                                            } else {
                                                None
                                            }
                                        });
                                    let ptype = if tif_fn.contains("unw") {
                                        "GEOCODED_UNW_GEOTIFF"
                                    } else if tif_fn.contains("cc") {
                                        "GEOCODED_COHERENCE_GEOTIFF"
                                    } else {
                                        "GEOCODED_IFG_GEOTIFF"
                                    };
                                    let sec_dt = format!(
                                        "{}-{}-{}T00:00:00Z",
                                        &pair[9..13],
                                        &pair[13..15],
                                        &pair[15..17]
                                    );
                                    verified_scenes.push(VerifiedLicsarScene {
                                        item_id: format!("LiCSAR_{frame_id}_{pair}"),
                                        track,
                                        frame_id: frame_id.clone(),
                                        pair: Some(pair),
                                        datetime_iso: sec_dt,
                                        frame_bbox,
                                        polygon_ring: ring.clone(),
                                        tif_url: format!("{pair_dir}{tif_fn}"),
                                        browse_png_url: Some(browse_png_url),
                                        preview_data_url,
                                        product_type: ptype.to_owned(),
                                    });
                                    added_for_frame += 1;
                                    if verified_scenes.len() >= max_scenes {
                                        break;
                                    }
                                }
                            }
                        }
                        if added_for_frame == 0 && verified_scenes.len() < max_scenes {
                            let hgt_url = format!(
                                "https://gws-access.jasmin.ac.uk/public/nceo_geohazards/LiCSAR_products/{track}/{frame_id}/metadata/{frame_id}.geo.hgt.tif"
                            );
                            let preview_data_url = fetch_preview_png_data_url(&client, &meta_mli_png);
                            verified_scenes.push(VerifiedLicsarScene {
                                item_id: format!("LiCSAR_{frame_id}"),
                                track,
                                frame_id,
                                pair: None,
                                datetime_iso: format!("{end_dt}T00:00:00Z"),
                                frame_bbox,
                                polygon_ring: ring,
                                tif_url: hgt_url,
                                browse_png_url: Some(meta_mli_png),
                                preview_data_url,
                                product_type: "GEOCODED_HGT_GEOTIFF".to_owned(),
                            });
                        }
                        if verified_scenes.len() >= max_scenes {
                            break;
                        }
                    }
                    if verified_scenes.len() >= max_scenes {
                        break;
                    }
                }

                let mut items = Vec::new();
                for vs in &verified_scenes {
                    let tif_filename = vs
                        .tif_url
                        .rsplit('/')
                        .next()
                        .filter(|s| s.ends_with(".tif"))
                        .unwrap_or("licsar_product.tif");
                    let expected_out_name = format!("LiCSAR_{}_{}", vs.frame_id, tif_filename);
                    let existing_local = has_explicit_out_dir
                        .then(|| out_dir.join(&expected_out_name))
                        .and_then(|p| existing_valid_science_file_bytes(&p).map(|sz| (p, sz)));
                    let licsar_orbit_dir = if vs.frame_id.contains('A') {
                        Some("ASCENDING")
                    } else if vs.frame_id.contains('D') {
                        Some("DESCENDING")
                    } else {
                        None
                    };
                    items.push(json!({
                        "id": vs.item_id,
                        "title": expected_out_name,
                        "datetime": vs.datetime_iso,
                        "frame_id": vs.frame_id,
                        "pair": vs.pair,
                        "track": vs.track,
                        "flight_direction": licsar_orbit_dir,
                        "product_type": vs.product_type,
                        "href": vs.tif_url,
                        "browse_url": vs.browse_png_url,
                        "thumbnail_url": vs.preview_data_url,
                        "preview_url": vs.preview_data_url,
                        "already_downloaded": existing_local.is_some(),
                        "local_file_path": existing_local.as_ref().map(|(p, _)| p.to_string_lossy().to_string()),
                        "size_bytes": existing_local.as_ref().map(|(_, sz)| *sz),
                        "bbox": vs.frame_bbox,
                        "geometry": {
                            "type": "Polygon",
                            "coordinates": [vs.polygon_ring]
                        }
                    }));
                }

                let mut downloaded_file: Option<String> = None;
                let mut downloaded_bytes: u64 = 0;
                let mut already_exists = false;
                let mut target_item_id_out: Option<String> = None;
                if should_download {
                    let chosen_vs = if let Some(tag) = requested_item_tag {
                        verified_scenes
                            .iter()
                            .find(|vs| vs.item_id == tag)
                            .or_else(|| {
                                verified_scenes
                                    .iter()
                                    .find(|vs| tag.starts_with(&format!("LiCSAR_{}", vs.frame_id)))
                            })
                            .or_else(|| verified_scenes.first())
                    } else {
                        verified_scenes.first()
                    }
                    .ok_or_else(|| {
                        "LICSAR_NO_INTERSECTING_FRAMES: JASMIN 公共归档中未找到真实边界覆盖当前 AOI 的 LiCSAR Frame"
                            .to_owned()
                    })?;
                    target_item_id_out = Some(chosen_vs.item_id.clone());
                    let tif_filename = chosen_vs
                        .tif_url
                        .rsplit('/')
                        .next()
                        .filter(|s| s.ends_with(".tif"))
                        .unwrap_or("licsar_product.tif");
                    let out_file = out_dir.join(format!("LiCSAR_{}_{}", chosen_vs.frame_id, tif_filename));

                    if let Some(existing_len) = existing_valid_science_file_bytes(&out_file) {
                        already_exists = true;
                        downloaded_bytes = existing_len;
                        downloaded_file = Some(out_file.to_string_lossy().to_string());
                    } else {
                        let raw_dl_file = out_dir.join(format!(".tmp_LiCSAR_{}_{}", chosen_vs.frame_id, tif_filename));

                        // JASMIN LiCSAR GeoTIFFs are strip-encoded (not tiled COGs). Download in a single
                        // contiguous HTTPS stream first to avoid hundreds of slow `/vsicurl/` strip Range requests,
                        // then clip locally if native GDAL is available.
                        let dl_client = build_http_client_with_timeout(app_data_dir, 120)?;
                        let mut saved_ok = false;
                        let mut last_dl_err = String::new();
                        for attempt in 0..3 {
                            match dl_client.get(&chosen_vs.tif_url).send() {
                                Ok(resp) if resp.status().is_success() => {
                                    match stream_http_to_file_verified(
                                        resp,
                                        &raw_dl_file,
                                        MAX_SCIENCE_DOWNLOAD_BYTES,
                                        "LICSAR_GEOTIFF_DOWNLOAD",
                                    ) {
                                        Ok(written) => {
                                            downloaded_bytes = written;
                                            saved_ok = true;
                                            break;
                                        }
                                        Err(e) => last_dl_err = e,
                                    }
                                }
                                Ok(resp) => last_dl_err = format!("HTTP {}", resp.status()),
                                Err(e) => last_dl_err = e.to_string(),
                            }
                            if attempt < 2 {
                                std::thread::sleep(Duration::from_millis(600));
                            }
                        }
                        if !saved_ok {
                            let _ = fs::remove_file(&raw_dl_file);
                            return Err(format!(
                                "LICSAR_GEOTIFF_DOWNLOAD_FAILED: 无法从 JASMIN 下载真实 GeoTIFF ({}): {last_dl_err}",
                                chosen_vs.tif_url
                            ));
                        }

                        let engine = discover_native_gis_engine();
                        let mut clipped_local = false;
                        if engine.is_discovered() {
                            let proc_req = RasterProcessRequest {
                                sources: vec![raw_dl_file.to_string_lossy().to_string()],
                                output_path: out_file.clone(),
                                operation: "clip".to_owned(),
                                crop_bbox: Some(bbox),
                                band_a: Some(1),
                                ..Default::default()
                            };
                            if let Ok(proc_res) = engine.process_raster_window(&proc_req) {
                                if proc_res["ok"] == true && out_file.is_file() {
                                    let written = fs::metadata(&out_file).map(|m| m.len()).unwrap_or(0);
                                    if written > 100 {
                                        downloaded_bytes = written;
                                        clipped_local = true;
                                    }
                                }
                            }
                        }
                        if clipped_local {
                            let _ = fs::remove_file(&raw_dl_file);
                        } else {
                            fs::rename(&raw_dl_file, &out_file)
                                .map_err(|e| format!("LICSAR_RENAME_FAILED: {e}"))?;
                        }
                        let head = fs::read(&out_file).map_err(|e| format!("LICSAR_READ_VERIFY_FAILED: {e}"))?;
                        if head.len() < 4 || !(head.starts_with(b"II*\0") || head.starts_with(b"MM\0*")) {
                            let _ = fs::remove_file(&out_file);
                            return Err(
                                "LICSAR_INVALID_TIFF_MAGIC: 下载的 LiCSAR 文件不是有效 GeoTIFF (缺少 49492a00 / 4d4d002a 幻数)"
                                    .to_owned(),
                            );
                        }
                        downloaded_file = Some(out_file.to_string_lossy().to_string());
                    }
                }

                let primary_track = verified_scenes
                    .first()
                    .map(|f| f.track)
                    .unwrap_or_else(|| candidate_tracks[0]);
                let mut out_json = json!({
                    "ok": true,
                    "dataset": "comet_licsar",
                    "track": primary_track,
                    "candidate_tracks": candidate_tracks,
                    "live_query_endpoint": primary_endpoint,
                    "count": items.len(),
                    "already_exists": already_exists,
                    "target_item_id": target_item_id_out,
                    "downloaded_file": downloaded_file,
                    "downloaded_path": downloaded_file,
                    "downloaded_bytes": downloaded_bytes,
                    "items": items,
                });
                if let Some(ref dl_s) = downloaded_file {
                    attach_raster_preview_fields(&mut out_json, Path::new(dl_s));
                } else if let Some(first_vs) = verified_scenes.first() {
                    if let Some(ref pdata) = first_vs.preview_data_url {
                        out_json["preview_data_url"] = json!(pdata);
                        out_json["mosaic_bbox"] = json!(first_vs.frame_bbox);
                    }
                }
                Ok(out_json)
            }
        }
        "ionospheric_ionex" | "igs_gim" => {
            let bbox = req
                .bbox
                .unwrap_or([-180.0, -90.0, 180.0, 90.0]);
            let search_only = has_search_only_token(req.keyword.as_deref())
                && req.tool_name.as_deref() != Some("download");
            let explicit_out_dir = req
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            let out_dir = explicit_out_dir
                .clone()
                .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join("ionospheric_ionex"));

            // Parse pipe-delimited keyword `${startDate}..${endDate}|${analysisCenter}|download`
            let mut kw_start_date: Option<String> = None;
            let mut kw_agency: Option<String> = None;
            if let Some(raw_kw) = req.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                for token in raw_kw.split('|').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some((s, _e)) = token.split_once("..") {
                        let s_trim = s.trim();
                        if !s_trim.is_empty() {
                            kw_start_date = Some(s_trim.to_owned());
                        }
                    } else if token.eq_ignore_ascii_case("download")
                        || token.eq_ignore_ascii_case("search")
                        || token.eq_ignore_ascii_case("preview")
                    {
                        // ignore action verb
                    } else if kw_agency.is_none() {
                        kw_agency = Some(token.to_owned());
                    }
                }
            }

            let date_str = req
                .start_date
                .as_deref()
                .filter(|s| !s.is_empty())
                .or(kw_start_date.as_deref())
                .or(req.code.as_deref().filter(|s| s.contains('-')))
                .ok_or_else(|| "IONEX_MISSING_DATE: 请提供 IONEX 观测日期 (YYYY-MM-DD)".to_owned())?;
            let mut date_parts = date_str.split('-');
            let year = date_parts
                .next()
                .and_then(|s| s.parse::<i32>().ok())
                .ok_or_else(|| format!("IONEX_INVALID_YEAR: 无法解析日期 {date_str}"))?;
            let month = date_parts
                .next()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or_else(|| format!("IONEX_INVALID_MONTH: 无法解析日期 {date_str}"))?;
            let day = date_parts
                .next()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or_else(|| format!("IONEX_INVALID_DAY: 无法解析日期 {date_str}"))?;

            let doy = calculate_day_of_year(year, month, day)?;
            let (gps_week, _gps_dow) = gps_week_and_dow(year, month, day)?;
            let yy = (year % 100).unsigned_abs();

            let agency = kw_agency
                .as_deref()
                .or(req.code.as_deref().filter(|s| !s.contains('-')))
                .filter(|s| !s.is_empty())
                .unwrap_or("COD");
            let agency_prefix = match agency.trim().to_ascii_uppercase().as_str() {
                "COD" | "CODG" | "CODE" => "COD",
                "IGS" | "IGSG" => "IGS",
                "JPL" | "JPLG" => "JPL",
                "ESA" | "ESAG" => "ESA",
                other => {
                    return Err(format!(
                        "IONEX_UNSUPPORTED_CENTER: 不支持的电离层分析中心 '{other}'（仅支持 COD / IGS / JPL / ESA）"
                    ))
                }
            };

            let filename = format!("{agency_prefix}G{doy:03}0.{yy:02}I");
            let ionex_file = out_dir.join(&filename);
            let agency_lower = agency_prefix.to_ascii_lowercase();
            let item_id = format!("{agency_prefix}_{year}_{doy:03}_IONEX_VTEC");

            let existing_ionex_bytes: Option<u64> = if ionex_file.is_file() {
                fs::read_to_string(&ionex_file)
                    .ok()
                    .filter(|txt| txt.len() > 256 && txt.contains("IONEX VERSION / TYPE"))
                    .map(|txt| txt.len() as u64)
            } else {
                None
            };

            // Strictly agency-authentic public HTTP/HTTPS archives (NEVER request CODG URL and mislabel as IGS/JPL/ESA)
            let mirror_urls: Vec<String> = match agency_prefix {
                "COD" => vec![
                    format!("http://garner.ucsd.edu/pub/products/{gps_week}/COD0OPSFIN_{year}{doy:03}0000_01D_01H_GIM.INX.gz"),
                    format!("http://garner.ucsd.edu/pub/products/{gps_week}/CODKOPSFIN_{year}{doy:03}0000_01D_01H_GIM.INX.Z"),
                    format!("http://garner.ucsd.edu/pub/products/{gps_week}/codg{doy:03}0.{yy:02}i.Z"),
                    format!("http://ftp.aiub.unibe.ch/CODE/{year}/COD0OPSFIN_{year}{doy:03}0000_01D_01H_GIM.INX.gz"),
                    format!("http://ftp.aiub.unibe.ch/CODE/{year}/CODG{doy:03}0.{yy:02}I.Z"),
                    format!("http://ftp.aiub.unibe.ch/CODE/{year}/CODG{doy:03}0.{yy:02}I"),
                    format!("https://igs.bkg.bundesamt.de/root_ftp/IGS/products/ionosphere/{gps_week}/COD0OPSFIN_{year}{doy:03}0000_01D_01H_GIM.INX.gz"),
                    format!("https://igs.bkg.bundesamt.de/root_ftp/IGS/products/ionosphere/{gps_week}/codg{doy:03}0.{yy:02}i.Z"),
                ],
                "JPL" => vec![
                    format!("https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y{year}/JPL0OPSFIN_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y{year}/JPLG{doy:03}0.{yy:02}I.gz"),
                    format!("https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_rapid/y{year}/JPL0OPSRAP_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_rapid/y{year}/JPRG{doy:03}0.{yy:02}I.gz"),
                    format!("http://garner.ucsd.edu/pub/products/{gps_week}/JPL0OPSFIN_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("http://garner.ucsd.edu/pub/products/{gps_week}/jplg{doy:03}0.{yy:02}i.Z"),
                    format!("https://igs.bkg.bundesamt.de/root_ftp/IGS/products/ionosphere/{gps_week}/JPL0OPSFIN_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("https://igs.bkg.bundesamt.de/root_ftp/IGS/products/ionosphere/{gps_week}/jplg{doy:03}0.{yy:02}i.Z"),
                ],
                "ESA" => vec![
                    format!("http://garner.ucsd.edu/pub/products/{gps_week}/ESA0OPSFIN_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("http://garner.ucsd.edu/pub/products/{gps_week}/esag{doy:03}0.{yy:02}i.Z"),
                    format!("http://navigation-office.esa.int/products/gnss-products/{gps_week}/ESA0OPSFIN_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("http://navigation-office.esa.int/products/gnss-products/{gps_week}/esag{doy:03}0.{yy:02}i.Z"),
                    format!("https://igs.bkg.bundesamt.de/root_ftp/IGS/products/ionosphere/{gps_week}/ESA0OPSFIN_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("https://igs.bkg.bundesamt.de/root_ftp/IGS/products/ionosphere/{gps_week}/esag{doy:03}0.{yy:02}i.Z"),
                ],
                _ => vec![
                    format!("https://igs.bkg.bundesamt.de/root_ftp/IGS/products/ionosphere/{gps_week}/IGS0OPSFIN_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("https://igs.bkg.bundesamt.de/root_ftp/IGS/products/ionosphere/{gps_week}/igsg{doy:03}0.{yy:02}i.Z"),
                    format!("https://igs.bkg.bundesamt.de/root_ftp/IGS/products/ionosphere/{gps_week}/IGSG{doy:03}0.{yy:02}I.Z"),
                    format!("https://igs.ign.fr/pub/igs/products/ionosphere/{gps_week}/IGS0OPSFIN_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("https://igs.ign.fr/pub/igs/products/ionosphere/{gps_week}/igsg{doy:03}0.{yy:02}i.Z"),
                    format!("http://garner.ucsd.edu/pub/products/{gps_week}/IGS0OPSFIN_{year}{doy:03}0000_01D_02H_GIM.INX.gz"),
                    format!("http://garner.ucsd.edu/pub/products/{gps_week}/igsg{doy:03}0.{yy:02}i.Z"),
                ],
            };

            if search_only {
                let primary_url = mirror_urls.first().cloned().unwrap_or_default();
                let local_file_opt = if existing_ionex_bytes.is_some() && explicit_out_dir.is_some() {
                    Some(ionex_file.to_string_lossy().to_string())
                } else {
                    None
                };
                let items = vec![json!({
                    "id": item_id,
                    "title": format!("{agency_prefix} Global Ionosphere Map (IONEX) {year}-{month:02}-{day:02} (DOY {doy:03}, GPS Week {gps_week})"),
                    "datetime": format!("{year}-{month:02}-{day:02}T00:00:00Z"),
                    "center": agency_prefix,
                    "doy": doy,
                    "gps_week": gps_week,
                    "product_type": "IONEX_VTEC_MAP",
                    "source_url": primary_url,
                    "href": primary_url,
                    "download_url": primary_url,
                    "bbox": bbox,
                    "already_downloaded": local_file_opt.is_some(),
                    "local_file_path": local_file_opt,
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[
                            [bbox[0], bbox[1]],
                            [bbox[2], bbox[1]],
                            [bbox[2], bbox[3]],
                            [bbox[0], bbox[3]],
                            [bbox[0], bbox[1]]
                        ]]
                    }
                })];
                return Ok(json!({
                    "ok": true,
                    "dataset": "ionospheric_ionex",
                    "analysis_center": agency_prefix,
                    "date": format!("{year}-{month:02}-{day:02}"),
                    "doy": doy,
                    "gps_week": gps_week,
                    "filename": filename,
                    "count": 1,
                    "already_exists": local_file_opt.is_some(),
                    "downloaded_file": Value::Null,
                    "downloaded_path": Value::Null,
                    "downloaded_bytes": 0,
                    "grid_resolution": "2.5x5.0 deg",
                    "items": items,
                }));
            }

            fs::create_dir_all(&out_dir).map_err(|e| format!("CREATE_DIR_FAILED: {e}"))?;

            if let Some(existing_bytes) = existing_ionex_bytes {
                let ionex_file_str = ionex_file.to_string_lossy().to_string();
                let primary_url = mirror_urls.first().cloned().unwrap_or_default();
                let items = vec![json!({
                    "id": item_id.clone(),
                    "title": format!("{agency_prefix} Global Ionosphere Map (IONEX) {year}-{month:02}-{day:02} (DOY {doy:03}, GPS Week {gps_week})"),
                    "datetime": format!("{year}-{month:02}-{day:02}T00:00:00Z"),
                    "center": agency_prefix,
                    "doy": doy,
                    "gps_week": gps_week,
                    "product_type": "IONEX_VTEC_MAP",
                    "source_url": primary_url,
                    "href": ionex_file_str.clone(),
                    "download_url": primary_url,
                    "bbox": bbox,
                    "already_downloaded": true,
                    "local_file_path": ionex_file_str.clone(),
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[
                            [bbox[0], bbox[1]],
                            [bbox[2], bbox[1]],
                            [bbox[2], bbox[3]],
                            [bbox[0], bbox[3]],
                            [bbox[0], bbox[1]]
                        ]]
                    }
                })];
                return Ok(json!({
                    "ok": true,
                    "dataset": "ionospheric_ionex",
                    "analysis_center": agency_prefix,
                    "date": format!("{year}-{month:02}-{day:02}"),
                    "doy": doy,
                    "gps_week": gps_week,
                    "filename": filename,
                    "ionex_file": ionex_file_str.clone(),
                    "count": 1,
                    "already_exists": true,
                    "target_item_id": item_id,
                    "downloaded_file": ionex_file_str.clone(),
                    "downloaded_path": ionex_file_str,
                    "downloaded_bytes": existing_bytes,
                    "grid_resolution": "2.5x5.0 deg",
                    "items": items,
                }));
            }

            let client = build_http_client(app_data_dir)?;
            let mut raw_payload: Option<(String, Vec<u8>)> = None;
            let mut mirror_errors: Vec<String> = Vec::new();

            for url in &mirror_urls {
                match client.get(url).timeout(Duration::from_secs(12)).send() {
                    Ok(resp) if resp.status().is_success() => match resp.bytes() {
                        Ok(b) if !b.is_empty() => {
                            let head_txt = String::from_utf8_lossy(&b[..b.len().min(256)]).to_ascii_lowercase();
                            if head_txt.contains("<!doctype") || head_txt.contains("<html") {
                                mirror_errors.push(format!("{url} -> HTML error page"));
                                continue;
                            }
                            raw_payload = Some((url.clone(), b.to_vec()));
                            break;
                        }
                        Ok(_) => mirror_errors.push(format!("{url} -> empty body")),
                        Err(e) => mirror_errors.push(format!("{url} -> read error: {e}")),
                    },
                    Ok(resp) => mirror_errors.push(format!("{url} -> HTTP {}", resp.status())),
                    Err(e) => mirror_errors.push(format!("{url} -> {e}")),
                }
            }

            // Authenticated NASA CDDIS fallback for the exact requested analysis center if Earthdata token is configured
            if raw_payload.is_none() {
                if let Ok(token) = insar_download::resolve_earthdata_search_token() {
                    let rate_tag = if agency_prefix == "COD" { "01H" } else { "02H" };
                    let cddis_urls = [
                        format!("https://cddis.nasa.gov/archive/gnss/products/ionex/{year}/{doy:03}/{agency_prefix}0OPSFIN_{year}{doy:03}0000_01D_{rate_tag}_GIM.INX.gz"),
                        format!("https://cddis.nasa.gov/archive/gnss/products/ionex/{year}/{doy:03}/{agency_lower}g{doy:03}0.{yy:02}i.Z"),
                    ];
                    for cddis_url in &cddis_urls {
                        if let Ok(resp) = client
                            .get(cddis_url)
                            .header("Authorization", format!("Bearer {token}"))
                            .timeout(Duration::from_secs(15))
                            .send()
                        {
                            if resp.status().is_success() {
                                if let Ok(bytes) = resp.bytes() {
                                    let head_txt = String::from_utf8_lossy(&bytes[..bytes.len().min(256)])
                                        .to_ascii_lowercase();
                                    if !bytes.is_empty()
                                        && !head_txt.contains("<!doctype")
                                        && !head_txt.contains("<html")
                                    {
                                        raw_payload = Some((cddis_url.clone(), bytes.to_vec()));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let (source_url, raw_bytes) = raw_payload.ok_or_else(|| {
                format!(
                    "IONEX_ALL_MIRRORS_FAILED: 无法获取 {agency_prefix} {year}-{month:02}-{day:02} (DOY {doy:03}, GPS Week {gps_week}) 的真实 IONEX 观测文件；已拒绝任何合成模型回退。尝试记录: {}",
                    mirror_errors.join(" | ")
                )
            })?;

            let mut compressed_archive_file: Option<String> = None;
            let decompressed_bytes = if raw_bytes.starts_with(&[0x1f, 0x9d]) {
                let comp_path = out_dir.join(format!("{filename}.Z"));
                let _ = fs::write(&comp_path, &raw_bytes);
                compressed_archive_file = Some(comp_path.to_string_lossy().to_string());
                decompress_unix_lzw_z(&raw_bytes)?
            } else if raw_bytes.starts_with(&[0x1f, 0x8b]) {
                let comp_path = out_dir.join(format!("{filename}.gz"));
                let _ = fs::write(&comp_path, &raw_bytes);
                compressed_archive_file = Some(comp_path.to_string_lossy().to_string());
                decompress_gzip_bytes(&raw_bytes)?
            } else {
                raw_bytes
            };

            let ionex_text = String::from_utf8(decompressed_bytes)
                .map_err(|e| format!("IONEX_UTF8_DECODE_FAILED ({source_url}): {e}"))?;
            if !ionex_text.contains("IONEX VERSION / TYPE") || !ionex_text.contains("START OF TEC MAP") {
                return Err(format!(
                    "IONEX_HEADER_VALIDATION_FAILED: 远端文件 ({source_url}) 解压后不包含标准 'IONEX VERSION / TYPE' 或 'START OF TEC MAP' 数据块"
                ));
            }

            fs::write(&ionex_file, ionex_text.as_bytes())
                .map_err(|e| format!("WRITE_IONEX_FAILED: {e}"))?;
            let downloaded_bytes = ionex_text.len() as u64;
            let maps_count = parse_ionex_maps_count(&ionex_text);

            // Extract ONLY real DIFFERENTIAL CODE BIASES auxiliary block if present (never duplicate full IONEX!)
            let dcb_file_opt = if let Some(dcb_block) = extract_ionex_dcb_block(&ionex_text) {
                let dcb_path = out_dir.join(format!("{agency_prefix}G{doy:03}0.DCBi"));
                fs::write(&dcb_path, dcb_block.as_bytes())
                    .map_err(|e| format!("WRITE_DCB_FAILED: {e}"))?;
                Some(dcb_path.to_string_lossy().to_string())
            } else {
                None
            };

            let ionex_file_str = ionex_file.to_string_lossy().to_string();
            let items = vec![json!({
                "id": item_id.clone(),
                "title": format!("{agency_prefix} Global Ionosphere Map (IONEX) {year}-{month:02}-{day:02} (DOY {doy:03}, GPS Week {gps_week})"),
                "datetime": format!("{year}-{month:02}-{day:02}T00:00:00Z"),
                "center": agency_prefix,
                "doy": doy,
                "gps_week": gps_week,
                "product_type": "IONEX_VTEC_MAP",
                "source_url": source_url,
                "href": ionex_file_str.clone(),
                "download_url": source_url,
                "bbox": bbox,
                "already_downloaded": true,
                "local_file_path": ionex_file_str.clone(),
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [bbox[0], bbox[1]],
                        [bbox[2], bbox[1]],
                        [bbox[2], bbox[3]],
                        [bbox[0], bbox[3]],
                        [bbox[0], bbox[1]]
                    ]]
                }
            })];

            Ok(json!({
                "ok": true,
                "dataset": "ionospheric_ionex",
                "analysis_center": agency_prefix,
                "date": format!("{year}-{month:02}-{day:02}"),
                "doy": doy,
                "gps_week": gps_week,
                "source_url": source_url,
                "filename": filename,
                "ionex_file": ionex_file_str.clone(),
                "dcb_file": dcb_file_opt,
                "count": 1,
                "already_exists": false,
                "target_item_id": item_id,
                "downloaded_file": ionex_file_str.clone(),
                "downloaded_path": ionex_file_str,
                "compressed_archive_file": compressed_archive_file,
                "downloaded_bytes": downloaded_bytes,
                "maps_count": maps_count,
                "grid_resolution": "2.5x5.0 deg",
                "items": items,
            }))
        }
        "nisar_orbit_oe" => {
            let (raw_start, raw_end, sub_filter, should_download) =
                parse_geodata_window_and_sub(req, 30, 7, "ALL");
            let win_start = if raw_start.contains('T') {
                raw_start.clone()
            } else {
                format!("{raw_start}T00:00:00Z")
            };
            let win_end = if raw_end.contains('T') {
                raw_end.clone()
            } else {
                format!("{raw_end}T23:59:59Z")
            };
            let temporal = format!("{win_start},{win_end}");
            let client = build_http_client(app_data_dir)?;
            let resp = http_get_with_retry(
                &client,
                "https://cmr.earthdata.nasa.gov/search/granules.json",
                Some(&[
                    ("short_name", "NISAR_OE"),
                    ("temporal", temporal.as_str()),
                    ("page_size", "80"),
                    ("sort_key", "-start_date"),
                ]),
                3,
            )
            .map_err(|err| format!("NISAR_OE_CMR_QUERY_FAILED: {err}"))?
            .error_for_status()
            .map_err(|err| format!("NISAR_OE_CMR_HTTP_ERROR: {err}"))?;
            let json_val: Value = resp
                .json()
                .map_err(|err| format!("NISAR_OE_CMR_JSON_FAILED: {err}"))?;
            let raw_entries = json_val["feed"]["entry"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let items = parse_nisar_oe_cmr_entries(&raw_entries, &sub_filter);

            let mut downloaded_file: Option<String> = None;
            let mut downloaded_bytes: u64 = 0;
            let mut already_exists = false;
            let mut target_item_id_out: Option<String> = None;

            if should_download {
                const AUTH_ERR_MSG: &str = "EARTHDATA_AUTH_REQUIRED: 下载 NISAR 精密/标称轨道历书需先在「设置 -> 数据服务账号」配置 NASA Earthdata 账号";
                if items.is_empty() {
                    return Err(format!(
                        "NISAR_OE_NO_GRANULES: 在 {raw_start} 至 {raw_end} ({sub_filter}) 窗口内未检索到 NISAR_OE 轨道历书"
                    ));
                }
                let target_item =
                    select_open_eo_target_item(&items, req.osm_tag.as_deref())?;
                let item_id = target_item["id"].as_str().unwrap_or("nisar_oe").to_owned();
                target_item_id_out = Some(item_id.clone());
                let data_url = target_item["href"].as_str().unwrap_or("").to_owned();
                if data_url.is_empty() {
                    return Err(format!(
                        "NISAR_OE_MISSING_XML_URL: {item_id} 未提供 .xml 轨道历书下载链接"
                    ));
                }
                let out_dir = req
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join("NISAR_OE"));
                fs::create_dir_all(&out_dir).map_err(|e| format!("CREATE_DIR_FAILED: {e}"))?;
                let file_stem = item_id.trim_end_matches(".xml");
                let fpath = out_dir.join(format!("{file_stem}.xml"));

                if let Some(existing_bytes) = existing_valid_science_file_bytes(&fpath) {
                    already_exists = true;
                    downloaded_bytes = existing_bytes;
                    downloaded_file = Some(fpath.to_string_lossy().to_string());
                } else {
                    let auth = resolve_earthdata_auth_materials(AUTH_ERR_MSG)?;
                    let edl_client = build_edl_manual_redirect_client(app_data_dir)?;
                    let dl_resp =
                        execute_edl_manual_get(&edl_client, &data_url, &auth, AUTH_ERR_MSG)?;
                    let written = stream_http_to_file_verified(
                        dl_resp,
                        &fpath,
                        MAX_SCIENCE_DOWNLOAD_BYTES,
                        &format!("NISAR_OE_DOWNLOAD ({item_id})"),
                    )?;
                    let head = {
                        use std::io::Read;
                        let mut f = fs::File::open(&fpath)
                            .map_err(|e| format!("READ_ORBIT_XML_FAILED: {e}"))?;
                        let mut b = [0u8; 128];
                        let n = f.read(&mut b).unwrap_or(0);
                        String::from_utf8_lossy(&b[..n])
                            .trim_start()
                            .to_ascii_lowercase()
                    };
                    if head.starts_with("<!doctype")
                        || head.starts_with("<html")
                        || head.contains("earthdata login")
                    {
                        let _ = fs::remove_file(&fpath);
                        return Err(AUTH_ERR_MSG.to_owned());
                    }
                    if !(head.starts_with("<?xml")
                        || head.starts_with("<earth_explorer")
                        || head.starts_with("<nisar")
                        || head.starts_with("<orbit")
                        || head.starts_with("<"))
                    {
                        let _ = fs::remove_file(&fpath);
                        return Err(format!(
                            "NISAR_OE_INVALID_XML: 下载的 {item_id} 非合法轨道 XML 文件"
                        ));
                    }
                    downloaded_bytes = written;
                    downloaded_file = Some(fpath.to_string_lossy().to_string());
                }
            }

            Ok(json!({
                "ok": true,
                "dataset": "nisar_orbit_oe",
                "provider": "NASA ASF / JPL CMR (NISAR_OE)",
                "start_date": win_start,
                "end_date": win_end,
                "count": items.len(),
                "already_exists": already_exists,
                "target_item_id": target_item_id_out,
                "downloaded_file": downloaded_file,
                "downloaded_path": downloaded_file,
                "downloaded_bytes": downloaded_bytes,
                "items": items,
            }))
        }
        "copernicus_pod_orbit" => {
            let (raw_start, raw_end, sub_filter, should_download) =
                parse_geodata_window_and_sub(req, 45, 7, "ALL");
            let win_start = if raw_start.contains('T') {
                raw_start.clone()
            } else {
                format!("{raw_start}T00:00:00.000Z")
            };
            let win_end = if raw_end.contains('T') {
                raw_end.clone()
            } else {
                format!("{raw_end}T23:59:59.999Z")
            };
            let upper_sub = sub_filter.trim().to_ascii_uppercase();
            let mission_clause = if upper_sub == "S2" {
                "(Collection/Name eq 'SENTINEL-2' and contains(Name,'AUX_POEORB'))"
            } else if upper_sub == "S3" {
                "(Collection/Name eq 'SENTINEL-3' and contains(Name,'SR___POE_AX'))"
            } else {
                "((Collection/Name eq 'SENTINEL-2' and contains(Name,'AUX_POEORB')) or (Collection/Name eq 'SENTINEL-3' and contains(Name,'SR___POE_AX')))"
            };
            let filter_str = format!(
                "{mission_clause} and ContentDate/Start ge {win_start} and ContentDate/Start le {win_end}"
            );
            let client = build_http_client(app_data_dir)?;
            let resp = http_get_with_retry(
                &client,
                "https://catalogue.dataspace.copernicus.eu/odata/v1/Products",
                Some(&[
                    ("$filter", filter_str.as_str()),
                    ("$orderby", "ContentDate/Start desc"),
                    ("$top", "60"),
                ]),
                3,
            )
            .map_err(|err| format!("CDSE_POD_QUERY_FAILED: {err}"))?
            .error_for_status()
            .map_err(|err| format!("CDSE_POD_HTTP_ERROR: {err}"))?;
            let json_val: Value = resp
                .json()
                .map_err(|err| format!("CDSE_POD_JSON_PARSE_FAILED: {err}"))?;
            let entries = json_val["value"].as_array().cloned().unwrap_or_default();
            let items = parse_copernicus_pod_odata_entries(&entries);

            let mut downloaded_file: Option<String> = None;
            let mut downloaded_bytes: u64 = 0;
            let mut already_exists = false;
            let mut target_item_id_out: Option<String> = None;

            if should_download {
                const CDSE_AUTH_ERR: &str =
                    "CDSE_AUTH_REQUIRED: 下载哥白尼精密定轨产品需先在「设置 -> 数据服务账号」配置 CDSE 账号";
                if items.is_empty() {
                    return Err(format!(
                        "CDSE_POD_NO_PRODUCTS: 在 {raw_start} 至 {raw_end} ({upper_sub}) 窗口内未检索到 Copernicus POD 精密轨道"
                    ));
                }
                let target_item = if let Some(target_id) =
                    req.osm_tag.as_deref().map(str::trim).filter(|s| !s.is_empty())
                {
                    items
                        .iter()
                        .find(|it| it["id"].as_str() == Some(target_id))
                        .unwrap_or(&items[0])
                } else {
                    &items[0]
                };
                let item_id = target_item["id"].as_str().unwrap_or("cdse_pod").to_owned();
                let raw_filename = target_item["filename"].as_str().unwrap_or(&item_id);
                let data_url = target_item["href"].as_str().unwrap_or("").to_owned();
                target_item_id_out = Some(item_id.clone());

                let out_dir = req
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        app_data_dir
                            .join("geodata_downloads")
                            .join("CDSE_POD_ORBIT")
                    });
                fs::create_dir_all(&out_dir).map_err(|e| format!("CREATE_DIR_FAILED: {e}"))?;
                let stem = item_id.trim_end_matches(".EOF").trim_end_matches(".zip");
                let candidate_eof = out_dir.join(format!("{stem}.EOF"));
                let candidate_zip = out_dir.join(format!("{stem}.zip"));

                if let Some(existing) = existing_valid_science_file_bytes(&candidate_eof) {
                    already_exists = true;
                    downloaded_bytes = existing;
                    downloaded_file = Some(candidate_eof.to_string_lossy().to_string());
                } else if let Some(existing) = existing_valid_science_file_bytes(&candidate_zip) {
                    already_exists = true;
                    downloaded_bytes = existing;
                    downloaded_file = Some(candidate_zip.to_string_lossy().to_string());
                } else {
                    let (cdse_user, cdse_pass) = resolve_cdse_credentials(CDSE_AUTH_ERR)?;
                    let token_resp = client
                        .post("https://identity.dataspace.copernicus.eu/auth/realms/CDSE/protocol/openid-connect/token")
                        .form(&[
                            ("client_id", "cdse-public"),
                            ("grant_type", "password"),
                            ("username", cdse_user.as_str()),
                            ("password", cdse_pass.as_str()),
                        ])
                        .send()
                        .map_err(|e| format!("CDSE_POD_TOKEN_REQUEST_FAILED: {e}"))?;
                    if !token_resp.status().is_success() {
                        return Err(CDSE_AUTH_ERR.to_owned());
                    }
                    let token_json: Value = token_resp
                        .json()
                        .map_err(|e| format!("CDSE_POD_TOKEN_PARSE_FAILED: {e}"))?;
                    let access_token = token_json["access_token"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| CDSE_AUTH_ERR.to_owned())?;

                    let dl_resp = client
                        .get(&data_url)
                        .bearer_auth(access_token)
                        .send()
                        .map_err(|e| format!("CDSE_POD_DOWNLOAD_FAILED ({data_url}): {e}"))?;
                    if dl_resp.status().as_u16() == 401 || dl_resp.status().as_u16() == 403 {
                        return Err(CDSE_AUTH_ERR.to_owned());
                    }
                    let tmp_path = out_dir.join(format!("{stem}.download.tmp"));
                    let written = stream_http_to_file_verified(
                        dl_resp,
                        &tmp_path,
                        MAX_SCIENCE_DOWNLOAD_BYTES,
                        &format!("CDSE_POD_DOWNLOAD ({item_id})"),
                    )?;
                    let head_bytes = {
                        use std::io::Read;
                        let mut f = fs::File::open(&tmp_path)
                            .map_err(|e| format!("READ_CDSE_POD_FAILED: {e}"))?;
                        let mut b = [0u8; 96];
                        let n = f.read(&mut b).unwrap_or(0);
                        b[..n].to_vec()
                    };
                    let head_lower = String::from_utf8_lossy(&head_bytes)
                        .trim_start()
                        .to_ascii_lowercase();
                    if head_lower.starts_with("<!doctype") || head_lower.starts_with("<html") {
                        let _ = fs::remove_file(&tmp_path);
                        return Err(CDSE_AUTH_ERR.to_owned());
                    }
                    let final_path = if head_bytes.starts_with(b"PK\x03\x04")
                        || !raw_filename.ends_with(".EOF")
                    {
                        out_dir.join(format!("{stem}.zip"))
                    } else {
                        out_dir.join(format!("{stem}.EOF"))
                    };
                    fs::rename(&tmp_path, &final_path)
                        .map_err(|e| format!("RENAME_CDSE_POD_FAILED: {e}"))?;
                    downloaded_bytes = written;
                    downloaded_file = Some(final_path.to_string_lossy().to_string());
                }
            }

            Ok(json!({
                "ok": true,
                "dataset": "copernicus_pod_orbit",
                "provider": "Copernicus Data Space Ecosystem (CDSE POD)",
                "start_date": win_start,
                "end_date": win_end,
                "count": items.len(),
                "already_exists": already_exists,
                "target_item_id": target_item_id_out,
                "downloaded_file": downloaded_file,
                "downloaded_path": downloaded_file,
                "downloaded_bytes": downloaded_bytes,
                "items": items,
            }))
        }
        "sentinel_2_l1c" | "sentinel_3_standard" => {
            let bbox = req
                .bbox
                .ok_or_else(|| format!("MISSING_BBOX: {dataset} 需要 bbox=[west, south, east, north]"))?;
            let w = bbox[0].min(bbox[2]);
            let s = bbox[1].min(bbox[3]);
            let e = bbox[0].max(bbox[2]);
            let n = bbox[1].max(bbox[3]);
            let poly_wkt = format!(
                "SRID=4326;POLYGON(({w:.4} {s:.4},{e:.4} {s:.4},{e:.4} {n:.4},{w:.4} {n:.4},{w:.4} {s:.4}))"
            );

            // Parse user's requested date window — NEVER substitute Oct 2024 / May 2024 / Aug 2023 fallback windows
            let mut kw_start_date: Option<String> = None;
            let mut kw_end_date: Option<String> = None;
            if let Some(raw_kw) = req.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                for token in raw_kw.split('|').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some((sd, ed)) = token.split_once("..") {
                        if !sd.trim().is_empty() {
                            kw_start_date = Some(sd.trim().to_owned());
                        }
                        if !ed.trim().is_empty() {
                            kw_end_date = Some(ed.trim().to_owned());
                        }
                    }
                }
            }
            let raw_start = req
                .start_date
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .or(kw_start_date.as_deref())
                .unwrap_or("2024-01-01");
            let raw_end = req
                .end_date
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .or(kw_end_date.as_deref())
                .unwrap_or("2024-12-31");
            let win_start = if raw_start.contains('T') {
                raw_start.to_owned()
            } else {
                format!("{raw_start}T00:00:00.000Z")
            };
            let win_end = if raw_end.contains('T') {
                raw_end.to_owned()
            } else {
                format!("{raw_end}T23:59:59.999Z")
            };

            let is_s2 = dataset == "sentinel_2_l1c";
            let coll_name = if is_s2 { "SENTINEL-2" } else { "SENTINEL-3" };
            let name_clause = if is_s2 {
                " and contains(Name,'MSIL1C')"
            } else {
                ""
            };
            let filter_str = format!(
                "Collection/Name eq '{coll_name}'{name_clause} and ContentDate/Start ge {win_start} and ContentDate/Start le {win_end} and OData.CSC.Intersects(area=geography'{poly_wkt}')"
            );
            let client = build_http_client(app_data_dir)?;
            let resp = http_get_with_retry(
                &client,
                "https://catalogue.dataspace.copernicus.eu/odata/v1/Products",
                Some(&[("$filter", filter_str.as_str()), ("$top", "60")]),
                3,
            )
            .map_err(|err| format!("CDSE_ODATA_QUERY_FAILED ({dataset}): {err}"))?
            .error_for_status()
            .map_err(|err| format!("CDSE_ODATA_HTTP_ERROR ({dataset}): {err}"))?;
            let json_val: Value = resp
                .json()
                .map_err(|err| format!("CDSE_ODATA_JSON_PARSE_FAILED ({dataset}): {err}"))?;
            let entries = json_val["value"].as_array().cloned().unwrap_or_default();

            let mut items = Vec::new();
            for entry in &entries {
                let name = entry["Name"]
                    .as_str()
                    .unwrap_or(if is_s2 { "S2A_MSIL1C" } else { "S3A_OL_2_WFR" })
                    .trim_end_matches(".SAFE")
                    .trim_end_matches(".SEN3")
                    .to_owned();
                let id_uuid = entry["Id"].as_str().unwrap_or("");
                let dt = entry["ContentDate"]["Start"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                let geom = if entry["GeoFootprint"].is_object() {
                    entry["GeoFootprint"].clone()
                } else {
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]
                    })
                };
                let prod_type = if is_s2 {
                    "S2MSI1C".to_owned()
                } else if name.contains("SL_2_LST") {
                    "SL_2_LST".to_owned()
                } else if name.contains("OL_2_WFR") {
                    "OL_2_WFR".to_owned()
                } else if name.contains("OL_1_EFR") {
                    "OL_1_EFR".to_owned()
                } else {
                    "S3_STANDARD".to_owned()
                };
                let platform = if name.starts_with("S2B") {
                    "Sentinel-2B"
                } else if name.starts_with("S2A") {
                    "Sentinel-2A"
                } else if name.starts_with("S3B") {
                    "Sentinel-3B"
                } else {
                    "Sentinel-3A"
                };
                let href = if !id_uuid.is_empty() {
                    format!("https://catalogue.dataspace.copernicus.eu/odata/v1/Products({id_uuid})/$value")
                } else {
                    "https://catalogue.dataspace.copernicus.eu/odata/v1/Products".to_owned()
                };
                let item_bbox = geojson_bbox_bounds(&geom).unwrap_or([w, s, e, n]);
                items.push(json!({
                    "id": name,
                    "title": name,
                    "datetime": dt,
                    "platform": platform,
                    "product_type": prod_type,
                    "bbox": item_bbox,
                    "geometry": geom,
                    "href": href,
                }));
            }
            filter_open_geodata_items_by_aoi(&mut items, req.aoi_geojson.as_ref());
            Ok(json!({
                "ok": true,
                "dataset": dataset,
                "provider": "Copernicus Data Space Ecosystem (CDSE)",
                "collection": coll_name,
                "start_date": win_start,
                "end_date": win_end,
                "count": items.len(),
                "items": items,
            }))
        }
        "gee_account_status" => {
            if is_isolated_empty_keychain_mode() {
                return Ok(json!({
                    "ok": true,
                    "configured": false,
                    "gee_ready": false,
                    "has_secret": false,
                    "project_id": "",
                    "auth_mode": "service_account",
                    "client_email": "",
                    "updated_at": "",
                }));
            }
            let cred_path = app_data_dir.join("gee_credentials.json");
            let mut configured = false;
            let mut project_id = String::new();
            let mut auth_mode = "service_account".to_owned();
            let mut client_email = String::new();
            let mut updated_at = String::new();

            if cred_path.is_file() {
                let raw = fs::read_to_string(&cred_path).unwrap_or_default();
                if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                    if let Some(pid) = parsed["project_id"].as_str().filter(|s| !s.trim().is_empty()) {
                        project_id = pid.trim().to_owned();
                        configured = resolve_gee_keyring_secret_for_project(&project_id).is_some();
                    }
                    if let Some(am) = parsed["auth_mode"].as_str().filter(|s| !s.trim().is_empty()) {
                        auth_mode = am.trim().to_owned();
                    }
                    if let Some(em) = parsed["client_email"].as_str().filter(|s| !s.trim().is_empty()) {
                        client_email = em.trim().to_owned();
                    }
                    if let Some(up) = parsed["updated_at"].as_str() {
                        updated_at = up.to_owned();
                    } else if let Some(up_num) = parsed["updated_at"].as_u64() {
                        updated_at = up_num.to_string();
                    }
                }
            }

            // Fallback to OS Keyring or local vault if metadata file is missing or incomplete
            if !configured {
                if let Some((k_proj, k_mode, k_sec, k_email)) = resolve_gee_keyring_bundle() {
                    if !k_proj.is_empty() && !k_sec.trim().is_empty() {
                        project_id = k_proj;
                        configured = true;
                        auth_mode = k_mode;
                        client_email = k_email;
                    }
                }
            }
            if !configured {
                if let Ok(Some(mat)) = crate::gee_cloud_suite::load_saved_gee_credentials(app_data_dir) {
                    configured = true;
                    project_id = mat.project_id;
                    auth_mode = mat.auth_mode;
                    client_email = mat.client_email.unwrap_or_default();
                }
            }

            Ok(json!({
                "ok": true,
                "configured": configured,
                "gee_ready": configured,
                "has_secret": configured,
                "project_id": project_id,
                "auth_mode": auth_mode,
                "client_email": client_email,
                "updated_at": updated_at,
            }))
        }
        "gee_account_reveal" => {
            if is_isolated_empty_keychain_mode() {
                return Err(
                    "GEE_ISOLATED_EMPTY_KEYCHAIN_READONLY: 当前处于空钥匙串隔离测试模式，未加载本机密钥"
                        .to_owned(),
                );
            }
            let cred_path = app_data_dir.join("gee_credentials.json");
            let mut project_id = req.code.as_deref().map(str::trim).unwrap_or("").to_owned();
            if project_id.is_empty() && cred_path.is_file() {
                let raw = fs::read_to_string(&cred_path).unwrap_or_default();
                if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                    if let Some(pid) = parsed["project_id"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                        project_id = pid.to_owned();
                    }
                }
            }
            let mut secret_value = String::new();
            if !project_id.is_empty() {
                if let Some(sec) = resolve_gee_keyring_secret_for_project(&project_id) {
                    secret_value = sec;
                }
            }
            if secret_value.is_empty() {
                if let Some((_, _, sec, _)) = resolve_gee_keyring_bundle() {
                    if !sec.trim().is_empty() {
                        secret_value = sec;
                    }
                }
            }
            if secret_value.is_empty() {
                if let Some((_, _, v_sec, _)) = resolve_gee_local_vault_bundle(
                    app_data_dir,
                    if project_id.is_empty() {
                        None
                    } else {
                        Some(project_id.as_str())
                    },
                ) {
                    if !v_sec.trim().is_empty() {
                        secret_value = v_sec;
                    }
                }
            }
            if secret_value.trim().is_empty() {
                return Err("GEE_CREDENTIAL_NOT_SAVED: 当前尚未保存可显示的 GEE 密钥或 Access Token".to_owned());
            }
            Ok(json!({
                "ok": true,
                "value": secret_value,
            }))
        }
        "gee_account_save" => {
            if is_isolated_empty_keychain_mode() {
                return Err(
                    "GEE_ISOLATED_EMPTY_KEYCHAIN_READONLY: 当前处于空钥匙串隔离测试模式，禁止修改或覆盖本机凭据"
                        .to_owned(),
                );
            }
            let mut project_id = req.code.as_deref().map(str::trim).unwrap_or("").to_owned();
            let mut secret_payload = req.sql.as_deref().map(str::trim).unwrap_or("").to_owned();
            let mut auth_mode = req.keyword.as_deref().map(str::trim).unwrap_or("service_account").to_owned();

            // Support dual contract: parse JSON if keyword contains a JSON string
            if let Some(raw_kw) = req.keyword.as_deref().map(str::trim) {
                if raw_kw.starts_with('{') && raw_kw.ends_with('}') {
                    if let Ok(parsed_kw) = serde_json::from_str::<Value>(raw_kw) {
                        if let Some(pid) = parsed_kw.get("project_id").and_then(|v| v.as_str()).map(str::trim) {
                            if !pid.is_empty() {
                                project_id = pid.to_owned();
                            }
                        }
                        if let Some(am) = parsed_kw.get("auth_mode").and_then(|v| v.as_str()).map(str::trim) {
                            if !am.is_empty() {
                                auth_mode = am.to_owned();
                            }
                        }
                        if let Some(sec) = parsed_kw.get("secret").and_then(|v| v.as_str()).map(str::trim) {
                            if !sec.is_empty() {
                                secret_payload = sec.to_owned();
                            }
                        }
                    }
                }
            }

            // Support `gee_params` fallback fields (`project_id`, `service_account_json` / `secret`, `auth_mode`)
            if let Some(gp) = req.gee_params.as_ref() {
                if project_id.is_empty() {
                    if let Some(pid) = gp
                        .get("project_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        project_id = pid.to_owned();
                    }
                }
                if secret_payload.is_empty() {
                    if let Some(sec) = gp
                        .get("secret")
                        .or_else(|| gp.get("service_account_json"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        secret_payload = sec.to_owned();
                    }
                }
                if req.keyword.is_none() {
                    if let Some(am) = gp
                        .get("auth_mode")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        auth_mode = am.to_owned();
                    } else if !secret_payload.is_empty() && !secret_payload.starts_with('{') {
                        auth_mode = "oauth_token".to_owned();
                    }
                }
            }

            let _ = fs::create_dir_all(app_data_dir);
            let cred_path = app_data_dir.join("gee_credentials.json");
            let mut preserved_api_base: Option<String> = None;
            let mut previous_pid: Option<String> = None;
            if cred_path.is_file() {
                if let Ok(existing_raw) = fs::read_to_string(&cred_path) {
                    if let Ok(existing_doc) = serde_json::from_str::<Value>(&existing_raw) {
                        if let Some(base) = existing_doc
                            .get("api_base_url")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            preserved_api_base = Some(base.to_owned());
                        }
                        if let Some(old_pid) = existing_doc
                            .get("project_id")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            previous_pid = Some(old_pid.to_owned());
                        }
                    }
                }
            }

            // If secret is blank on update, retain existing secret ONLY from OS Keyring (never disk)
            if secret_payload.is_empty() {
                if let Some(sec) = resolve_gee_keyring_secret_for_project(&project_id) {
                    secret_payload = sec;
                } else if let Some(ref old_pid) = previous_pid {
                    if let Some(sec) = resolve_gee_keyring_secret_for_project(old_pid) {
                        secret_payload = sec;
                    }
                }
                if secret_payload.is_empty() {
                    if let Some((_, _, existing_sec, _)) = resolve_gee_keyring_bundle() {
                        if !existing_sec.is_empty() {
                            secret_payload = existing_sec;
                        }
                    }
                }
                if secret_payload.is_empty() {
                    let fb_pid = if !project_id.is_empty() {
                        Some(project_id.as_str())
                    } else {
                        previous_pid.as_deref()
                    };
                    if let Some((_, existing_mode, existing_sec, _)) =
                        resolve_gee_local_vault_bundle(app_data_dir, fb_pid)
                    {
                        if !existing_sec.is_empty() {
                            secret_payload = existing_sec;
                            if req.keyword.is_none() {
                                auth_mode = existing_mode;
                            }
                        }
                    }
                }
            }

            if secret_payload.is_empty() && project_id.is_empty() {
                return Err("GEE_PROJECT_ID_REQUIRED: 请填写 Google Cloud Project ID（如 ee-yourname）".to_owned());
            }
            if secret_payload.is_empty() {
                return Err("GEE_CREDENTIAL_REQUIRED: 请粘贴 Service Account JSON 密钥或 OAuth2 Access Token".to_owned());
            }
            let mut client_email = String::new();
            if auth_mode == "service_account" || secret_payload.starts_with('{') {
                let sa: Value = serde_json::from_str(&secret_payload)
                    .map_err(|e| format!("INVALID_SERVICE_ACCOUNT_JSON: Service Account JSON 格式无效 ({e})"))?;
                if project_id.is_empty() {
                    if let Some(pid) = sa["project_id"]
                        .as_str()
                        .or_else(|| sa["quota_project_id"].as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        project_id = pid.to_owned();
                    }
                }
                client_email = sa["client_email"].as_str().unwrap_or("").to_owned();
                if client_email.is_empty()
                    && sa["private_key"].as_str().is_none()
                    && sa["access_token"].as_str().is_none()
                    && sa["refresh_token"].as_str().is_none()
                {
                    return Err("INVALID_SERVICE_ACCOUNT_JSON: JSON 中缺少 client_email / private_key / refresh_token 字段".to_owned());
                }
            }
            if project_id.is_empty() {
                return Err("GEE_PROJECT_ID_REQUIRED: 请填写 Google Cloud Project ID（如 ee-yourname）".to_owned());
            }

            // Store secret EXCLUSIVELY in native OS Keyring (fail closed if Keyring write fails)
            store_gee_keyring_bundle(&project_id, &auth_mode, &secret_payload, &client_email)?;

            // Write ONLY non-sensitive metadata to gee_credentials.json (NEVER write plaintext `secret`!)
            let mut doc = json!({
                "configured": true,
                "project_id": project_id,
                "auth_mode": auth_mode,
                "client_email": client_email,
                "updated_at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_default(),
            });
            if let Some(base) = preserved_api_base {
                doc["api_base_url"] = json!(base);
            }
            fs::write(&cred_path, serde_json::to_string_pretty(&doc).unwrap_or_default())
                .map_err(|e| format!("WRITE_GEE_CRED_FAILED: {e}"))?;
            crate::gee_cloud_suite::invalidate_gee_runtime_caches();

            Ok(json!({
                "ok": true,
                "configured": true,
                "gee_ready": true,
                "project_id": project_id,
                "auth_mode": auth_mode,
                "client_email": client_email,
            }))
        }
        "gee_account_clear" => {
            if is_isolated_empty_keychain_mode() {
                return Err(
                    "GEE_ISOLATED_EMPTY_KEYCHAIN_READONLY: 当前处于空钥匙串隔离测试模式，禁止清除本机凭据"
                        .to_owned(),
                );
            }
            let cred_path = app_data_dir.join("gee_credentials.json");
            let mut existing_pid: Option<String> = None;
            if cred_path.is_file() {
                if let Ok(raw) = fs::read_to_string(&cred_path) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                        existing_pid = parsed["project_id"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                    }
                }
            }
            let _ = fs::remove_file(&cred_path);
            clear_gee_keyring_bundle_for_project(existing_pid.as_deref());
            scrub_gee_entries_from_local_vault(app_data_dir);
            crate::gee_cloud_suite::invalidate_gee_runtime_caches();
            Ok(json!({
                "ok": true,
                "configured": false,
                "gee_ready": false,
            }))
        }
        "gee_account_verify" => {
            let cred_path = app_data_dir.join("gee_credentials.json");
            let mut project_id = req.code.as_deref().map(str::trim).unwrap_or("").to_owned();
            let mut secret = req.sql.as_deref().map(str::trim).unwrap_or("").to_owned();
            let mut auth_mode = req.keyword.as_deref().map(str::trim).unwrap_or("").to_owned();
            let mut api_base = "https://earthengine.googleapis.com".to_owned();

            if let Some(gp) = req.gee_params.as_ref() {
                if project_id.is_empty() {
                    if let Some(pid) = gp
                        .get("project_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        project_id = pid.to_owned();
                    }
                }
                if secret.is_empty() {
                    if let Some(sec) = gp
                        .get("secret")
                        .or_else(|| gp.get("service_account_json"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        secret = sec.to_owned();
                    }
                }
                if auth_mode.is_empty() {
                    if let Some(am) = gp
                        .get("auth_mode")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        auth_mode = am.to_owned();
                    } else if !secret.is_empty() && !secret.starts_with('{') {
                        auth_mode = "oauth_token".to_owned();
                    }
                }
            }

            if cred_path.is_file() {
                let raw = fs::read_to_string(&cred_path).unwrap_or_default();
                if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                    if project_id.is_empty() {
                        if let Some(pid) = parsed["project_id"].as_str().map(str::trim) {
                            project_id = pid.to_owned();
                        }
                    }
                    if auth_mode.is_empty() {
                        auth_mode = parsed["auth_mode"].as_str().unwrap_or("service_account").to_owned();
                    }
                    if let Some(base) = parsed["api_base_url"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                        api_base = base.trim_end_matches('/').to_owned();
                    }
                }
            }

            if project_id.is_empty() && secret.trim_start().starts_with('{') {
                if let Ok(sa) = serde_json::from_str::<Value>(&secret) {
                    if let Some(pid) = sa["project_id"]
                        .as_str()
                        .or_else(|| sa["quota_project_id"].as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        project_id = pid.to_owned();
                    }
                }
            }

            if secret.is_empty() && !project_id.is_empty() {
                if let Some(k_sec) = resolve_gee_keyring_secret_for_project(&project_id) {
                    secret = k_sec;
                }
            }
            if project_id.is_empty() || secret.is_empty() {
                if let Some((k_proj, k_mode, k_sec, _)) = resolve_gee_keyring_bundle() {
                    if project_id.is_empty() {
                        project_id = k_proj;
                    }
                    if secret.is_empty() {
                        secret = k_sec;
                    }
                    if auth_mode.is_empty() {
                        auth_mode = k_mode;
                    }
                }
            }
            if project_id.is_empty() || secret.is_empty() {
                let fb_pid = if project_id.is_empty() {
                    None
                } else {
                    Some(project_id.as_str())
                };
                if let Some((v_proj, v_mode, v_sec, _)) =
                    resolve_gee_local_vault_bundle(app_data_dir, fb_pid)
                {
                    if project_id.is_empty() {
                        project_id = v_proj;
                    }
                    if secret.is_empty() {
                        secret = v_sec;
                    }
                    if auth_mode.is_empty() {
                        auth_mode = v_mode;
                    }
                }
            }
            if auth_mode.is_empty() {
                auth_mode = "service_account".to_owned();
            }

            if project_id.is_empty() || secret.is_empty() {
                return Err(
                    "GEE_NOT_CONFIGURED: GEE Project ID 或凭据为空，请先保存有效密钥后再测试连接"
                        .to_owned(),
                );
            }

            let mut client_email = String::new();
            let mut token_uri = "https://oauth2.googleapis.com/token".to_owned();
            if auth_mode.eq_ignore_ascii_case("service_account")
                && !secret.trim_start().starts_with('{')
            {
                return Err(
                    "INVALID_SA_JSON: service_account 模式需要提供合法的 Google Service Account JSON 对象"
                        .to_owned(),
                );
            }
            if secret.trim_start().starts_with('{') {
                let sa: Value = serde_json::from_str(&secret)
                    .map_err(|e| format!("INVALID_SA_JSON: Service Account JSON 解析失败 ({e})"))?;
                client_email = sa["client_email"].as_str().unwrap_or("").trim().to_owned();
                if let Some(tu) = sa["token_uri"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    token_uri = tu.to_owned();
                }
                if client_email.is_empty()
                    && sa["private_key"].as_str().is_none()
                    && sa["access_token"].as_str().is_none()
                    && sa["refresh_token"].as_str().is_none()
                {
                    return Err(
                        "INVALID_SA_JSON: Service Account / OAuth JSON 中缺少 client_email / private_key / refresh_token"
                            .to_owned(),
                    );
                }
            }

            let client = build_http_client(app_data_dir)?;
            // 1. Strictly exchange / validate Bearer token (NEVER swallow RSA / OAuth2 failures!)
            let bearer_token = resolve_gee_bearer_token(app_data_dir, &client, &secret)?;

            // 2. Strictly probe an authenticated Earth Engine REST endpoint (NEVER fall back to unauthenticated $discovery/rest!)
            let compute_val_url = format!("{api_base}/v1/projects/{project_id}/value:compute");
            let compute_payload = json!({
                "expression": gee_expr_constant_one()
            });
            let resp = client
                .post(&compute_val_url)
                .bearer_auth(&bearer_token)
                .json(&compute_payload)
                .send()
                .map_err(|_err| {
                    "GEE_NETWORK_UNREACHABLE: 无法连接 Google Earth Engine 认证接口，请在「网络与代理」中确认代理配置".to_owned()
                })?;

            let status = resp.status();
            if !status.is_success() {
                let status_code = status.as_u16();
                let body_snippet = resp.text().unwrap_or_default();
                let detail = extract_gee_http_error_summary(&body_snippet);
                return Err(format!(
                    "GEE_VERIFY_HTTP_{status_code}: Google Earth Engine 认证接口拒绝访问 (HTTP {status_code}: {detail})"
                ));
            }

            let msg = format!("GEE 官方认证接口校验通过 (Project: {project_id})");
            Ok(json!({
                "ok": true,
                "verified": true,
                "auth_mode": auth_mode,
                "project_id": project_id,
                "client_email": client_email,
                "token_uri": token_uri,
                "token_preview": if bearer_token.len() > 10 {
                    format!("{}...", &bearer_token[..8])
                } else {
                    "***".to_owned()
                },
                "message": msg,
                "message_zh": msg,
            }))
        }
        "gee_catalog" => {
            // Supports both Route 1 (Public Cloud Direct for GEE canonical datasets) and Route 2 (Official GEE Account REST API)
            let raw_bbox = req
                .bbox
                .ok_or_else(|| "MISSING_BBOX: gee_catalog 需要 bbox=[west, south, east, north]".to_owned())?;
            let bbox = [
                raw_bbox[0].min(raw_bbox[2]),
                raw_bbox[1].min(raw_bbox[3]),
                raw_bbox[0].max(raw_bbox[2]),
                raw_bbox[1].max(raw_bbox[3]),
            ];

            // Parse pipe-delimited keyword `${startDate}..${endDate}|${geeAssetId}|${band}|download`
            let mut kw_start_date: Option<String> = None;
            let mut kw_end_date: Option<String> = None;
            let mut kw_asset_id: Option<String> = None;
            let mut kw_band: Option<String> = None;
            let mut kw_download = false;
            if let Some(raw_kw) = req.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                for token in raw_kw.split('|').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some((s, e)) = token.split_once("..") {
                        let s_trim = s.trim();
                        let e_trim = e.trim();
                        if !s_trim.is_empty() {
                            kw_start_date = Some(s_trim.to_owned());
                        }
                        if !e_trim.is_empty() {
                            kw_end_date = Some(e_trim.to_owned());
                        }
                    } else if token.eq_ignore_ascii_case("download") {
                        kw_download = true;
                    } else if token.eq_ignore_ascii_case("search") || token.eq_ignore_ascii_case("preview") {
                        // ignore action verb
                    } else if token.contains('/') {
                        if kw_asset_id.is_none() {
                            kw_asset_id = Some(token.to_owned());
                        }
                    } else if kw_band.is_none() {
                        kw_band = Some(token.to_owned());
                    }
                }
            }

            let gee_asset_id = req
                .code
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or(kw_asset_id.as_deref())
                .or(kw_band.as_deref())
                .unwrap_or("ESA/WorldCover/v200");

            let force_gee_official_rest = req
                .sql
                .as_deref()
                .map(str::trim)
                .is_some_and(|s| s.eq_ignore_ascii_case("gee_official_rest"));

            if !force_gee_official_rest {
                if gee_asset_id == "projects/climate-and-sustainability/assets/gem-15_v1" {
                    return execute_gem15_public_gcs_query(
                        app_data_dir,
                        req,
                        bbox,
                        kw_download,
                        kw_band.as_deref(),
                    );
                }
                if let Some((delegated_ds, collection, default_band)) = map_canonical_gee_asset(gee_asset_id) {
                    let search_only = has_search_only_token(req.keyword.as_deref())
                        && !kw_download
                        && req.tool_name.as_deref() != Some("download");
                    let mut delegated_req = req.clone();
                    delegated_req.dataset = delegated_ds.to_owned();
                    delegated_req.code = Some(collection.to_owned());
                    delegated_req.bbox = Some(bbox);
                    if delegated_req.start_date.as_deref().unwrap_or("").is_empty() {
                        delegated_req.start_date = kw_start_date.clone();
                    }
                    if delegated_req.end_date.as_deref().unwrap_or("").is_empty() {
                        delegated_req.end_date = kw_end_date.clone();
                    }
                    if kw_download && delegated_req.tool_name.is_none() {
                        delegated_req.tool_name = Some("download".to_owned());
                    }
                    let explicit_band = kw_band
                        .as_deref()
                        .filter(|t| *t != gee_asset_id && !t.contains('/'))
                        .unwrap_or(default_band);
                    delegated_req.keyword = Some(if search_only {
                        format!("{explicit_band}|search")
                    } else if kw_download {
                        format!("{explicit_band}|download")
                    } else {
                        explicit_band.to_owned()
                    });

                    let mut res = execute_open_geodata_query(app_data_dir, &delegated_req)?;
                    res["gee_asset_id"] = json!(gee_asset_id);
                    res["route_mode"] = json!("cloud_native_direct");
                    if delegated_ds != "open_meteo_era5" {
                        if let Ok(client) = build_http_client(app_data_dir) {
                            if let Ok((tile_tpl, _)) = resolve_mpc_tilejson_template(
                                app_data_dir,
                                &client,
                                collection,
                                explicit_band,
                                Some(bbox),
                            ) {
                                res["tile_url_template"] = json!(tile_tpl);
                                res["url_format"] = json!(tile_tpl);
                            }
                        }
                    }
                    return Ok(res);
                }
            }

            // Route 2: Custom GEE Asset ID via user's configured GEE account (Keyring-only secret)
            let cred_path = app_data_dir.join("gee_credentials.json");
            let mut project_id = String::new();
            let mut api_base = "https://earthengine.googleapis.com".to_owned();

            if cred_path.is_file() {
                let raw = fs::read_to_string(&cred_path).unwrap_or_default();
                let parsed: Value = serde_json::from_str(&raw).unwrap_or(json!({}));
                project_id = parsed["project_id"].as_str().unwrap_or("").trim().to_owned();
                if let Some(base) = parsed["api_base_url"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    api_base = base.trim_end_matches('/').to_owned();
                }
            }
            let mut secret = if !project_id.is_empty() {
                resolve_gee_keyring_secret_for_project(&project_id).unwrap_or_default()
            } else {
                String::new()
            };
            if project_id.is_empty() || secret.is_empty() {
                if let Some((k_proj, _, k_sec, _)) = resolve_gee_keyring_bundle() {
                    if project_id.is_empty() {
                        project_id = k_proj;
                    }
                    if secret.is_empty() {
                        secret = k_sec;
                    }
                }
            }
            if project_id.is_empty() || secret.is_empty() {
                let fb_pid = if project_id.is_empty() {
                    None
                } else {
                    Some(project_id.as_str())
                };
                if let Some((v_proj, _, v_sec, _)) =
                    resolve_gee_local_vault_bundle(app_data_dir, fb_pid)
                {
                    if project_id.is_empty() {
                        project_id = v_proj;
                    }
                    if secret.is_empty() {
                        secret = v_sec;
                    }
                }
            }

            if project_id.is_empty() || secret.is_empty() {
                return Err(format!(
                    "GEE_ACCOUNT_REQUIRED: 自定义 GEE 资产 ({gee_asset_id}) 需要先在「设置 -> 数据服务账号」中配置 Google Earth Engine 账号（Project ID 与系统钥匙串凭据）"
                ));
            }

            let client = build_http_client(app_data_dir)?;
            let safe_name = sanitize_gee_route2_safe_name(gee_asset_id);
            let min_lon = bbox[0].min(bbox[2]);
            let min_lat = bbox[1].min(bbox[3]);
            let max_lon = bbox[0].max(bbox[2]);
            let max_lat = bbox[1].max(bbox[3]);
            let eff_start_date = kw_start_date
                .as_deref()
                .or(req.start_date.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let eff_end_date = kw_end_date
                .as_deref()
                .or(req.end_date.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let item_dt = eff_start_date
                .map(|s| normalize_gee_date_bound(s, false))
                .unwrap_or_else(|| "2026-01-01T00:00:00Z".to_owned());
            let explicit_bands: Option<Vec<String>> = kw_band
                .as_deref()
                .filter(|t| *t != gee_asset_id && !t.contains('/'))
                .map(|t| {
                    t.split(',')
                        .map(str::trim)
                        .filter(|b| !b.is_empty())
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty());
            let (scoped_item_id, scoped_tif_name) = compute_gee_route2_scope_id(
                gee_asset_id,
                &safe_name,
                min_lon,
                min_lat,
                max_lon,
                max_lat,
                eff_start_date,
                eff_end_date,
                explicit_bands.as_deref(),
            );

            let route2_search_only = has_search_only_token(req.keyword.as_deref())
                && !kw_download
                && req.tool_name.as_deref() != Some("download");

            if route2_search_only {
                let use_avail = req
                    .gee_params
                    .as_ref()
                    .and_then(|p| p.get("use_availability_check"))
                    .and_then(Value::as_bool)
                    == Some(true);
                if use_avail {
                    let req_max_cloud = req
                        .gee_params
                        .as_ref()
                        .and_then(|p| p.get("max_cloud"))
                        .and_then(Value::as_f64);
                    let req_sar_pol = req
                        .gee_params
                        .as_ref()
                        .and_then(|p| p.get("sar_polarization"))
                        .and_then(Value::as_str);
                    let req_orbit_dir = req
                        .gee_params
                        .as_ref()
                        .and_then(|p| p.get("orbit_direction"))
                        .and_then(Value::as_str);
                    let avail_json = crate::gee_cloud_suite::check_gee_availability_with_filters(
                        app_data_dir,
                        gee_asset_id,
                        bbox,
                        eff_start_date,
                        eff_end_date,
                        req_max_cloud,
                        None,
                        req_sar_pol,
                        req_orbit_dir,
                    )?;
                    return Ok(build_gee_catalog_search_response(
                        gee_asset_id,
                        bbox,
                        &scoped_item_id,
                        &scoped_tif_name,
                        explicit_bands.as_deref(),
                        req.output_path.as_deref(),
                        &item_dt,
                        &avail_json,
                    ));
                }

                let bearer_token = resolve_gee_bearer_token(app_data_dir, &client, &secret)?;
                let image_expr = gee_expr_image_load(gee_asset_id);
                let (expr_mode, discovered_bands) = match gee_fetch_band_names_for_expr(
                    &client,
                    &api_base,
                    &project_id,
                    &bearer_token,
                    &image_expr,
                ) {
                    Ok(bands) => ("Image.load", bands),
                    Err((400, _)) => {
                        let coll_expr = gee_expr_collection_mosaic_filtered(
                            gee_asset_id,
                            eff_start_date,
                            eff_end_date,
                        );
                        match gee_fetch_band_names_for_expr(
                            &client,
                            &api_base,
                            &project_id,
                            &bearer_token,
                            &coll_expr,
                        ) {
                            Ok(bands) => ("ImageCollection.mosaic", bands),
                            Err((0, net_err)) => {
                                return Err(format!(
                                    "GEE_NETWORK_UNREACHABLE: 无法连接 Google Earth Engine 资产检索接口 ({net_err})"
                                ));
                            }
                            Err((code, detail)) => {
                                return Err(format!(
                                    "GEE_ASSET_SEARCH_FAILED: HTTP {code} 无法检索 GEE 资产 {gee_asset_id} ({detail})"
                                ));
                            }
                        }
                    }
                    Err((0, net_err)) => {
                        return Err(format!(
                            "GEE_NETWORK_UNREACHABLE: 无法连接 Google Earth Engine 资产检索接口 ({net_err})"
                        ));
                    }
                    Err((code, detail)) => {
                        return Err(format!(
                            "GEE_ASSET_SEARCH_FAILED: HTTP {code} 无法检索 GEE 资产 {gee_asset_id} ({detail})"
                        ));
                    }
                };

                let explicit_existing = req
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|dir_str| PathBuf::from(dir_str).join(&scoped_tif_name))
                    .and_then(|p| existing_valid_science_file_bytes(&p).map(|sz| (p, sz)));

                let already_in_explicit_dir = explicit_existing.is_some();
                let local_path_val = explicit_existing
                    .as_ref()
                    .map(|(p, _)| Value::String(p.to_string_lossy().to_string()))
                    .unwrap_or(Value::Null);

                if discovered_bands.is_empty() {
                    return Ok(json!({
                        "ok": true,
                        "dataset": "gee_catalog",
                        "gee_asset_id": gee_asset_id,
                        "route_mode": "gee_official_rest",
                        "expression_mode": expr_mode,
                        "search_only": true,
                        "count": 0,
                        "total_matching_scenes": 0,
                        "already_exists": already_in_explicit_dir,
                        "target_item_id": scoped_item_id,
                        "bands": discovered_bands,
                        "mosaic_bbox": [min_lon, min_lat, max_lon, max_lat],
                        "items": [],
                        "empty_reason_zh": "当前研究区与筛选条件下暂无可用 GEE 影像波段（0 景）",
                    }));
                }

                return Ok(json!({
                    "ok": true,
                    "dataset": "gee_catalog",
                    "gee_asset_id": gee_asset_id,
                    "route_mode": "gee_official_rest",
                    "expression_mode": expr_mode,
                    "search_only": true,
                    "count": 1,
                    "already_exists": already_in_explicit_dir,
                    "target_item_id": scoped_item_id,
                    "bands": discovered_bands,
                    "mosaic_bbox": [min_lon, min_lat, max_lon, max_lat],
                    "items": [{
                        "id": scoped_item_id,
                        "title": gee_asset_id,
                        "collection": gee_asset_id,
                        "datetime": item_dt,
                        "bbox": [min_lon, min_lat, max_lon, max_lat],
                        "bands": discovered_bands,
                        "already_downloaded": already_in_explicit_dir,
                        "local_file_path": local_path_val,
                        "geometry": {
                            "type": "Polygon",
                            "coordinates": [[
                                [min_lon, min_lat],
                                [max_lon, min_lat],
                                [max_lon, max_lat],
                                [min_lon, max_lat],
                                [min_lon, min_lat]
                            ]]
                        }
                    }],
                }));
            }

            let out_dir = req
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join("gee_exports"));
            fs::create_dir_all(&out_dir).map_err(|e| format!("CREATE_DIR_FAILED: {e}"))?;

            let out_file = out_dir.join(&scoped_tif_name);
            if let Some(existing_bytes) = existing_valid_science_file_bytes(&out_file) {
                let out_file_str = out_file.to_string_lossy().to_string();
                let mut out_json = json!({
                    "ok": true,
                    "dataset": "gee_catalog",
                    "gee_asset_id": gee_asset_id,
                    "route_mode": "gee_official_rest",
                    "count": 1,
                    "already_exists": true,
                    "target_item_id": scoped_item_id,
                    "downloaded_file": out_file_str,
                    "downloaded_path": out_file_str,
                    "downloaded_bytes": existing_bytes,
                    "size_bytes": existing_bytes,
                    "mosaic_bbox": [min_lon, min_lat, max_lon, max_lat],
                    "items": [{
                        "id": scoped_item_id,
                        "datetime": item_dt,
                        "href": out_file_str,
                        "bbox": [min_lon, min_lat, max_lon, max_lat],
                        "already_downloaded": true,
                        "local_file_path": out_file_str,
                        "geometry": {
                            "type": "Polygon",
                            "coordinates": [[
                                [min_lon, min_lat],
                                [max_lon, min_lat],
                                [max_lon, max_lat],
                                [min_lon, max_lat],
                                [min_lon, min_lat]
                            ]]
                        }
                    }],
                });
                attach_raster_preview_fields(&mut out_json, &out_file);
                return Ok(out_json);
            }
            let compute_url = format!("{api_base}/v1/projects/{project_id}/image:computePixels");
            let lon_span = (max_lon - min_lon).max(1e-5);
            let lat_span = (max_lat - min_lat).max(1e-5);
            let mid_lat_rad = ((min_lat + max_lat) * 0.5).to_radians();
            let ground_w = (lon_span * mid_lat_rad.cos().abs().max(0.1)).max(1e-5);
            let ground_h = lat_span;
            let (grid_w, grid_h): (u32, u32) = if ground_w >= ground_h {
                let w = 1024u32;
                let h = ((1024.0 * ground_h / ground_w).round() as u32).clamp(64, 1024);
                (w, h)
            } else {
                let h = 1024u32;
                let w = ((1024.0 * ground_w / ground_h).round() as u32).clamp(64, 1024);
                (w, h)
            };
            let scale_x = (lon_span / (grid_w as f64)).max(1e-6);
            let scale_y = -(lat_span / (grid_h as f64)).max(1e-6);
            let grid_spec = json!({
                "dimensions": {
                    "width": grid_w,
                    "height": grid_h
                },
                "affineTransform": {
                    "scaleX": scale_x,
                    "shearX": 0.0,
                    "translateX": min_lon,
                    "shearY": 0.0,
                    "scaleY": scale_y,
                    "translateY": max_lat
                },
                "crsCode": "EPSG:4326"
            });

            let image_expr = gee_expr_image_load(gee_asset_id);
            let image_load_payload =
                build_gee_compute_pixels_payload(&image_expr, &grid_spec, explicit_bands.as_deref());

            let bearer_token = resolve_gee_bearer_token(app_data_dir, &client, &secret)?;
            let post_compute_pixels = |payload: &Value, err_tag: &str| -> Result<Result<Vec<u8>, (u16, String)>, String> {
                let r = client
                    .post(&compute_url)
                    .bearer_auth(&bearer_token)
                    .json(payload)
                    .send()
                    .map_err(|e| format!("{err_tag}: 无法连接 GEE 像素导出接口 ({e})"))?;
                if r.status().is_success() {
                    let b = r
                        .bytes()
                        .map_err(|e| format!("GEE_COMPUTE_PIXELS_READ_FAILED: {e}"))?;
                    Ok(Ok(b.to_vec()))
                } else {
                    let code = r.status().as_u16();
                    let txt = r.text().unwrap_or_default();
                    Ok(Err((code, txt)))
                }
            };

            let mut used_collection_mosaic = false;
            let mut pixel_outcome =
                post_compute_pixels(&image_load_payload, "GEE_COMPUTE_PIXELS_NETWORK_ERROR")?;

            if let Err((400, ref first_err_body)) = pixel_outcome {
                let first_err_lower = first_err_body.to_ascii_lowercase();
                let is_band_type_mismatch_on_image = !first_err_lower.contains("collection")
                    && (first_err_lower.contains("compatible")
                        || first_err_lower.contains("pixel type")
                        || first_err_lower.contains("data type")
                        || first_err_lower.contains("band"));

                if is_band_type_mismatch_on_image && explicit_bands.is_none() {
                    if let Ok(img_bands) = gee_fetch_band_names_for_expr(
                        &client,
                        &api_base,
                        &project_id,
                        &bearer_token,
                        &image_expr,
                    ) {
                        if let Some(chosen_bands) = select_homogeneous_gee_bands(&img_bands) {
                            let retry_img_payload = build_gee_compute_pixels_payload(
                                &image_expr,
                                &grid_spec,
                                Some(&chosen_bands),
                            );
                            pixel_outcome = post_compute_pixels(
                                &retry_img_payload,
                                "GEE_COMPUTE_PIXELS_NETWORK_ERROR",
                            )?;
                        }
                    }
                }

                if pixel_outcome.is_err() {
                    // Retry automatically with ImageCollection.load -> (optional Collection.filter DateRange) -> ImageCollection.mosaic
                    let coll_expr = gee_expr_collection_mosaic_filtered(
                        gee_asset_id,
                        eff_start_date,
                        eff_end_date,
                    );
                    let collection_mosaic_payload = build_gee_compute_pixels_payload(
                        &coll_expr,
                        &grid_spec,
                        explicit_bands.as_deref(),
                    );
                    used_collection_mosaic = true;
                    pixel_outcome = post_compute_pixels(
                        &collection_mosaic_payload,
                        "GEE_COMPUTE_PIXELS_COLLECTION_RETRY_FAILED",
                    )?;

                    if let Err((400, _)) = pixel_outcome {
                        if explicit_bands.is_none() {
                            if let Ok(coll_bands) = gee_fetch_band_names_for_expr(
                                &client,
                                &api_base,
                                &project_id,
                                &bearer_token,
                                &coll_expr,
                            ) {
                                if let Some(chosen_bands) = select_homogeneous_gee_bands(&coll_bands) {
                                    let homog_payload = build_gee_compute_pixels_payload(
                                        &coll_expr,
                                        &grid_spec,
                                        Some(&chosen_bands),
                                    );
                                    pixel_outcome = post_compute_pixels(
                                        &homog_payload,
                                        "GEE_COMPUTE_PIXELS_COLLECTION_RETRY_FAILED",
                                    )?;
                                }
                            }
                        }
                    }
                }
            }

            let bytes = match pixel_outcome {
                Ok(b) => b,
                Err((code, err_txt)) => {
                    let detail = extract_gee_http_error_summary(&err_txt);
                    return Err(format!(
                        "GEE_COMPUTE_PIXELS_FAILED: HTTP {code} 无法通过 GEE 官方 REST API 导出 {gee_asset_id} ({detail})"
                    ));
                }
            };
            if bytes.len() < 4 || !(bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")) {
                let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]);
                return Err(format!(
                    "GEE_INVALID_TIFF_HEADER: computePixels 响应不含标准 GeoTIFF 幻数 (4949 2a00 / 4d4d 002a)，返回内容: {preview}"
                ));
            }

            fs::write(&out_file, &bytes)
                .map_err(|e| format!("GEE_WRITE_TIFF_FAILED: {e}"))?;
            let out_file_str = out_file.to_string_lossy().to_string();
            let magic = if bytes.starts_with(b"II*\0") { "49492a00" } else { "4d4d002a" };
            let mut out_json = json!({
                "ok": true,
                "dataset": "gee_catalog",
                "gee_asset_id": gee_asset_id,
                "route_mode": "gee_official_rest",
                "expression_mode": if used_collection_mosaic { "ImageCollection.mosaic" } else { "Image.load" },
                "grid_width": grid_w,
                "grid_height": grid_h,
                "count": 1,
                "already_exists": false,
                "target_item_id": scoped_item_id,
                "downloaded_file": out_file_str,
                "downloaded_path": out_file_str,
                "downloaded_bytes": bytes.len(),
                "size_bytes": bytes.len(),
                "magic_bytes": magic,
                "mosaic_bbox": [min_lon, min_lat, max_lon, max_lat],
                "items": [{
                    "id": scoped_item_id,
                    "datetime": item_dt,
                    "href": out_file_str,
                    "bbox": [min_lon, min_lat, max_lon, max_lat],
                    "already_downloaded": true,
                    "local_file_path": out_file_str,
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[
                            [min_lon, min_lat],
                            [max_lon, min_lat],
                            [max_lon, max_lat],
                            [min_lon, max_lat],
                            [min_lon, min_lat]
                        ]]
                    }
                }],
            });
            attach_raster_preview_fields(&mut out_json, &out_file);
            Ok(out_json)
        }
        "gee_map_tiles" | "gee_tiles" | "gee_tile_fetcher" => {
            let gee_asset_id = req
                .code
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or(req.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty() && s.contains('/')))
                .unwrap_or("COPERNICUS/S2_SR_HARMONIZED");

            let cred_path = app_data_dir.join("gee_credentials.json");
            let mut project_id = String::new();
            let mut api_base = "https://earthengine.googleapis.com".to_owned();

            if cred_path.is_file() {
                if let Ok(raw) = fs::read_to_string(&cred_path) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                        project_id = parsed["project_id"].as_str().unwrap_or("").trim().to_owned();
                        if let Some(base) = parsed["api_base_url"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                            api_base = base.trim_end_matches('/').to_owned();
                        }
                    }
                }
            }
            let mut secret = if !project_id.is_empty() {
                resolve_gee_keyring_secret_for_project(&project_id).unwrap_or_default()
            } else {
                String::new()
            };
            if project_id.is_empty() || secret.is_empty() {
                if let Some((k_proj, _, k_sec, _)) = resolve_gee_keyring_bundle() {
                    if project_id.is_empty() {
                        project_id = k_proj;
                    }
                    if secret.is_empty() {
                        secret = k_sec;
                    }
                }
            }
            if project_id.is_empty() || secret.is_empty() {
                let fb_pid = if project_id.is_empty() {
                    None
                } else {
                    Some(project_id.as_str())
                };
                if let Some((v_proj, _, v_sec, _)) =
                    resolve_gee_local_vault_bundle(app_data_dir, fb_pid)
                {
                    if project_id.is_empty() {
                        project_id = v_proj;
                    }
                    if secret.is_empty() {
                        secret = v_sec;
                    }
                }
            }

            // Parse visualization options from sql, tool_args, or keyword
            let mut vis_bands = vec!["B4".to_string(), "B3".to_string(), "B2".to_string()];
            let mut explicit_vis_bands = false;
            let mut min_val = 0.0_f64;
            let mut max_val = 3000.0_f64;
            let vis_spec = req.sql.as_deref()
                .or(req.keyword.as_deref())
                .or_else(|| req.tool_args.as_ref().and_then(|a| a.first()).map(|s| s.as_str()));
            if let Some(spec) = vis_spec {
                if spec.starts_with('{') {
                    if let Ok(v) = serde_json::from_str::<Value>(spec) {
                        if let Some(arr) = v["bands"].as_array() {
                            let parsed_bands: Vec<String> = arr.iter().filter_map(|b| b.as_str().map(str::to_owned)).collect();
                            if !parsed_bands.is_empty() {
                                vis_bands = parsed_bands;
                                explicit_vis_bands = true;
                            }
                        }
                        if let Some(mn) = v["min"].as_f64() {
                            min_val = mn;
                        }
                        if let Some(mx) = v["max"].as_f64() {
                            max_val = mx;
                        }
                    }
                } else if let Some((b_part, r_part)) = spec.split_once('|') {
                    let bands_split: Vec<String> = b_part
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty() && !s.contains('/'))
                        .collect();
                    if !bands_split.is_empty() {
                        vis_bands = bands_split;
                        explicit_vis_bands = true;
                    }
                    if let Some((mn_s, mx_s)) = r_part.split_once("..") {
                        if let Ok(mn) = mn_s.trim().parse::<f64>() { min_val = mn; }
                        if let Ok(mx) = mx_s.trim().parse::<f64>() { max_val = mx; }
                    }
                }
            }

            let client = build_http_client(app_data_dir)?;
            let mut official_auth_err: Option<String> = None;

            // 1. Attempt official authenticated GEE Maps REST API when credentials are present
            if !project_id.is_empty() && !secret.is_empty() {
                match resolve_gee_bearer_token(app_data_dir, &client, &secret) {
                    Ok(bearer_token) => {
                        let maps_url = format!("{api_base}/v1/projects/{project_id}/maps");
                        let initial_band_ids: Option<&[String]> =
                            if explicit_vis_bands || gee_asset_id.contains("S2") {
                                Some(vis_bands.as_slice())
                            } else {
                                None
                            };
                        let post_map = |payload: &Value| -> Result<Result<Value, (u16, String)>, String> {
                            let r = client
                                .post(&maps_url)
                                .bearer_auth(&bearer_token)
                                .json(payload)
                                .send()
                                .map_err(|e| format!("GEE_MAPS_NETWORK_ERROR: {e}"))?;
                            if r.status().is_success() {
                                let v = r
                                    .json::<Value>()
                                    .map_err(|e| format!("GEE_MAPS_PARSE_ERROR: {e}"))?;
                                Ok(Ok(v))
                            } else {
                                let code = r.status().as_u16();
                                let txt = r.text().unwrap_or_default();
                                Ok(Err((code, txt)))
                            }
                        };

                        let img_expr = gee_expr_image_load(gee_asset_id);
                        let img_payload =
                            build_gee_map_payload(&img_expr, initial_band_ids, min_val, max_val);
                        match post_map(&img_payload) {
                            Ok(mut map_outcome) => {
                                if let Err((400, ref first_err_txt)) = map_outcome {
                                    let first_lower = first_err_txt.to_ascii_lowercase();
                                    if first_lower.contains("collection")
                                        || first_lower.contains("not an image")
                                    {
                                        let coll_expr = gee_expr_collection_mosaic_filtered(
                                            gee_asset_id,
                                            req.start_date.as_deref(),
                                            req.end_date.as_deref(),
                                        );
                                        let coll_payload = build_gee_map_payload(
                                            &coll_expr,
                                            initial_band_ids,
                                            min_val,
                                            max_val,
                                        );
                                        if let Ok(next_out) = post_map(&coll_payload) {
                                            map_outcome = next_out;
                                        }
                                        if matches!(map_outcome, Err((400, _))) {
                                            if let Ok(coll_bands) = gee_fetch_band_names_for_expr(
                                                &client,
                                                &api_base,
                                                &project_id,
                                                &bearer_token,
                                                &coll_expr,
                                            ) {
                                                if let Some(chosen) =
                                                    select_homogeneous_gee_bands(&coll_bands)
                                                {
                                                    let fallback_payload = build_gee_map_payload(
                                                        &coll_expr,
                                                        Some(&chosen),
                                                        min_val,
                                                        max_val,
                                                    );
                                                    if let Ok(next_out2) =
                                                        post_map(&fallback_payload)
                                                    {
                                                        map_outcome = next_out2;
                                                    }
                                                }
                                            }
                                        }
                                    } else if let Ok(img_bands) = gee_fetch_band_names_for_expr(
                                        &client,
                                        &api_base,
                                        &project_id,
                                        &bearer_token,
                                        &img_expr,
                                    ) {
                                        if let Some(chosen) = select_homogeneous_gee_bands(&img_bands)
                                        {
                                            let fallback_payload = build_gee_map_payload(
                                                &img_expr,
                                                Some(&chosen),
                                                min_val,
                                                max_val,
                                            );
                                            if let Ok(next_out2) = post_map(&fallback_payload) {
                                                map_outcome = next_out2;
                                            }
                                        }
                                    }
                                }
                                match map_outcome {
                                    Ok(resp_json) => {
                                        let map_id = resp_json["name"]
                                            .as_str()
                                            .map(|n| n.rsplit('/').next().unwrap_or(n).to_owned())
                                            .unwrap_or_default();
                                        let map_token = resp_json["token"]
                                            .as_str()
                                            .filter(|s| !s.is_empty())
                                            .map(str::to_owned);
                                        if !map_id.is_empty() {
                                            let mut url_format = format!(
                                                "{api_base}/v1/projects/{project_id}/maps/{map_id}/tiles/{{z}}/{{x}}/{{y}}"
                                            );
                                            if let Some(ref tok) = map_token {
                                                url_format = format!("{url_format}?token={tok}");
                                            }
                                            return Ok(json!({
                                                "ok": true,
                                                "dataset": "gee_map_tiles",
                                                "route_mode": "gee_official_rest",
                                                "gee_asset_id": gee_asset_id,
                                                "project_id": project_id,
                                                "map_id": map_id,
                                                "token": map_token,
                                                "tile_url_template": url_format,
                                                "url_format": url_format,
                                                "attribution": "Google Earth Engine",
                                                "bands": vis_bands,
                                                "min": min_val,
                                                "max": max_val,
                                            }));
                                        }
                                    }
                                    Err((status_code, err_body)) => {
                                        let detail = extract_gee_http_error_summary(&err_body);
                                        official_auth_err = Some(format!(
                                            "GEE_MAPS_HTTP_{status_code}: Google Earth Engine 瓦片接口返回 HTTP {status_code} ({detail})"
                                        ));
                                    }
                                }
                            }
                            Err(e) => official_auth_err = Some(e),
                        }
                    }
                    Err(e) => official_auth_err = Some(e),
                }
            }

            // 2. For Route 1 public raster datasets, return a REAL Planetary Computer Data API TileJSON template (NEVER fabricate SHA-256 map_id!)
            if let Some((delegated_ds, collection, default_band)) = map_canonical_gee_asset(gee_asset_id) {
                if delegated_ds != "open_meteo_era5" {
                    let (tile_tpl, item_id) =
                        resolve_mpc_tilejson_template(app_data_dir, &client, collection, default_band, req.bbox)?;
                    return Ok(json!({
                        "ok": true,
                        "dataset": "gee_map_tiles",
                        "route_mode": "cloud_native_direct",
                        "gee_asset_id": gee_asset_id,
                        "collection": collection,
                        "stac_item_id": item_id,
                        "map_id": item_id,
                        "token": Value::Null,
                        "tile_url_template": tile_tpl,
                        "url_format": tile_tpl,
                        "attribution": "Microsoft Planetary Computer Data API",
                        "bands": vis_bands,
                        "min": min_val,
                        "max": max_val,
                    }));
                }
                return Err(format!(
                    "GEE_TILES_UNSUPPORTED_FOR_TABULAR: {gee_asset_id} 为气象时间序列表格数据，不支持无认证栅格瓦片渲染"
                ));
            }

            // 3. Unauthenticated or failed Route 2 custom asset MUST fail honestly (zero fake SHA-256 map_id!)
            if let Some(err) = official_auth_err {
                Err(format!(
                    "GEE_MAP_TILES_FAILED: 无法为自定义资产 {gee_asset_id} 创建真实 GEE 瓦片会话 ({err})"
                ))
            } else {
                Err(format!(
                    "GEE_ACCOUNT_REQUIRED: 自定义 GEE 资产 ({gee_asset_id}) 渲染瓦片需先在「设置 -> 数据服务账号」配置有效的 Google Earth Engine 账号"
                ))
            }
        }
        "bhoonidhi_sband_products" => {
            let bbox = req
                .bbox
                .unwrap_or([77.0, 28.0, 77.5, 28.5]);
            let kw_raw = req.keyword.as_deref().unwrap_or("").trim();
            let req_level = req
                .code
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_ascii_uppercase)
                .or_else(|| {
                    kw_raw
                        .split('|')
                        .map(str::trim)
                        .find(|tok| {
                            matches!(
                                tok.to_ascii_uppercase().as_str(),
                                "RSLC" | "GSLC" | "GCOV" | "GUNW" | "GOFF" | "RIFG" | "RUNW" | "ROFF" | "SME2"
                            )
                        })
                        .map(str::to_ascii_uppercase)
                })
                .unwrap_or_else(|| "RSLC".to_owned());

            if req_level == "SME2" {
                return Err(
                    "NISAR S-Band 目前开放 RSLC/GSLC/GCOV/GUNW/GOFF/RIFG/RUNW/ROFF 8 级产品；SME2 土壤湿度产品仅在 L-Band 提供。"
                        .to_owned(),
                );
            }
            let sband_meta = insar_provider::bhoonidhi::lookup_bhoonidhi_sband_product(&req_level)
                .ok_or_else(|| format!("NISAR_SBAND_UNSUPPORTED_LEVEL: 未知 S-Band 产品级别 {req_level}"))?;

            let (start, end) = parse_date_range_window(req.start_date.as_deref(), req.end_date.as_deref(), kw_raw);
            let search_only = has_search_only_token(req.keyword.as_deref())
                && req.tool_name.as_deref() != Some("download");

            let sess_guard =
                insar_download::nisar_multilevel_subset::BhoonidhiSessionGuard::authenticate()?;
            let (mut bearer_tok, mut tok_gen) = sess_guard.access_token()?;
            let client = build_http_client_with_timeout(app_data_dir, 90)?;

            let search_url = insar_provider::bhoonidhi::BHOONIDHI_SEARCH_URL;
            let date_windows = split_bhoonidhi_date_windows(&start, &end);
            let mut collected_features: Vec<Value> = Vec::new();
            let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut any_window_succeeded = false;
            let mut last_search_err = String::new();

            let requested_orbit_filter = parse_orbit_filter_token(Some(kw_raw));

            'window_loop: for (win_start, win_end) in date_windows {
                let search_body = json!({
                    "collections": [sband_meta.collection_id],
                    "bbox": [bbox[0].min(bbox[2]), bbox[1].min(bbox[3]), bbox[0].max(bbox[2]), bbox[1].max(bbox[3])],
                    "datetime": format!("{win_start}T00:00:00Z/{win_end}T23:59:59Z"),
                    "filter": {
                        "op": "and",
                        "args": [{ "op": "eq", "args": [{ "property": "Online" }, "Y"] }]
                    },
                    "filter-lang": "cql2-json",
                    "limit": 50
                });

                for attempt in 0..3 {
                    let resp = client
                        .post(search_url)
                        .header(reqwest::header::ACCEPT, "application/json")
                        .bearer_auth(&bearer_tok)
                        .json(&search_body)
                        .send();
                    match resp {
                        Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED && attempt < 2 => {
                            let (nt, ng) = sess_guard.refresh_after_rejection(tok_gen)?;
                            bearer_tok = nt;
                            tok_gen = ng;
                            continue;
                        }
                        Ok(r) if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < 2 => {
                            std::thread::sleep(Duration::from_secs(35));
                            continue;
                        }
                        Ok(r) if r.status().is_success() => {
                            let win_json = r
                                .json::<Value>()
                                .map_err(|e| format!("BHOONIDHI_SEARCH_JSON_PARSE_FAILED: {e}"))?;
                            any_window_succeeded = true;
                            if let Some(feats) = win_json["features"].as_array() {
                                for feat in feats {
                                    let Some(id) = feat["id"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                                        continue;
                                    };
                                    let props = &feat["properties"];
                                    let online_val = props["Online"].as_str().or_else(|| props["online"].as_str()).unwrap_or("Y");
                                    if online_val.eq_ignore_ascii_case("N") {
                                        continue;
                                    }
                                    if let Some(req_orbit) = requested_orbit_filter {
                                        if parse_bhoonidhi_orbit_direction(props, id) != Some(req_orbit) {
                                            continue;
                                        }
                                    }
                                    if seen_ids.insert(id.to_owned()) {
                                        collected_features.push(feat.clone());
                                        if collected_features.len() >= 50 {
                                            break 'window_loop;
                                        }
                                    }
                                }
                            }
                            break;
                        }
                        Ok(r) => {
                            let st = r.status();
                            let txt = r.text().unwrap_or_default();
                            let lower_txt = txt.to_ascii_lowercase();
                            if st.as_u16() == 404
                                && (lower_txt.contains("no results found")
                                    || lower_txt.contains("no data found"))
                            {
                                any_window_succeeded = true;
                                break;
                            }
                            last_search_err = format!("HTTP {st}: {}", txt.chars().take(180).collect::<String>());
                            break;
                        }
                        Err(e) => {
                            last_search_err = e.to_string();
                        }
                    }
                }
            }
            if !any_window_succeeded {
                return Err(format!(
                    "BHOONIDHI_SBAND_SEARCH_FAILED ({}): {last_search_err}",
                    sband_meta.collection_id
                ));
            }

            let check_out_dir = req
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            let mut items: Vec<Value> = Vec::new();
            for feat in &collected_features {
                let Some(id) = feat["id"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                    continue;
                };
                let props = &feat["properties"];
                let online_val = props["Online"].as_str().or_else(|| props["online"].as_str()).unwrap_or("Y");
                if online_val.eq_ignore_ascii_case("N") {
                    continue;
                }
                let item_bbox = feat
                    .get("geometry")
                    .and_then(|g| insar_provider::bhoonidhi::derive_bbox_from_geojson(g).ok())
                    .map(|b| [b.west, b.south, b.east, b.north])
                    .unwrap_or(bbox);
                let dl_url = reqwest::Url::parse_with_params(
                    "https://bhoonidhi-api.nrsc.gov.in/download",
                    &[("id", id), ("collection", sband_meta.collection_id)],
                )
                .map(|u| u.to_string())
                .unwrap_or_default();
                let dt = props["datetime"]
                    .as_str()
                    .or_else(|| props["start_datetime"].as_str())
                    .unwrap_or("");
                let size_bytes = props["size"]
                    .as_u64()
                    .or_else(|| props["ContentLength"].as_u64())
                    .or_else(|| props["file_size_bytes"].as_u64());
                let existing_local_path = check_out_dir.as_ref().and_then(|dir| {
                    let cand = dir.join(format!("{id}.h5"));
                    existing_valid_science_file_bytes(&cand)
                        .map(|_| cand.to_string_lossy().to_string())
                });
                let geometry = feat
                    .get("geometry")
                    .filter(|g| g.is_object())
                    .cloned()
                    .unwrap_or_else(|| {
                        json!({
                            "type": "Polygon",
                            "coordinates": [[
                                [item_bbox[0], item_bbox[1]],
                                [item_bbox[2], item_bbox[1]],
                                [item_bbox[2], item_bbox[3]],
                                [item_bbox[0], item_bbox[3]],
                                [item_bbox[0], item_bbox[1]]
                            ]]
                        })
                    });
                let orbit_dir = parse_bhoonidhi_orbit_direction(props, id);
                let track_num = props["Track"].as_u64();
                let frame_num = props["Frame"].as_u64();
                let scene_identifier = props["Scene_Identifier"]
                    .as_str()
                    .or_else(|| props["scene_identifier"].as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(id);
                let pol_str = props["Polarization"]
                    .as_str()
                    .or_else(|| props["polarization"].as_str())
                    .unwrap_or("-");
                let look_dir = props["Look_dir"]
                    .as_str()
                    .or_else(|| props["LookDirection"].as_str())
                    .map(|v| if v.trim().to_ascii_uppercase().starts_with('L') { "LEFT" } else { "RIGHT" });
                let raw_thumb_href = feat
                    .pointer("/assets/thumbnail/href")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                items.push(json!({
                    "id": id,
                    "title": scene_identifier,
                    "short_name": sband_meta.collection_id,
                    "collection": sband_meta.collection_id,
                    "product_level": req_level,
                    "product_type": req_level,
                    "beam_mode": req_level,
                    "polarization": pol_str,
                    "band": "S",
                    "flight_direction": orbit_dir,
                    "look_direction": look_dir,
                    "track": track_num,
                    "path": track_num,
                    "relative_orbit": track_num,
                    "frame": frame_num,
                    "datetime": dt,
                    "href": dl_url,
                    "data_url": dl_url,
                    "download_url": dl_url,
                    "browse_url": raw_thumb_href,
                    "thumbnail_url": raw_thumb_href,
                    "preview_url": raw_thumb_href,
                    "size_bytes": size_bytes,
                    "file_size": size_bytes,
                    "bbox": item_bbox,
                    "already_downloaded": existing_local_path.is_some(),
                    "local_file_path": existing_local_path,
                    "geometry": geometry,
                }));
            }

            let mut already_exists = false;
            let mut target_item_id_out: Option<String> = None;
            let mut downloaded_file: Option<String> = None;
            let mut downloaded_bytes: u64 = 0;

            if !search_only && !items.is_empty() {
                let req_target = req
                    .osm_tag
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let target_item = if let Some(tid) = req_target {
                    items
                        .iter()
                        .find(|it| it["id"].as_str() == Some(tid))
                        .ok_or_else(|| format!("BHOONIDHI_TARGET_NOT_FOUND: 未在检索结果中找到 {tid}"))?
                } else {
                    &items[0]
                };
                let item_id = target_item["id"].as_str().unwrap_or("nisar_sband").to_owned();
                target_item_id_out = Some(item_id.clone());
                let data_url = target_item["data_url"]
                    .as_str()
                    .ok_or_else(|| format!("BHOONIDHI_MISSING_DOWNLOAD_URL: {item_id}"))?;
                let out_dir = req
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join("bhoonidhi_sband"));
                let _ = fs::create_dir_all(&out_dir);
                let safe_stem: String = item_id
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let safe_name = if safe_stem.to_ascii_lowercase().ends_with(".h5") {
                    safe_stem
                } else {
                    format!("{safe_stem}.h5")
                };
                let fpath = out_dir.join(safe_name);

                if let Some(existing_bytes) = existing_valid_science_file_bytes(&fpath) {
                    already_exists = true;
                    downloaded_bytes = existing_bytes;
                    downloaded_file = Some(fpath.to_string_lossy().to_string());
                } else {
                    let mut dl_resp = None;
                    for attempt in 0..4 {
                        let r = client.get(data_url).bearer_auth(&bearer_tok).send();
                        match r {
                            Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED && attempt < 3 => {
                                let (nt, ng) = sess_guard.refresh_after_rejection(tok_gen)?;
                                bearer_tok = nt;
                                tok_gen = ng;
                                continue;
                            }
                            Ok(resp) if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < 3 => {
                                std::thread::sleep(Duration::from_secs(35));
                                continue;
                            }
                            Ok(resp) if resp.status().is_success() => {
                                dl_resp = Some(resp);
                                break;
                            }
                            Ok(resp) => {
                                return Err(format!(
                                    "BHOONIDHI_DOWNLOAD_HTTP_ERROR ({item_id}): HTTP {}",
                                    resp.status()
                                ));
                            }
                            Err(e) => {
                                return Err(format!("BHOONIDHI_DOWNLOAD_NETWORK_FAILED ({item_id}): {e}"));
                            }
                        }
                    }
                    let resp = dl_resp.ok_or_else(|| format!("BHOONIDHI_DOWNLOAD_RATE_LIMITED ({item_id})"))?;
                    let written = stream_http_to_file_verified(
                        resp,
                        &fpath,
                        8 * 1024 * 1024 * 1024,
                        &format!("BHOONIDHI_SBAND_DOWNLOAD ({item_id})"),
                    )?;
                    let head = {
                        use std::io::Read;
                        let mut f = fs::File::open(&fpath)
                            .map_err(|e| format!("BHOONIDHI_READ_DOWNLOADED_FAILED: {e}"))?;
                        let mut b = [0u8; 8];
                        let n = f.read(&mut b).unwrap_or(0);
                        b[..n].to_vec()
                    };
                    if !head.starts_with(b"\x89HDF\r\n\x1a\n") {
                        let _ = fs::remove_file(&fpath);
                        return Err(format!(
                            "BHOONIDHI_INVALID_HDF5_MAGIC: 下载的 {item_id} 非标准 NISAR HDF5 文件"
                        ));
                    }
                    downloaded_bytes = written;
                    downloaded_file = Some(fpath.to_string_lossy().to_string());
                }
            }
            let _ = sess_guard.explicit_logout();

            let empty_reason_zh = if items.is_empty() {
                Some(format!(
                    "当前研究区与时间窗口内暂未检索到 ISRO Bhoonidhi {} 公开归档数据。",
                    sband_meta.collection_id
                ))
            } else {
                None
            };
            Ok(json!({
                "ok": true,
                "dataset": "bhoonidhi_sband_products",
                "short_name": sband_meta.collection_id,
                "product_level": req_level,
                "band": "S",
                "count": items.len(),
                "empty_reason_zh": empty_reason_zh,
                "already_exists": already_exists,
                "target_item_id": target_item_id_out,
                "downloaded_file": downloaded_file,
                "downloaded_path": downloaded_file,
                "downloaded_bytes": downloaded_bytes,
                "items": items,
            }))
        }
        "nisar_hdf5_subset" => {
            let gp = req.gee_params.as_ref();
            let url = gp
                .and_then(|v| v["url"].as_str())
                .or(req.osm_tag.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "NISAR_SUBSET_URL_REQUIRED: 缺少远程 HDF5 产品下载地址".to_owned())?
                .to_owned();
            let granule_id = gp
                .and_then(|v| v["granule_id"].as_str())
                .or(req.code.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("NISAR_SUBSET_PRODUCT")
                .to_owned();
            let product_level = gp
                .and_then(|v| v["product_level"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("GSLC")
                .to_ascii_uppercase();
            let band = gp
                .and_then(|v| v["band"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("L")
                .to_ascii_uppercase();
            let frequency = gp
                .and_then(|v| v["frequency"].as_str())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            let datasets = gp
                .and_then(|v| v["datasets"].as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.trim().to_owned()))
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                });
            let bbox = req
                .bbox
                .ok_or_else(|| "NISAR_SUBSET_BBOX_REQUIRED: 请先在地图或弹窗中框选裁剪范围 (AOI)".to_owned())?;
            let out_dir = req
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| app_data_dir.join("geodata_downloads").join("nisar_subsets"));

            let sub_req = insar_download::nisar_multilevel_subset::NisarMultilevelSubsetRequest {
                url,
                granule_id: granule_id.clone(),
                product_level,
                band,
                bbox_wgs84: bbox,
                output_dir: out_dir,
                frequency,
                datasets,
            };
            if req.tool_name.as_deref() == Some("preflight") {
                let pre = insar_download::nisar_multilevel_subset::preflight_remote_nisar_subset(&sub_req)?;
                serde_json::to_value(pre).map_err(|e| format!("NISAR_PREFLIGHT_SERIALIZE: {e}"))
            } else {
                let res = insar_download::nisar_multilevel_subset::execute_remote_nisar_subset(&sub_req)?;
                let out_str = res.output_path.to_string_lossy().to_string();
                Ok(json!({
                    "ok": true,
                    "dataset": "nisar_hdf5_subset",
                    "target_item_id": granule_id,
                    "product_level": res.product_level,
                    "band": res.band,
                    "frequency": res.frequency,
                    "grid_kind": res.grid_kind,
                    "projection_epsg": res.projection_epsg,
                    "source_dims": res.source_dims,
                    "subset_dims": res.subset_dims,
                    "copied_datasets": res.copied_datasets,
                    "downloaded_file": out_str,
                    "downloaded_path": out_str,
                    "output_path": out_str,
                    "downloaded_bytes": res.output_bytes,
                    "source_total_bytes": res.source_total_bytes,
                    "remote_bytes_fetched": res.remote_bytes_fetched,
                    "range_requests_count": res.range_requests_count,
                }))
            }
        }
        "scan_local_slc_dates" => {
            let dir_str = req
                .code
                .as_deref()
                .or(req.output_path.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "LOCAL_SLC_DIR_REQUIRED: 请选择或输入包含本地 SLC 影像的文件夹路径".to_owned())?;
            scan_local_slc_dates_in_directory(Path::new(dir_str))
        }
        "whu_clcd" | "tum_eot20" | "gebco_2026" | "noaa_etopo_2022" => {
            execute_open_earth_env_dataset_query(app_data_dir, &dataset, req)
        }
        "aws_sentinel1_grd" | "sentinel1_grd_aws" => {
            execute_aws_sentinel1_grd_query(app_data_dir, req)
        }
        "noaa_aws_weather" | "noaa_gfs" | "noaa_goes" => {
            execute_noaa_aws_weather_query(app_data_dir, req)
        }
        "overture_maps" | "overture" => execute_overture_maps_query(app_data_dir, req),
        "esri_wayback_versions"
        | "esri_wayback_probe_zoom"
        | "esri_wayback_scan_start"
        | "esri_wayback_scan_status"
        | "esri_wayback_scan_cancel" => wayback::handle_wayback_dataset(app_data_dir, &dataset, req),
        other => Err(format!(
            "UNSUPPORTED_OPEN_GEODATA_DATASET: {other} (支持 open_dem_tiles / xyz_satellite_tiles / mca_xzqh / mca_placename / datav_bound / open_meteo_era5 / overpass_osm / worldpop / whitebox_run / duckdb_sql / gee_catalog / gee_map_tiles / whu_clcd / tum_eot20 / gebco_2026 / noaa_etopo_2022 / aws_sentinel1_grd / noaa_aws_weather / overture_maps)"
        )),
    }
}

fn execute_open_earth_env_dataset_query(
    app_data_dir: &Path,
    dataset: &str,
    req: &OpenGeodataQueryRequest,
) -> Result<Value, String> {
    let (raw_start, raw_end, _sub_filter, kw_download) =
        parse_geodata_window_and_sub(req, 3650, 0, "ALL");
    let has_output_path = req
        .output_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some();
    let is_explicit_search = req
        .keyword
        .as_deref()
        .map(|k| {
            k.split('|')
                .any(|tok| tok.trim().eq_ignore_ascii_case("search"))
        })
        .unwrap_or(false);
    let should_download = kw_download || (has_output_path && !is_explicit_search);
    let bbox = req.bbox.unwrap_or([100.0, 20.0, 125.0, 45.0]);
    let w = bbox[0].min(bbox[2]);
    let s = bbox[1].min(bbox[3]);
    let e = bbox[0].max(bbox[2]);
    let n = bbox[1].max(bbox[3]);

    let rect_geom = |bb: [f64; 4]| {
        json!({
            "type": "Polygon",
            "coordinates": [[
                [bb[0], bb[1]],
                [bb[2], bb[1]],
                [bb[2], bb[3]],
                [bb[0], bb[3]],
                [bb[0], bb[1]]
            ]]
        })
    };

    let mut items: Vec<Value> = Vec::new();
    let provider_name: &str;

    match dataset {
        "whu_clcd" => {
            provider_name = "Zenodo / WHU CLCD v1.0.4";
            let china_bb = [73.5_f64, 18.0, 135.1, 53.6];
            let intersects_china =
                w <= china_bb[2] && e >= china_bb[0] && s <= china_bb[3] && n >= china_bb[1];
            if intersects_china {
                let parse_year = |raw: &str, fallback: i32| -> i32 {
                    raw.get(0..4)
                        .and_then(|y| y.parse::<i32>().ok())
                        .unwrap_or(fallback)
                };
                let y0 = parse_year(&raw_start, 2015).clamp(1985, 2025);
                let y1 = parse_year(&raw_end, 2025).clamp(1985, 2025);
                let (yr_lo, yr_hi) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
                for yr in (yr_lo..=yr_hi).rev().take(12) {
                    let fname = format!("CLCD_v01_{yr}_albert.tif");
                    let dl_url =
                        format!("https://zenodo.org/records/18180184/files/{fname}?download=1");
                    items.push(json!({
                        "id": format!("WHU_CLCD_v1.0.4_{yr}"),
                        "title": format!("WHU China Land Cover Dataset (CLCD v1.0.4 · {yr} 年 · 30m)"),
                        "filename": fname,
                        "datetime": format!("{yr}-07-01T00:00:00Z"),
                        "year": yr,
                        "provider": provider_name,
                        "resolution_m": 30,
                        "bbox": china_bb,
                        "geometry": rect_geom(china_bb),
                        "href": dl_url,
                        "download_url": dl_url,
                    }));
                }
            }
        }
        "tum_eot20" => {
            provider_name = "SEANOE / DGFI-TUM EOT20";
            let global_bb = [-180.0_f64, -90.0, 180.0, 90.0];
            items.push(json!({
                "id": "DGFI_TUM_EOT20_OCEAN_TIDES_NETCDF",
                "title": "DGFI-TUM EOT20 Ocean Tides (0.125° Global NetCDF Bundle)",
                "filename": "EOT20_ocean_tides_nc.zip",
                "datetime": "2021-06-01T00:00:00Z",
                "provider": provider_name,
                "bbox": global_bb,
                "geometry": rect_geom(global_bb),
                "href": "https://www.seanoe.org/data/00683/79489/data/85762.zip",
                "download_url": "https://www.seanoe.org/data/00683/79489/data/85762.zip",
            }));
            items.push(json!({
                "id": "DGFI_TUM_EOT20_LOAD_TIDES_NETCDF",
                "title": "DGFI-TUM EOT20 Load Tides (0.125° Global NetCDF Bundle)",
                "filename": "EOT20_load_tides_nc.zip",
                "datetime": "2021-06-01T00:00:00Z",
                "provider": provider_name,
                "bbox": global_bb,
                "geometry": rect_geom(global_bb),
                "href": "https://www.seanoe.org/data/00683/79489/data/85763.zip",
                "download_url": "https://www.seanoe.org/data/00683/79489/data/85763.zip",
            }));
        }
        "gebco_2026" => {
            provider_name = "GEBCO / BODC";
            let lat_bands: [(f64, f64, &str); 2] =
                [(-90.0, 0.0, "n0.0_s-90.0"), (0.0, 90.0, "n90.0_s0.0")];
            let lon_bands: [(f64, f64, &str); 4] = [
                (-180.0, -90.0, "w-180.0_e-90.0"),
                (-90.0, 0.0, "w-90.0_e0.0"),
                (0.0, 90.0, "w0.0_e90.0"),
                (90.0, 180.0, "w90.0_e180.0"),
            ];
            for (lat_lo, lat_hi, lat_tag) in lat_bands {
                if n < lat_lo || s > lat_hi {
                    continue;
                }
                for (lon_lo, lon_hi, lon_tag) in lon_bands {
                    if e < lon_lo || w > lon_hi {
                        continue;
                    }
                    let tile_bb = [lon_lo, lat_lo, lon_hi, lat_hi];
                    let fname = format!("gebco_2026_sub_ice_{lat_tag}_{lon_tag}_geotiff.tif");
                    let dl_url = format!(
                        "https://dap.ceda.ac.uk/bodc/gebco/global/gebco_2026/sub_ice_topography_bathymetry/geotiff/{fname}"
                    );
                    items.push(json!({
                        "id": format!("GEBCO_15S_{lat_tag}_{lon_tag}"),
                        "title": format!("GEBCO 2026 Global 15 arc-sec Grid Tile ({lat_tag}, {lon_tag})"),
                        "filename": fname,
                        "datetime": "2026-04-22T00:00:00Z",
                        "provider": provider_name,
                        "bbox": tile_bb,
                        "geometry": rect_geom(tile_bb),
                        "href": dl_url,
                        "download_url": dl_url,
                    }));
                }
            }
        }
        "noaa_etopo_2022" => {
            provider_name = "NOAA NCEI ETOPO 2022";
            let min_lat_idx = (s.floor() as i32 / 15 * 15).clamp(-90, 75);
            let max_lat_idx = (n.floor() as i32 / 15 * 15).clamp(-90, 75);
            let min_lon_idx = (w.floor() as i32 / 15 * 15).clamp(-180, 165);
            let max_lon_idx = (e.floor() as i32 / 15 * 15).clamp(-180, 165);
            let mut lat_top = (max_lat_idx + 15).min(90);
            while lat_top > min_lat_idx && items.len() < 24 {
                let lat_bot = lat_top - 15;
                let lat_tag = if lat_top >= 0 {
                    format!("N{:02}", lat_top)
                } else {
                    format!("S{:02}", -lat_top)
                };
                let mut lon_left = min_lon_idx;
                while lon_left <= max_lon_idx && items.len() < 24 {
                    let lon_right = (lon_left + 15).min(180);
                    let lon_tag = if lon_left >= 0 {
                        format!("E{:03}", lon_left)
                    } else {
                        format!("W{:03}", -lon_left)
                    };
                    let tile_bb = [
                        lon_left as f64,
                        lat_bot as f64,
                        lon_right as f64,
                        lat_top as f64,
                    ];
                    let fname = format!("ETOPO_2022_v1_15s_{lat_tag}{lon_tag}_surface.tif");
                    let dl_url = format!(
                        "https://www.ngdc.noaa.gov/mgg/global/relief/ETOPO2022/data/15s/15s_surface_elev_gtif/{fname}"
                    );
                    items.push(json!({
                        "id": format!("ETOPO_2022_v1_15s_{lat_tag}{lon_tag}_surface"),
                        "title": format!("NOAA ETOPO 2022 15 arc-sec Surface Elevation Tile ({lat_tag}{lon_tag})"),
                        "filename": fname,
                        "datetime": "2022-12-01T00:00:00Z",
                        "provider": provider_name,
                        "bbox": tile_bb,
                        "geometry": rect_geom(tile_bb),
                        "href": dl_url,
                        "download_url": dl_url,
                    }));
                    lon_left += 15;
                }
                lat_top -= 15;
            }
        }
        _ => unreachable!(),
    }

    let mut downloaded_file: Option<String> = None;
    let mut downloaded_bytes: u64 = 0;
    let mut target_item_id_out: Option<String> = None;
    let mut already_exists = false;

    if should_download {
        let target_item = if let Some(target_id) = req
            .osm_tag
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            items
                .iter()
                .find(|it| it["id"].as_str() == Some(target_id))
                .or_else(|| items.first())
                .ok_or_else(|| {
                    format!("OPEN_GEODATA_NO_ITEMS: 当前区域与时间窗下无可用 {dataset} 条目")
                })?
        } else {
            items.first().ok_or_else(|| {
                format!("OPEN_GEODATA_NO_ITEMS: 当前区域与时间窗下无可用 {dataset} 条目")
            })?
        };
        let item_id = target_item["id"].as_str().unwrap_or(dataset).to_owned();
        let fname = target_item["filename"]
            .as_str()
            .unwrap_or(&item_id)
            .to_owned();
        let dl_url = target_item["download_url"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        target_item_id_out = Some(item_id.clone());

        let out_dir = req
            .output_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                app_data_dir
                    .join("geodata_downloads")
                    .join(dataset.to_ascii_uppercase())
            });
        fs::create_dir_all(&out_dir).map_err(|e| {
            format!(
                "CREATE_OUTPUT_DIR_FAILED ({}): {e}",
                out_dir.to_string_lossy()
            )
        })?;
        let dest = out_dir.join(&fname);
        if dest.is_file() && dest.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            already_exists = true;
            downloaded_bytes = dest.metadata().map(|m| m.len()).unwrap_or(0);
            downloaded_file = Some(dest.to_string_lossy().to_string());
        } else {
            let client = build_download_http_client(app_data_dir)?;
            let resp = match http_get_with_retry(&client, &dl_url, None, 3)
                .map_err(|e| format!("OPEN_GEODATA_DOWNLOAD_FAILED ({dl_url}): {e}"))?
            {
                r if r.status().as_u16() == 404
                    && dataset == "noaa_etopo_2022"
                    && dl_url.contains("15s_bed_elev_gtif/")
                    && dl_url.ends_with("_bed.tif") =>
                {
                    let surface_url = dl_url
                        .replace("15s_bed_elev_gtif/", "15s_surface_elev_gtif/")
                        .replace("_bed.tif", "_surface.tif");
                    http_get_with_retry(&client, &surface_url, None, 3)
                        .map_err(|e| format!("OPEN_GEODATA_DOWNLOAD_FAILED ({surface_url}): {e}"))?
                        .error_for_status()
                        .map_err(|e| format!("OPEN_GEODATA_HTTP_ERROR ({surface_url}): {e}"))?
                }
                r => r
                    .error_for_status()
                    .map_err(|e| format!("OPEN_GEODATA_HTTP_ERROR ({dl_url}): {e}"))?,
            };
            let tmp_path = out_dir.join(format!("{fname}.download.tmp"));
            let written = stream_http_to_file_verified(
                resp,
                &tmp_path,
                4 * 1024 * 1024 * 1024,
                &format!("OPEN_GEODATA_DOWNLOAD ({item_id})"),
            )?;
            let head_buf = {
                use std::io::Read;
                let mut f = fs::File::open(&tmp_path).map_err(|e| e.to_string())?;
                let mut b = [0u8; 96];
                let n = f.read(&mut b).unwrap_or(0);
                b[..n].to_vec()
            };
            let head_lower = String::from_utf8_lossy(&head_buf)
                .trim_start()
                .to_ascii_lowercase();
            if head_lower.starts_with("<!doctype") || head_lower.starts_with("<html") {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!("OPEN_GEODATA_UNEXPECTED_HTML_PAYLOAD ({dl_url})"));
            }
            fs::rename(&tmp_path, &dest).map_err(|e| format!("RENAME_FAILED: {e}"))?;
            downloaded_bytes = written;
            downloaded_file = Some(dest.to_string_lossy().to_string());
        }
    }

    Ok(json!({
        "ok": true,
        "dataset": dataset,
        "provider": provider_name,
        "count": items.len(),
        "already_exists": already_exists,
        "target_item_id": target_item_id_out,
        "downloaded_file": downloaded_file,
        "downloaded_path": downloaded_file,
        "downloaded_bytes": downloaded_bytes,
        "items": items,
    }))
}

fn parse_date_range_window(
    start_opt: Option<&str>,
    end_opt: Option<&str>,
    keyword: &str,
) -> (String, String) {
    let mut start = start_opt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let mut end = end_opt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if start.is_none() || end.is_none() {
        for tok in keyword.split('|').map(str::trim) {
            if let Some((a, b)) = tok.split_once("..") {
                let sa = a.trim();
                let sb = b.trim();
                if start.is_none() && !sa.is_empty() {
                    start = Some(sa.to_owned());
                }
                if end.is_none() && !sb.is_empty() {
                    end = Some(sb.to_owned());
                }
            }
        }
    }
    (
        start.unwrap_or_else(|| "2025-01-01".to_owned()),
        end.unwrap_or_else(|| "2026-12-31".to_owned()),
    )
}

/// Split an arbitrary `YYYY-MM-DD` date window newest-first into contiguous <=360-day
/// chunks because Bhoonidhi STAC `/data/search` rejects intervals exceeding 366 days (HTTP 406).
fn split_bhoonidhi_date_windows(start_ymd: &str, end_ymd: &str) -> Vec<(String, String)> {
    let Ok(start_d) = chrono::NaiveDate::parse_from_str(start_ymd.trim(), "%Y-%m-%d") else {
        return vec![(start_ymd.to_owned(), end_ymd.to_owned())];
    };
    let Ok(end_d) = chrono::NaiveDate::parse_from_str(end_ymd.trim(), "%Y-%m-%d") else {
        return vec![(start_ymd.to_owned(), end_ymd.to_owned())];
    };
    if end_d <= start_d + chrono::Duration::days(360) {
        return vec![(start_ymd.to_owned(), end_ymd.to_owned())];
    }
    let mut windows = Vec::new();
    let mut cur_end = end_d;
    for _ in 0..10 {
        if cur_end < start_d {
            break;
        }
        let span_start = cur_end - chrono::Duration::days(359);
        let chunk_start = if span_start < start_d {
            start_d
        } else {
            span_start
        };
        windows.push((
            chunk_start.format("%Y-%m-%d").to_string(),
            cur_end.format("%Y-%m-%d").to_string(),
        ));
        if chunk_start <= start_d {
            break;
        }
        cur_end = chunk_start - chrono::Duration::days(1);
    }
    if windows.is_empty() {
        windows.push((start_ymd.to_owned(), end_ymd.to_owned()));
    }
    windows
}

fn is_valid_yyyymmdd_slice(bytes: &[u8]) -> Option<String> {
    if bytes.len() != 8 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let y: u32 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
    let m: u32 = std::str::from_utf8(&bytes[4..6]).ok()?.parse().ok()?;
    let d: u32 = std::str::from_utf8(&bytes[6..8]).ok()?.parse().ok()?;
    if !(2010..=2035).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(format!("{y:04}{m:02}{d:02}"))
}

fn extract_slc_dates_and_times(name: &str) -> (Vec<String>, Option<(String, String)>) {
    let b = name.as_bytes();
    let mut dates = Vec::new();
    let mut utc_hm: Option<(String, String)> = None;

    for i in 0..b.len() {
        if i + 8 > b.len() {
            break;
        }
        let prev_ok = i == 0 || !b[i - 1].is_ascii_digit();
        let next_idx = i + 8;
        let next_ok = next_idx == b.len()
            || !b[next_idx].is_ascii_digit()
            || (matches!(b[next_idx], b'T' | b't') && next_idx + 5 <= b.len());
        if !prev_ok || !next_ok {
            continue;
        }
        if let Some(ymd) = is_valid_yyyymmdd_slice(&b[i..next_idx]) {
            if next_idx + 5 <= b.len()
                && matches!(b[next_idx], b'T' | b't')
                && b[next_idx + 1..next_idx + 5].iter().all(u8::is_ascii_digit)
            {
                if let (Ok(hh), Ok(mm)) = (
                    std::str::from_utf8(&b[next_idx + 1..next_idx + 3])
                        .unwrap_or("")
                        .parse::<u32>(),
                    std::str::from_utf8(&b[next_idx + 3..next_idx + 5])
                        .unwrap_or("")
                        .parse::<u32>(),
                ) {
                    if hh < 24 && mm < 60 && utc_hm.is_none() {
                        utc_hm = Some((format!("{hh:02}"), format!("{mm:02}")));
                    }
                }
            }
            if !dates.contains(&ymd) {
                dates.push(ymd);
            }
        }
    }
    (dates, utc_hm)
}

fn is_slc_candidate_entry(name: &str, is_dir: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    if is_dir {
        return lower.ends_with(".safe")
            || lower.contains("s1a_")
            || lower.contains("s1b_")
            || lower.contains("s1c_")
            || lower.contains("_slc")
            || lower.contains("nisar_")
            || lower.contains("alos2");
    }
    [
        ".zip", ".safe", ".h5", ".hdf", ".hdf5", ".slc", ".tif", ".tiff", ".bin", ".dim",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

fn is_supported_slc_script_or_manifest(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("py" | "metalink" | "meta4" | "txt" | "urls" | "csv" | "geojson" | "json")
    )
}

fn extract_s1_scenes_from_text(text: &str) -> Vec<String> {
    let mut scenes = std::collections::BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 25 <= bytes.len() {
        if bytes[i] == b'S'
            && bytes[i + 1] == b'1'
            && matches!(bytes[i + 2], b'A' | b'B' | b'C' | b'D')
            && bytes[i + 3] == b'_'
        {
            let prev_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            if prev_ok {
                let mut end = i + 4;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                {
                    end += 1;
                }
                if let Ok(token) = std::str::from_utf8(&bytes[i..end]) {
                    if token.len() >= 32
                        && (token.contains("_SLC_")
                            || token.contains("_GRD_")
                            || token.contains("_RAW_"))
                    {
                        scenes.insert(token.to_owned());
                    }
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    scenes.into_iter().collect()
}

fn scan_local_slc_dates_in_file(file: &Path) -> Result<Value, String> {
    if !is_supported_slc_script_or_manifest(file) {
        return Err(format!(
            "LOCAL_SLC_SCRIPT_UNSUPPORTED: 仅支持读取目录或 ASF 下载脚本/清单文件（.py / .metalink / .meta4 / .txt / .urls / .csv / .geojson / .json）({})",
            file.to_string_lossy()
        ));
    }
    let meta = fs::metadata(file).map_err(|_| {
        format!(
            "LOCAL_SLC_DIR_NOT_FOUND: 文件不存在或不可访问 ({})",
            file.to_string_lossy()
        )
    })?;
    if meta.len() > 8 * 1024 * 1024 {
        return Err("LOCAL_SLC_SCRIPT_TOO_LARGE: 脚本或清单文件体积超过 8 MiB 上限".to_owned());
    }
    let text = fs::read_to_string(file)
        .map_err(|e| format!("LOCAL_SLC_SCRIPT_READ_FAILED: 无法按文本读取文件 ({e})"))?;
    let mut all_dates: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut hm_counts: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    let (dates, hm) = extract_slc_dates_and_times(&text);
    for d in dates {
        all_dates.insert(d);
    }
    if let Some(hm_pair) = hm {
        *hm_counts.entry(hm_pair).or_insert(0) += 1;
    }
    for line in text.lines() {
        let (_, line_hm) = extract_slc_dates_and_times(line);
        if let Some(pair) = line_hm {
            *hm_counts.entry(pair).or_insert(0) += 1;
        }
    }
    let s1_scenes = extract_s1_scenes_from_text(&text);
    for scene in &s1_scenes {
        let (scene_dates, scene_hm) = extract_slc_dates_and_times(scene);
        for d in scene_dates {
            all_dates.insert(d);
        }
        if let Some(pair) = scene_hm {
            *hm_counts.entry(pair).or_insert(0) += 1;
        }
    }
    let dominant_hm = hm_counts
        .into_iter()
        .max_by(|(ka, ca), (kb, cb)| ca.cmp(cb).then_with(|| kb.cmp(ka)))
        .map(|(k, _)| k);
    let dates_vec: Vec<String> = all_dates.into_iter().collect();
    let file_count = if !s1_scenes.is_empty() {
        s1_scenes.len()
    } else if !dates_vec.is_empty() {
        1
    } else {
        0
    };
    let matched_entries: Vec<String> = s1_scenes.iter().take(40).cloned().collect();
    Ok(json!({
        "ok": true,
        "dataset": "scan_local_slc_dates",
        "directory": file.to_string_lossy(),
        "source_kind": "script_or_manifest",
        "dates": dates_vec,
        "s1_scenes": s1_scenes,
        "file_count": file_count,
        "dominant_utc_hour": dominant_hm.as_ref().map(|(h, _)| h.clone()),
        "dominant_utc_minute": dominant_hm.as_ref().map(|(_, m)| m.clone()),
        "matched_entries": matched_entries,
    }))
}

fn scan_local_slc_dates_in_directory(dir: &Path) -> Result<Value, String> {
    if dir.is_file() {
        return scan_local_slc_dates_in_file(dir);
    }
    if !dir.is_dir() {
        return Err(format!(
            "LOCAL_SLC_DIR_NOT_FOUND: 目录不存在或不可访问 ({})",
            dir.to_string_lossy()
        ));
    }
    let mut stack: Vec<(PathBuf, u8)> = vec![(dir.to_path_buf(), 0)];
    let mut visited = 0usize;
    let mut all_dates: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut all_s1_scenes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut hm_counts: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    let mut matched_entries: Vec<String> = Vec::new();

    while let Some((cur, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&cur) else {
            continue;
        };
        for entry in rd.flatten() {
            visited += 1;
            if visited > 5000 {
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = path.is_dir();
            if is_slc_candidate_entry(&name, is_dir) {
                for scene in extract_s1_scenes_from_text(&name) {
                    all_s1_scenes.insert(scene);
                }
                let (dates, hm) = extract_slc_dates_and_times(&name);
                if !dates.is_empty() {
                    for d in dates {
                        all_dates.insert(d);
                    }
                    if let Some(hm_pair) = hm {
                        *hm_counts.entry(hm_pair).or_insert(0) += 1;
                    }
                    if matched_entries.len() < 40 {
                        matched_entries.push(name);
                    }
                    // Do not descend inside a .SAFE bundle once matched.
                    if is_dir
                        && path
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(|s| s.eq_ignore_ascii_case("safe"))
                            == Some(true)
                    {
                        continue;
                    }
                }
            }
            if is_dir && depth < 3 {
                stack.push((path, depth + 1));
            }
        }
        if visited > 5000 {
            break;
        }
    }

    let dominant_hm = hm_counts
        .into_iter()
        .max_by(|(ka, ca), (kb, cb)| ca.cmp(cb).then_with(|| kb.cmp(ka)))
        .map(|(k, _)| k);
    let dates_vec: Vec<String> = all_dates.into_iter().collect();
    let s1_scenes: Vec<String> = all_s1_scenes.into_iter().collect();
    let file_count = matched_entries.len();

    Ok(json!({
        "ok": true,
        "dataset": "scan_local_slc_dates",
        "directory": dir.to_string_lossy(),
        "source_kind": "directory",
        "dates": dates_vec,
        "s1_scenes": s1_scenes,
        "file_count": file_count,
        "dominant_utc_hour": dominant_hm.as_ref().map(|(h, _)| h.clone()),
        "dominant_utc_minute": dominant_hm.as_ref().map(|(_, m)| m.clone()),
        "matched_entries": matched_entries,
    }))
}

pub fn maybe_record_geodata_download_task(
    state: &crate::application::ApplicationServiceState,
    request: &OpenGeodataQueryRequest,
    result: &mut serde_json::Value,
    elapsed: std::time::Duration,
) {
    let agent_app = state.agent_application();
    maybe_record_geodata_download_task_on_service(&agent_app.0, request, result, elapsed);
}

pub fn maybe_record_geodata_download_task_on_service(
    service: &insar_application_service::ApplicationService,
    request: &OpenGeodataQueryRequest,
    result: &mut serde_json::Value,
    elapsed: std::time::Duration,
) {
    let candidate_file_str = [
        "downloaded_file",
        "downloaded_path",
        "mbtiles_path",
        "zip_archive_path",
        "exported_mosaic_path",
        "mosaic_geotiff_path",
        "mosaic_vrt_path",
    ]
    .iter()
    .find_map(|key| {
        result
            .get(*key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })
    .or_else(|| {
        if matches!(
            request.dataset.trim().to_ascii_lowercase().as_str(),
            "gee_chunked_download" | "gee_download"
        ) && result.get("quota_confirm_required") != Some(&Value::Bool(true))
        {
            result
                .get("output_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        } else {
            None
        }
    })
    .map(str::to_owned);

    if let Some(dl_path_str) = candidate_file_str {
        let saved_file_path = std::path::PathBuf::from(&dl_path_str);
        if let Ok(meta) = std::fs::metadata(&saved_file_path) {
            if meta.is_file() && meta.len() > 0 {
                let output_root = request
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(std::path::PathBuf::from)
                    .or_else(|| saved_file_path.parent().map(std::path::Path::to_path_buf))
                    .unwrap_or_else(|| saved_file_path.clone());
                let dataset_str = result
                    .get("dataset")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(request.dataset.as_str())
                    .to_owned();
                let provider_label = match dataset_str.as_str() {
                    "comet_licsar" => "COMET_LICSAR",
                    "open_meteo_era5" => "OPEN_METEO_ERA5",
                    "mpc_stac_science" | "esa_worldcover" | "jrc_gsw" | "alos_palsar_mosaic"
                    | "modis_ndvi" | "sentinel_5p" => "MPC_SCIENCE",
                    "asf_cmr_science" | "opera_disp_s1" | "aria_s1_gunw" | "nisar_asf_cmr"
                    | "nisar_products" => "ASF_CMR",
                    "nasa_black_marble" | "black_marble" | "viirs_black_marble" => "NASA_LAADS_CMR",
                    "nasa_emit" | "emit" | "emit_l2a" | "nasa_ecostress" | "ecostress"
                    | "ecostress_l2t" => "NASA_LPDAAC_CMR",
                    "bhoonidhi_sband_products" => "ISRO_BHOONIDHI",
                    "nisar_hdf5_subset" => "NISAR_SUBSET",
                    "usgs_earthquake" => "USGS_FDSN",
                    "ngl_gnss" => "UNR_NGL",
                    "ionospheric_ionex" | "igs_gim" => "IGS_IONEX",
                    "gee_catalog" | "gee_chunked_download" | "gee_download" => "GEE_CLOUD",
                    "xyz_satellite_tiles" => "XYZ_TILES",
                    "whu_clcd" => "ZENODO_WHU_CLCD",
                    "tum_eot20" => "SEANOE_TUM_EOT20",
                    "gebco_2026" => "BODC_GEBCO",
                    "noaa_etopo_2022" => "NOAA_ETOPO2022",
                    "aws_sentinel1_grd" | "sentinel1_grd_aws" => "AWS_SENTINEL1_GRD",
                    "noaa_aws_weather" | "noaa_gfs" | "noaa_goes" => "NOAA_AWS_WEATHER",
                    "overture_maps" | "overture" => "OVERTURE_MAPS_AWS",
                    _ => "OPEN_GEODATA",
                }
                .to_owned();
                let fallback_stem = saved_file_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("scene");
                let item_id = result
                    .get("target_item_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        request
                            .osm_tag
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                    })
                    .or_else(|| {
                        result
                            .get("items")
                            .and_then(|v| v.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|it| it.get("id"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or(fallback_stem)
                    .to_owned();
                let display_name = saved_file_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&item_id)
                    .to_owned();
                let acquisition_datetime = result
                    .get("items")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| {
                        arr.iter()
                            .find(|it| {
                                it.get("id").and_then(|v| v.as_str()) == Some(item_id.as_str())
                            })
                            .or_else(|| arr.first())
                    })
                    .and_then(|it| it.get("datetime"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                let bytes_written = result
                    .get("downloaded_bytes")
                    .and_then(|v| v.as_u64())
                    .filter(|&b| b > 0)
                    .unwrap_or(meta.len());
                let elapsed_millis = elapsed.as_millis().min(u64::MAX as u128) as u64;
                if let Ok(recorded) = service.record_external_download_task(
                    &insar_application_service::RecordExternalDownloadTaskRequest {
                        dataset: dataset_str,
                        provider_label,
                        item_id,
                        display_name,
                        acquisition_datetime,
                        bbox: request.bbox,
                        output_root,
                        saved_file_path,
                        bytes_written,
                        elapsed_millis: elapsed_millis.max(1),
                    },
                ) {
                    result["task_id"] = serde_json::json!(recorded.task_id);
                    result["task_revision"] = serde_json::json!(recorded.revision);
                    result["output_root"] =
                        serde_json::json!(recorded.output_root.to_string_lossy().to_string());
                    result["task_already_recorded"] = serde_json::json!(recorded.already_recorded);
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GfsIdxRecord {
    pub record_no: u64,
    pub start_byte: u64,
    pub end_byte_inclusive: Option<u64>,
    pub var: String,
    pub level: String,
    pub forecast: String,
}

fn aws_rect_geom(bb: [f64; 4]) -> Value {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [bb[0], bb[1]],
            [bb[2], bb[1]],
            [bb[2], bb[3]],
            [bb[0], bb[3]],
            [bb[0], bb[1]],
        ]]
    })
}

fn aws_should_download(req: &OpenGeodataQueryRequest) -> bool {
    let (_, _, _, kw_download) = parse_geodata_window_and_sub(req, 3650, 0, "ALL");
    let explicit_download_tool = req.tool_name.as_deref().map(str::trim).is_some_and(|t| {
        matches!(
            t.to_ascii_lowercase().as_str(),
            "download" | "crop" | "full" | "export_geojson"
        )
    });
    let explicit_gee_download = req
        .gee_params
        .as_ref()
        .and_then(|v| v.get("download"))
        .and_then(|v| v.as_bool())
        == Some(true);
    if kw_download || explicit_download_tool || explicit_gee_download {
        return true;
    }
    let has_output_path = req
        .output_path
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    let is_explicit_search = req.keyword.as_deref().is_some_and(|k| {
        k.split(['|', ',', ';'])
            .any(|tok| tok.trim().eq_ignore_ascii_case("search"))
    }) || req
        .tool_name
        .as_deref()
        .map(str::trim)
        .is_some_and(|t| t.eq_ignore_ascii_case("search"));
    has_output_path && !is_explicit_search
}

fn noaa_gfs_custom_filters(keyword: Option<&str>) -> Vec<String> {
    keyword
        .map(|kw| {
            kw.split(['|', ',', ';'])
                .map(str::trim)
                .filter(|tok| {
                    !tok.is_empty()
                        && !tok.contains("..")
                        && !matches!(
                            tok.to_ascii_lowercase().as_str(),
                            "gfs"
                                | "goes"
                                | "goes18"
                                | "goes19"
                                | "goes-18"
                                | "goes-19"
                                | "abi"
                                | "subset"
                                | "full"
                                | "search"
                                | "download"
                                | "preview"
                        )
                })
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn overture_export_paths(dest: &Path) -> (PathBuf, PathBuf) {
    let is_geojson_target = dest
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("geojson"));
    if is_geojson_target {
        (dest.with_extension("zstd.parquet"), dest.to_path_buf())
    } else {
        (dest.to_path_buf(), dest.with_extension("geojson"))
    }
}

fn sanitize_aws_filename(raw: &str, fallback: &str) -> String {
    let base = Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(fallback);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.trim_matches('.').is_empty() {
        fallback.to_owned()
    } else {
        cleaned
    }
}

fn aws_resolve_download_dest(
    app_data_dir: &Path,
    req: &OpenGeodataQueryRequest,
    subdir: &str,
    default_filename: &str,
) -> Result<PathBuf, String> {
    let safe_fname = sanitize_aws_filename(default_filename, "aws_data.bin");
    let raw_out = req
        .output_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let dest = match raw_out {
        Some(p_str) => {
            let p = PathBuf::from(p_str);
            let looks_like_file = p.extension().and_then(|s| s.to_str()).is_some_and(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "tif" | "tiff" | "grib2" | "grb2" | "nc" | "parquet" | "geojson" | "json"
                )
            });
            if looks_like_file {
                p
            } else {
                p.join(&safe_fname)
            }
        }
        None => app_data_dir
            .join("geodata_downloads")
            .join(subdir)
            .join(&safe_fname),
    };
    if let Some(parent) = dest.parent() {
        if fs::symlink_metadata(parent)
            .ok()
            .is_some_and(|m| m.file_type().is_symlink())
        {
            return Err("UNSAFE_OUTPUT_DIR: 拒绝写入符号链接目录".to_owned());
        }
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "CREATE_OUTPUT_DIR_FAILED ({}): {e}",
                parent.to_string_lossy()
            )
        })?;
    }
    if fs::symlink_metadata(&dest)
        .ok()
        .is_some_and(|m| m.file_type().is_symlink())
    {
        return Err("UNSAFE_OUTPUT_PATH: 拒绝覆盖符号链接目标文件".to_owned());
    }
    Ok(dest)
}

fn read_prefix_bytes(path: &Path, len: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut f = fs::File::open(path).map_err(|e| format!("READ_FILE_FAILED: {e}"))?;
    let mut buf = vec![0u8; len];
    let n = f
        .read(&mut buf)
        .map_err(|e| format!("READ_MAGIC_FAILED: {e}"))?;
    buf.truncate(n);
    Ok(buf)
}

pub(crate) fn is_valid_tiff_magic(head: &[u8]) -> bool {
    head.starts_with(&[0x49, 0x49, 0x2A, 0x00])
        || head.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
        || head.starts_with(&[0x49, 0x49, 0x2B, 0x00])
        || head.starts_with(&[0x4D, 0x4D, 0x00, 0x2B])
}

pub(crate) fn is_valid_grib_magic(head: &[u8]) -> bool {
    head.starts_with(b"GRIB")
}

pub(crate) fn is_valid_netcdf_or_hdf5_magic(head: &[u8]) -> bool {
    head.starts_with(b"CDF") || head.starts_with(&[0x89, b'H', b'D', b'F', 0x0d, 0x0a, 0x1a, 0x0a])
}

pub(crate) fn validate_parquet_file_magic(path: &Path) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(path).map_err(|e| format!("PARQUET_OPEN_FAILED: {e}"))?;
    let file_len = f
        .metadata()
        .map_err(|e| format!("PARQUET_STAT_FAILED: {e}"))?
        .len();
    if file_len < 12 {
        return Err("INVALID_PARQUET_MAGIC: 文件过小，非有效 Apache Parquet".to_owned());
    }
    let mut head = [0u8; 4];
    f.read_exact(&mut head)
        .map_err(|e| format!("PARQUET_HEAD_READ_FAILED: {e}"))?;
    let mut tail = [0u8; 4];
    f.seek(SeekFrom::End(-4))
        .map_err(|e| format!("PARQUET_TAIL_SEEK_FAILED: {e}"))?;
    f.read_exact(&mut tail)
        .map_err(|e| format!("PARQUET_TAIL_READ_FAILED: {e}"))?;
    if &head != b"PAR1" || &tail != b"PAR1" {
        return Err("INVALID_PARQUET_MAGIC: 缺少 PAR1 头尾魔数，下载可能损坏".to_owned());
    }
    Ok(())
}

pub(crate) fn rewrite_sentinel_s1_s3_to_https(href: &str) -> String {
    let trimmed = href.trim();
    if let Some(rest) = trimmed.strip_prefix("s3://sentinel-s1-l1c/") {
        format!(
            "https://sentinel-s1-l1c.s3.eu-central-1.amazonaws.com/{}",
            rest.trim_start_matches('/')
        )
    } else {
        trimmed.to_owned()
    }
}

fn normalize_stac_rfc3339(raw: Option<&str>, is_end: bool, fallback_days_ago: i64) -> String {
    if let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) {
        if s.contains('T') {
            return if s.ends_with('Z') || s.contains('+') {
                s.to_owned()
            } else {
                format!("{s}Z")
            };
        }
        return if is_end {
            format!("{s}T23:59:59Z")
        } else {
            format!("{s}T00:00:00Z")
        };
    }
    let dt = chrono::Utc::now() - chrono::Duration::days(fallback_days_ago);
    if is_end {
        dt.format("%Y-%m-%dT23:59:59Z").to_string()
    } else {
        dt.format("%Y-%m-%dT00:00:00Z").to_string()
    }
}

fn select_s1_grd_pol_asset(
    assets: &serde_json::Map<String, Value>,
    preferred_pol: Option<&str>,
) -> Option<(String, String, Option<u64>)> {
    let mut order = Vec::with_capacity(4);
    if let Some(pref) = preferred_pol {
        let lower = pref.trim().to_ascii_lowercase();
        if matches!(lower.as_str(), "vv" | "vh" | "hh" | "hv") {
            order.push(lower);
        }
    }
    for cand in ["vv", "vh", "hh", "hv"] {
        if !order.iter().any(|x| x == cand) {
            order.push(cand.to_owned());
        }
    }
    for key in order {
        if let Some(href) = assets
            .get(&key)
            .and_then(|v| v.get("href"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            let size = assets
                .get(&key)
                .and_then(|v| v.get("file:size"))
                .and_then(|v| v.as_u64());
            return Some((
                key.to_ascii_uppercase(),
                rewrite_sentinel_s1_s3_to_https(href),
                size,
            ));
        }
    }
    None
}

fn execute_aws_sentinel1_grd_query(
    app_data_dir: &Path,
    req: &OpenGeodataQueryRequest,
) -> Result<Value, String> {
    let raw_bb = req.bbox.unwrap_or([117.10, 31.70, 117.45, 31.95]);
    let bbox = [
        raw_bb[0].min(raw_bb[2]).clamp(-180.0, 180.0),
        raw_bb[1].min(raw_bb[3]).clamp(-85.0, 85.0),
        raw_bb[0].max(raw_bb[2]).clamp(-180.0, 180.0),
        raw_bb[1].max(raw_bb[3]).clamp(-85.0, 85.0),
    ];
    let start_iso = normalize_stac_rfc3339(req.start_date.as_deref(), false, 180);
    let end_iso = normalize_stac_rfc3339(req.end_date.as_deref(), true, 0);
    let limit = req
        .gee_params
        .as_ref()
        .and_then(|v| v.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .clamp(1, 50);

    let pol_hint = [
        req.dem_product.as_deref(),
        req.keyword.as_deref(),
        req.osm_tag.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find_map(|raw| {
        raw.split(['|', ',', ';', ' '])
            .map(|tok| tok.trim().to_ascii_lowercase())
            .find(|tok| matches!(tok.as_str(), "vh" | "vv" | "hh" | "hv"))
    });

    let should_download = aws_should_download(req);
    let stac_url = "https://earth-search.aws.element84.com/v1/search";
    let body_req = json!({
        "collections": ["sentinel-1-grd"],
        "bbox": bbox,
        "datetime": format!("{start_iso}/{end_iso}"),
        "limit": limit,
    });
    let client = build_http_client_with_timeout(app_data_dir, 45)?;
    let resp = client
        .post(stac_url)
        .json(&body_req)
        .send()
        .map_err(|e| format!("AWS_S1_GRD_SEARCH_FAILED: {e}"))?
        .error_for_status()
        .map_err(|e| format!("AWS_S1_GRD_HTTP_ERROR: {e}"))?;
    let payload: Value = resp
        .json()
        .map_err(|e| format!("AWS_S1_GRD_JSON_FAILED: {e}"))?;
    let matched_count = payload
        .get("numberMatched")
        .or_else(|| payload.get("context").and_then(|c| c.get("matched")))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let features = payload
        .get("features")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut items = Vec::with_capacity(features.len());
    for feat in features {
        let Some(id) = feat.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(assets) = feat.get("assets").and_then(|v| v.as_object()) else {
            continue;
        };
        let Some((pol_label, dl_url, asset_size)) =
            select_s1_grd_pol_asset(assets, pol_hint.as_deref())
        else {
            continue;
        };
        let props = feat.get("properties").and_then(|v| v.as_object());
        let dt = props
            .and_then(|p| p.get("datetime"))
            .and_then(|v| v.as_str())
            .unwrap_or(&end_iso)
            .to_owned();
        let platform = props
            .and_then(|p| p.get("platform"))
            .and_then(|v| v.as_str())
            .unwrap_or("Sentinel-1")
            .to_owned();
        let orbit_state = props
            .and_then(|p| p.get("sat:orbit_state"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let item_bbox = feat
            .get("bbox")
            .and_then(|v| v.as_array())
            .filter(|a| a.len() >= 4)
            .and_then(|a| {
                Some([
                    a[0].as_f64()?,
                    a[1].as_f64()?,
                    a[2].as_f64()?,
                    a[3].as_f64()?,
                ])
            })
            .unwrap_or(bbox);
        let geometry = feat
            .get("geometry")
            .cloned()
            .filter(|g| g.is_object())
            .unwrap_or_else(|| aws_rect_geom(item_bbox));
        let thumb_url = assets
            .get("thumbnail")
            .and_then(|v| v.get("href"))
            .and_then(|v| v.as_str())
            .map(rewrite_sentinel_s1_s3_to_https)
            .unwrap_or_default();
        let preview_data_url: Option<String> = None;
        let fname = format!("{id}_{pol_label}.tif");
        let mut item_json = json!({
            "id": id,
            "title": format!("{id} ({pol_label} COG)"),
            "filename": fname,
            "datetime": dt,
            "provider": "AWS Open Data (Element 84 / Sentinel-1 GRD)",
            "platform": platform,
            "product_type": format!("GRD {pol_label}"),
            "orbit_state": orbit_state,
            "polarization": pol_label,
            "bbox": item_bbox,
            "geometry": geometry,
            "href": dl_url,
            "download_url": dl_url,
            "thumbnail_url": if thumb_url.is_empty() { Value::Null } else { json!(thumb_url) },
            "file_size_remote": asset_size,
        });
        if let Some(data_url) = preview_data_url {
            item_json["preview_data_url"] = json!(data_url);
        }
        items.push(item_json);
    }

    let mut downloaded_file: Option<String> = None;
    let mut downloaded_bytes: u64 = 0;
    let mut target_item_id_out: Option<String> = None;
    let mut already_exists = false;

    if should_download {
        let target_id = req
            .osm_tag
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let target_item = target_id
            .and_then(|tid| items.iter().find(|it| it["id"].as_str() == Some(tid)))
            .or_else(|| items.first())
            .ok_or_else(|| {
                "AWS_S1_GRD_NO_ITEMS: 当前时空范围下未检索到可用 Sentinel-1 GRD COG 条目".to_owned()
            })?;
        let item_id = target_item["id"]
            .as_str()
            .unwrap_or("sentinel1_grd")
            .to_owned();
        let fname = target_item["filename"]
            .as_str()
            .unwrap_or("sentinel1_grd.tif")
            .to_owned();
        let dl_url = target_item["href"]
            .as_str()
            .filter(|s| s.starts_with("https://"))
            .ok_or_else(|| format!("AWS_S1_GRD_INVALID_URL ({item_id})"))?
            .to_owned();
        target_item_id_out = Some(item_id.clone());

        let dest = aws_resolve_download_dest(app_data_dir, req, "AWS_SENTINEL1_GRD", &fname)?;
        if dest.is_file()
            && read_prefix_bytes(&dest, 8)
                .map(|b| is_valid_tiff_magic(&b))
                .unwrap_or(false)
        {
            already_exists = true;
            downloaded_bytes = dest.metadata().map(|m| m.len()).unwrap_or(0);
            downloaded_file = Some(dest.to_string_lossy().to_string());
        } else if req.tool_name.as_deref() == Some("crop") {
            let engine = discover_native_gis_engine();
            if !engine.is_discovered() {
                return Err(
                    "GIS_ENGINE_UNAVAILABLE: 未检测到内置 GDAL 引擎，无法执行 /vsicurl/ 局部裁剪"
                        .to_owned(),
                );
            }
            let crop_req = RasterProcessRequest {
                sources: vec![format!("/vsicurl/{dl_url}")],
                output_path: dest.clone(),
                operation: "clip".to_owned(),
                crop_bbox: Some(bbox),
                ..Default::default()
            };
            engine
                .process_raster_window(&crop_req)
                .map_err(|e| format!("AWS_S1_GRD_CROP_FAILED: {e}"))?;
            let magic = read_prefix_bytes(&dest, 8)?;
            if !is_valid_tiff_magic(&magic) {
                let _ = fs::remove_file(&dest);
                return Err("AWS_S1_GRD_INVALID_TIFF: 裁剪输出非有效 GeoTIFF".to_owned());
            }
            downloaded_bytes = dest.metadata().map(|m| m.len()).unwrap_or(0);
            downloaded_file = Some(dest.to_string_lossy().to_string());
        } else {
            let dl_client = build_http_client_with_timeout(app_data_dir, 300)?;
            let resp = http_get_with_retry(&dl_client, &dl_url, None, 3)
                .map_err(|e| format!("AWS_S1_GRD_DOWNLOAD_FAILED ({dl_url}): {e}"))?
                .error_for_status()
                .map_err(|e| format!("AWS_S1_GRD_HTTP_ERROR ({dl_url}): {e}"))?;
            let tmp_path = dest.with_extension("tif.download.tmp");
            let written = stream_http_to_file_verified(
                resp,
                &tmp_path,
                2 * 1024 * 1024 * 1024,
                &format!("AWS_S1_GRD ({item_id})"),
            )?;
            let head = read_prefix_bytes(&tmp_path, 16)?;
            if !is_valid_tiff_magic(&head) {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!(
                    "AWS_S1_GRD_INVALID_TIFF: {item_id} 响应头非有效 GeoTIFF/BigTIFF 魔数"
                ));
            }
            fs::rename(&tmp_path, &dest).map_err(|e| format!("RENAME_FAILED: {e}"))?;
            downloaded_bytes = written;
            downloaded_file = Some(dest.to_string_lossy().to_string());
        }
    }

    Ok(json!({
        "ok": true,
        "dataset": "aws_sentinel1_grd",
        "provider": "AWS Open Data (Sentinel-1 GRD)",
        "matched_count": matched_count,
        "count": items.len(),
        "already_exists": already_exists,
        "target_item_id": target_item_id_out,
        "downloaded_file": downloaded_file,
        "downloaded_path": downloaded_file,
        "downloaded_bytes": downloaded_bytes,
        "items": items,
    }))
}

pub(crate) fn parse_gfs_idx_ranges(idx_text: &str, filters: &[&str]) -> Vec<GfsIdxRecord> {
    struct RawLine {
        record_no: u64,
        start_byte: u64,
        var: String,
        level: String,
        forecast: String,
        full_tag: String,
    }
    let mut raw_records = Vec::new();
    for line in idx_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split(':').collect();
        if parts.len() < 5 {
            continue;
        }
        let Ok(record_no) = parts[0].trim().parse::<u64>() else {
            continue;
        };
        let Ok(start_byte) = parts[1].trim().parse::<u64>() else {
            continue;
        };
        let var = parts[3].trim().to_owned();
        let level = parts[4].trim().to_owned();
        let forecast = parts.get(5).copied().unwrap_or("").trim().to_owned();
        let full_tag = format!("{var}:{level}:{forecast}");
        raw_records.push(RawLine {
            record_no,
            start_byte,
            var,
            level,
            forecast,
            full_tag,
        });
    }
    raw_records.sort_by_key(|r| r.start_byte);

    let normalized_filters: Vec<String> = filters
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty() && s != "all")
        .collect();
    let keep_all = normalized_filters.is_empty()
        || filters.iter().any(|s| s.trim().eq_ignore_ascii_case("all"));

    let mut out = Vec::new();
    for i in 0..raw_records.len() {
        let rec = &raw_records[i];
        let end_byte_inclusive = raw_records
            .get(i + 1)
            .and_then(|next| next.start_byte.checked_sub(1));
        if !keep_all {
            let tag_lc = rec.full_tag.to_ascii_lowercase();
            let var_level_lc = format!("{}:{}", rec.var, rec.level).to_ascii_lowercase();
            let var_lc = rec.var.to_ascii_lowercase();
            let matches = normalized_filters
                .iter()
                .any(|flt| tag_lc.contains(flt) || var_level_lc.contains(flt) || var_lc == *flt);
            if !matches {
                continue;
            }
        }
        out.push(GfsIdxRecord {
            record_no: rec.record_no,
            start_byte: rec.start_byte,
            end_byte_inclusive,
            var: rec.var.clone(),
            level: rec.level.clone(),
            forecast: rec.forecast.clone(),
        });
    }
    out
}

pub(crate) fn parse_s3_list_objects_xml(xml: &str) -> Vec<(String, u64)> {
    fn extract_tag(block: &str, tag: &str) -> Option<String> {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let start = block.find(&open)? + open.len();
        let end = block[start..].find(&close)?;
        Some(block[start..start + end].trim().to_owned())
    }
    let mut out = Vec::new();
    let mut cursor = xml;
    while let Some(pos) = cursor.find("<Contents>") {
        let after = &cursor[pos + "<Contents>".len()..];
        let Some(end) = after.find("</Contents>") else {
            break;
        };
        let block = &after[..end];
        if let Some(key) = extract_tag(block, "Key") {
            let size = extract_tag(block, "Size")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            out.push((key, size));
        }
        cursor = &after[end + "</Contents>".len()..];
    }
    out
}

fn merge_gfs_byte_ranges(records: &[GfsIdxRecord]) -> Vec<(u64, Option<u64>)> {
    let mut ranges: Vec<(u64, Option<u64>)> = Vec::new();
    for r in records {
        if let Some(last) = ranges.last_mut() {
            if let Some(prev_end) = last.1 {
                if prev_end.saturating_add(1) == r.start_byte {
                    last.1 = r.end_byte_inclusive;
                    continue;
                }
            }
        }
        ranges.push((r.start_byte, r.end_byte_inclusive));
    }
    ranges
}

fn execute_noaa_aws_weather_query(
    app_data_dir: &Path,
    req: &OpenGeodataQueryRequest,
) -> Result<Value, String> {
    use chrono::{Datelike, Timelike};
    let mode_hint = format!(
        "{} {} {}",
        req.dem_product.as_deref().unwrap_or(""),
        req.keyword.as_deref().unwrap_or(""),
        req.dataset
    )
    .to_ascii_lowercase();
    let is_goes = mode_hint.contains("goes") || mode_hint.contains("abi");
    let should_download = aws_should_download(req);

    if is_goes {
        let bucket = if mode_hint.contains("goes18") || mode_hint.contains("goes-18") {
            "noaa-goes18"
        } else {
            "noaa-goes19"
        };
        let product = if mode_hint.contains("mcmipc") {
            "ABI-L2-MCMIPC"
        } else {
            "ABI-L2-CMIPC"
        };
        let client = build_http_client_with_timeout(app_data_dir, 30)?;
        let now_utc = chrono::Utc::now();
        let mut found_objs: Vec<(String, u64, String)> = Vec::new();
        for hours_back in 0..12_i64 {
            let t = now_utc - chrono::Duration::hours(hours_back);
            let prefix = format!(
                "{product}/{:04}/{:03}/{:02}/",
                t.year(),
                t.ordinal(),
                t.hour()
            );
            let list_url = format!("https://{bucket}.s3.amazonaws.com/");
            let Ok(resp) = client
                .get(&list_url)
                .query(&[
                    ("list-type", "2"),
                    ("prefix", prefix.as_str()),
                    ("max-keys", "24"),
                ])
                .send()
            else {
                continue;
            };
            if !resp.status().is_success() {
                continue;
            }
            let Ok(xml) = resp.text() else {
                continue;
            };
            let mut objs: Vec<(String, u64)> = parse_s3_list_objects_xml(&xml)
                .into_iter()
                .filter(|(k, _)| k.ends_with(".nc"))
                .collect();
            if !objs.is_empty() {
                objs.sort_by(|a, b| b.0.cmp(&a.0));
                let dt_iso = t.format("%Y-%m-%dT%H:00:00Z").to_string();
                for (k, sz) in objs.into_iter().take(10) {
                    found_objs.push((k, sz, dt_iso.clone()));
                }
                break;
            }
        }
        if found_objs.is_empty() {
            return Err(format!(
                "NOAA_GOES_NO_RECENT_SCANS: 最近 12 小时在 {bucket}/{product} 未检索到 .nc 扫描文件"
            ));
        }

        let goes_bb = [-135.0, 15.0, -55.0, 55.0];
        let items: Vec<Value> = found_objs
            .iter()
            .map(|(key, size, dt)| {
                let fname = Path::new(key)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("goes.nc")
                    .to_owned();
                let item_id = Path::new(&fname)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&fname)
                    .to_owned();
                let dl_url = format!("https://{bucket}.s3.amazonaws.com/{key}");
                json!({
                    "id": item_id,
                    "title": format!("{bucket} {product} ({fname})"),
                    "filename": fname,
                    "datetime": dt,
                    "provider": "NOAA AWS Open Data (GOES ABI)",
                    "platform": bucket,
                    "product_type": product,
                    "bbox": goes_bb,
                    "geometry": aws_rect_geom(goes_bb),
                    "href": dl_url,
                    "download_url": dl_url,
                    "file_size_remote": size,
                })
            })
            .collect();

        let mut downloaded_file: Option<String> = None;
        let mut downloaded_bytes: u64 = 0;
        let mut target_item_id_out: Option<String> = None;
        let mut already_exists = false;

        if should_download {
            let target_id = req
                .osm_tag
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let target_item = target_id
                .and_then(|tid| items.iter().find(|it| it["id"].as_str() == Some(tid)))
                .or_else(|| items.first())
                .ok_or_else(|| "NOAA_GOES_NO_ITEMS".to_owned())?;
            let item_id = target_item["id"].as_str().unwrap_or("goes").to_owned();
            let fname = target_item["filename"].as_str().unwrap_or("goes.nc");
            let dl_url = target_item["download_url"].as_str().unwrap_or("");
            target_item_id_out = Some(item_id.clone());

            let dest = aws_resolve_download_dest(app_data_dir, req, "NOAA_AWS_WEATHER", fname)?;
            if dest.is_file()
                && read_prefix_bytes(&dest, 8)
                    .map(|b| is_valid_netcdf_or_hdf5_magic(&b))
                    .unwrap_or(false)
            {
                already_exists = true;
                downloaded_bytes = dest.metadata().map(|m| m.len()).unwrap_or(0);
                downloaded_file = Some(dest.to_string_lossy().to_string());
            } else {
                let dl_client = build_http_client_with_timeout(app_data_dir, 180)?;
                let resp = http_get_with_retry(&dl_client, dl_url, None, 3)
                    .map_err(|e| format!("NOAA_GOES_DOWNLOAD_FAILED: {e}"))?
                    .error_for_status()
                    .map_err(|e| format!("NOAA_GOES_HTTP_ERROR: {e}"))?;
                let tmp_path = dest.with_extension("nc.download.tmp");
                let written = stream_http_to_file_verified(
                    resp,
                    &tmp_path,
                    512 * 1024 * 1024,
                    &format!("NOAA_GOES ({item_id})"),
                )?;
                let head = read_prefix_bytes(&tmp_path, 16)?;
                if !is_valid_netcdf_or_hdf5_magic(&head) {
                    let _ = fs::remove_file(&tmp_path);
                    return Err(format!(
                        "NOAA_GOES_INVALID_NETCDF: {item_id} 响应非有效 NetCDF/HDF5 魔数"
                    ));
                }
                fs::rename(&tmp_path, &dest).map_err(|e| format!("RENAME_FAILED: {e}"))?;
                downloaded_bytes = written;
                downloaded_file = Some(dest.to_string_lossy().to_string());
            }
        }

        return Ok(json!({
            "ok": true,
            "dataset": "noaa_aws_weather",
            "mode": "goes",
            "provider": "NOAA AWS Open Data (GOES ABI)",
            "count": items.len(),
            "already_exists": already_exists,
            "target_item_id": target_item_id_out,
            "downloaded_file": downloaded_file,
            "downloaded_path": downloaded_file,
            "downloaded_bytes": downloaded_bytes,
            "items": items,
        }));
    }

    // Default GFS 0.25-deg model with cycle walk-back + .idx byte-range subsetting
    let max_utc = chrono::Utc::now() - chrono::Duration::hours(4);
    let start_cursor = req
        .end_date
        .as_deref()
        .or(req.start_date.as_deref())
        .and_then(|s| chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(18, 0, 0))
        .map(|ndt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(ndt, chrono::Utc))
        .map(|dt| dt.min(max_utc))
        .unwrap_or(max_utc);
    let aligned_hour = (start_cursor.hour() / 6) * 6;
    let base_dt = start_cursor
        .date_naive()
        .and_hms_opt(aligned_hour, 0, 0)
        .map(|ndt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(ndt, chrono::Utc))
        .unwrap_or(start_cursor);

    let client = build_http_client_with_timeout(app_data_dir, 20)?;
    let mut resolved_cycle: Option<(chrono::DateTime<chrono::Utc>, String, String, String)> = None;
    for step in 0..8_i64 {
        let cand_dt = base_dt - chrono::Duration::hours(step * 6);
        let ymd = cand_dt.format("%Y%m%d").to_string();
        let cc = format!("{:02}", cand_dt.hour());
        let idx_url = format!(
            "https://noaa-gfs-bdp-pds.s3.amazonaws.com/gfs.{ymd}/{cc}/atmos/gfs.t{cc}z.pgrb2.0p25.f000.idx"
        );
        let Ok(resp) = client.get(&idx_url).send() else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(idx_text) = resp.text() else {
            continue;
        };
        if idx_text.contains(':') && !idx_text.trim().is_empty() {
            resolved_cycle = Some((cand_dt, ymd, cc, idx_text));
            break;
        }
    }
    let (cycle_dt, ymd, cc, f000_idx_text) = resolved_cycle.ok_or_else(|| {
        "NOAA_GFS_CYCLE_NOT_FOUND: 最近 48 小时未检索到可用的 NOAA GFS 0.25° 预报周期 (.idx)"
            .to_owned()
    })?;

    let custom_filters = noaa_gfs_custom_filters(req.keyword.as_deref());
    let default_filters = [
        "TMP:2 m above ground",
        "UGRD:10 m above ground",
        "VGRD:10 m above ground",
        "PRMSL:mean sea level",
        "APCP:surface",
    ];
    let filter_refs: Vec<&str> = if custom_filters.is_empty() {
        default_filters.to_vec()
    } else {
        custom_filters.iter().map(String::as_str).collect()
    };

    let f000_matched = parse_gfs_idx_ranges(&f000_idx_text, &filter_refs);
    let est_subset_bytes: u64 = f000_matched
        .iter()
        .map(|r| {
            r.end_byte_inclusive
                .map(|e| e.saturating_sub(r.start_byte) + 1)
                .unwrap_or(500_000)
        })
        .sum();
    let display_bb = req.bbox.unwrap_or([-180.0, -85.0, 180.0, 85.0]);
    let cycle_iso = cycle_dt.format("%Y-%m-%dT%H:00:00Z").to_string();
    let items: Vec<Value> = ["f000", "f006", "f012", "f024"]
        .into_iter()
        .map(|fhr| {
            let item_id = format!("GFS_{ymd}_t{cc}z_{fhr}");
            let base_file = format!(
                "https://noaa-gfs-bdp-pds.s3.amazonaws.com/gfs.{ymd}/{cc}/atmos/gfs.t{cc}z.pgrb2.0p25.{fhr}"
            );
            let full_dl = req.tool_name.as_deref() == Some("full");
            let fname = if full_dl {
                format!("{item_id}_full.grib2")
            } else {
                format!("{item_id}_subset.grib2")
            };
            json!({
                "id": item_id,
                "title": format!("NOAA GFS 0.25° {ymd} {cc}Z ({fhr})"),
                "filename": fname,
                "forecast_hour": fhr,
                "datetime": cycle_iso,
                "provider": "NOAA AWS Open Data (GFS 0.25°)",
                "platform": "GFS-0.25deg",
                "product_type": if full_dl { "GRIB2 Full" } else { "GRIB2 .idx Byte-Range Subset" },
                "bbox": display_bb,
                "geometry": aws_rect_geom(display_bb),
                "href": base_file,
                "download_url": base_file,
                "idx_url": format!("{base_file}.idx"),
                "matched_records": f000_matched.len(),
                "file_size_remote": if full_dl { Value::Null } else { json!(est_subset_bytes) },
            })
        })
        .collect();

    let mut downloaded_file: Option<String> = None;
    let mut downloaded_bytes: u64 = 0;
    let mut target_item_id_out: Option<String> = None;
    let mut already_exists = false;
    let mut downloaded_var_count = 0usize;

    if should_download {
        let target_id = req
            .osm_tag
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let target_item = target_id
            .and_then(|tid| items.iter().find(|it| it["id"].as_str() == Some(tid)))
            .or_else(|| items.first())
            .ok_or_else(|| "NOAA_GFS_NO_ITEMS".to_owned())?;
        let item_id = target_item["id"].as_str().unwrap_or("gfs").to_owned();
        let fhr = target_item["forecast_hour"].as_str().unwrap_or("f000");
        let fname = target_item["filename"].as_str().unwrap_or("gfs.grib2");
        let grib_url = target_item["download_url"].as_str().unwrap_or("");
        let idx_url = target_item["idx_url"].as_str().unwrap_or("");
        target_item_id_out = Some(item_id.clone());

        let dest = aws_resolve_download_dest(app_data_dir, req, "NOAA_AWS_WEATHER", fname)?;
        if dest.is_file()
            && read_prefix_bytes(&dest, 8)
                .map(|b| is_valid_grib_magic(&b))
                .unwrap_or(false)
        {
            already_exists = true;
            downloaded_bytes = dest.metadata().map(|m| m.len()).unwrap_or(0);
            downloaded_file = Some(dest.to_string_lossy().to_string());
        } else if req.tool_name.as_deref() == Some("full") {
            let dl_client = build_http_client_with_timeout(app_data_dir, 240)?;
            let resp = http_get_with_retry(&dl_client, grib_url, None, 3)
                .map_err(|e| format!("NOAA_GFS_DOWNLOAD_FAILED: {e}"))?
                .error_for_status()
                .map_err(|e| format!("NOAA_GFS_HTTP_ERROR: {e}"))?;
            let tmp_path = dest.with_extension("grib2.download.tmp");
            let written = stream_http_to_file_verified(
                resp,
                &tmp_path,
                1024 * 1024 * 1024,
                &format!("NOAA_GFS ({item_id})"),
            )?;
            let head = read_prefix_bytes(&tmp_path, 8)?;
            if !is_valid_grib_magic(&head) {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!("NOAA_GFS_INVALID_GRIB2: {item_id} 缺少 GRIB 魔数"));
            }
            fs::rename(&tmp_path, &dest).map_err(|e| format!("RENAME_FAILED: {e}"))?;
            downloaded_bytes = written;
            downloaded_file = Some(dest.to_string_lossy().to_string());
        } else {
            let dl_client = build_http_client_with_timeout(app_data_dir, 120)?;
            let step_idx_text = if fhr == "f000" {
                f000_idx_text
            } else {
                http_get_with_retry(&dl_client, idx_url, None, 3)
                    .map_err(|e| format!("NOAA_GFS_IDX_FAILED: {e}"))?
                    .error_for_status()
                    .map_err(|e| format!("NOAA_GFS_IDX_HTTP_ERROR: {e}"))?
                    .text()
                    .map_err(|e| format!("NOAA_GFS_IDX_TEXT_FAILED: {e}"))?
            };
            let matched = parse_gfs_idx_ranges(&step_idx_text, &filter_refs);
            if matched.is_empty() {
                return Err(
                    "NOAA_GFS_IDX_NO_MATCH: .idx 索引中未匹配到指定气象变量，请检查 keyword 筛选条件"
                        .to_owned(),
                );
            }
            downloaded_var_count = matched.len();
            let merged = merge_gfs_byte_ranges(&matched);
            let tmp_path = dest.with_extension("grib2.download.tmp");
            if let Some(parent) = tmp_path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("CREATE_DIR_FAILED: {e}"))?;
            }
            use std::io::Write;
            let mut out_f =
                fs::File::create(&tmp_path).map_err(|e| format!("CREATE_TMP_FAILED: {e}"))?;
            let max_bytes: u64 = 512 * 1024 * 1024;
            let mut total_written: u64 = 0;
            for (start, end_opt) in merged {
                let range_hdr = match end_opt {
                    Some(end_b) => format!("bytes={start}-{end_b}"),
                    None => format!("bytes={start}-"),
                };
                let mut resp = dl_client
                    .get(grib_url)
                    .header(reqwest::header::RANGE, &range_hdr)
                    .send()
                    .map_err(|e| {
                        let _ = fs::remove_file(&tmp_path);
                        format!("NOAA_GFS_RANGE_FAILED ({range_hdr}): {e}")
                    })?;
                if !(resp.status().is_success() || resp.status().as_u16() == 206) {
                    let st = resp.status();
                    let _ = fs::remove_file(&tmp_path);
                    return Err(format!(
                        "NOAA_GFS_RANGE_HTTP_ERROR ({range_hdr}): HTTP {st}"
                    ));
                }
                let mut chunk = Vec::new();
                std::io::copy(&mut resp, &mut chunk).map_err(|e| {
                    let _ = fs::remove_file(&tmp_path);
                    format!("NOAA_GFS_RANGE_READ_FAILED: {e}")
                })?;
                total_written += chunk.len() as u64;
                if total_written > max_bytes {
                    let _ = fs::remove_file(&tmp_path);
                    return Err("NOAA_GFS_SUBSET_TOO_LARGE: 裁剪子集超过 512 MiB 上限".to_owned());
                }
                out_f.write_all(&chunk).map_err(|e| {
                    let _ = fs::remove_file(&tmp_path);
                    format!("NOAA_GFS_WRITE_FAILED: {e}")
                })?;
            }
            out_f
                .flush()
                .map_err(|e| format!("NOAA_GFS_FLUSH_FAILED: {e}"))?;
            drop(out_f);
            let head = read_prefix_bytes(&tmp_path, 8)?;
            if !is_valid_grib_magic(&head) {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!(
                    "NOAA_GFS_INVALID_GRIB2: {item_id} Range 子集缺少 GRIB 魔数"
                ));
            }
            fs::rename(&tmp_path, &dest).map_err(|e| format!("RENAME_FAILED: {e}"))?;
            downloaded_bytes = total_written;
            downloaded_file = Some(dest.to_string_lossy().to_string());
        }
    }

    Ok(json!({
        "ok": true,
        "dataset": "noaa_aws_weather",
        "mode": "gfs",
        "cycle": format!("{ymd}_{cc}z"),
        "provider": "NOAA AWS Open Data (GFS 0.25°)",
        "count": items.len(),
        "matched_records": downloaded_var_count.max(f000_matched.len()),
        "already_exists": already_exists,
        "target_item_id": target_item_id_out,
        "downloaded_file": downloaded_file,
        "downloaded_path": downloaded_file,
        "downloaded_bytes": downloaded_bytes,
        "items": items,
    }))
}

pub(crate) fn overture_intersecting_item_indices(
    collection: &Value,
    bbox: [f64; 4],
) -> Vec<(usize, [f64; 4])> {
    let q = [
        bbox[0].min(bbox[2]),
        bbox[1].min(bbox[3]),
        bbox[0].max(bbox[2]),
        bbox[1].max(bbox[3]),
    ];
    let Some(bboxes) = collection
        .get("extent")
        .and_then(|v| v.get("spatial"))
        .and_then(|v| v.get("bbox"))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    let item_count = collection
        .get("links")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|l| l.get("rel").and_then(|r| r.as_str()) == Some("item"))
                .count()
        })
        .unwrap_or(0);

    let parse_bb = |entry: &Value| -> Option<[f64; 4]> {
        let a = entry.as_array()?;
        if a.len() < 4 {
            return None;
        }
        Some([
            a[0].as_f64()?,
            a[1].as_f64()?,
            a[2].as_f64()?,
            a[3].as_f64()?,
        ])
    };
    let intersects =
        |p: [f64; 4]| -> bool { !(p[2] < q[0] || p[0] > q[2] || p[3] < q[1] || p[1] > q[3]) };

    let mut out = Vec::new();
    if bboxes.len() > 1 {
        for (i, entry) in bboxes.iter().enumerate().skip(1) {
            let item_idx = i - 1;
            if item_count > 0 && item_idx >= item_count {
                break;
            }
            if let Some(part_bb) = parse_bb(entry) {
                if intersects(part_bb) {
                    out.push((item_idx, part_bb));
                    if out.len() >= 12 {
                        break;
                    }
                }
            }
        }
    } else if bboxes.len() == 1 && item_count > 0 {
        if let Some(part_bb) = parse_bb(&bboxes[0]) {
            if intersects(part_bb) {
                out.push((0, part_bb));
            }
        }
    }
    out
}

fn resolve_overture_theme_and_type(req: &OpenGeodataQueryRequest) -> (&'static str, &'static str) {
    let hint = [req.dem_product.as_deref(), req.keyword.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if hint.contains("place") {
        ("places", "place")
    } else if hint.contains("transport") || hint.contains("road") || hint.contains("segment") {
        ("transportation", "segment")
    } else if hint.contains("division") || hint.contains("admin") {
        ("divisions", "division")
    } else if hint.contains("address") {
        ("addresses", "address")
    } else if hint.contains("base") || hint.contains("land") || hint.contains("water") {
        ("base", "water")
    } else {
        ("buildings", "building")
    }
}

fn execute_overture_maps_query(
    app_data_dir: &Path,
    req: &OpenGeodataQueryRequest,
) -> Result<Value, String> {
    let raw_bb = req.bbox.unwrap_or([117.10, 31.70, 117.45, 31.95]);
    let bbox = [
        raw_bb[0].min(raw_bb[2]).clamp(-180.0, 180.0),
        raw_bb[1].min(raw_bb[3]).clamp(-85.0, 85.0),
        raw_bb[0].max(raw_bb[2]).clamp(-180.0, 180.0),
        raw_bb[1].max(raw_bb[3]).clamp(-85.0, 85.0),
    ];
    let (theme, typ) = resolve_overture_theme_and_type(req);
    let client = build_http_client_with_timeout(app_data_dir, 45)?;

    let root_catalog: Value = client
        .get("https://stac.overturemaps.org/catalog.json")
        .send()
        .map_err(|e| format!("OVERTURE_CATALOG_FAILED: {e}"))?
        .error_for_status()
        .map_err(|e| format!("OVERTURE_CATALOG_HTTP_ERROR: {e}"))?
        .json()
        .map_err(|e| format!("OVERTURE_CATALOG_JSON_FAILED: {e}"))?;
    let latest_release = root_catalog
        .get("latest")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "OVERTURE_NO_LATEST_RELEASE: catalog.json 缺少 latest 版本字段".to_owned())?
        .to_owned();

    let col_url =
        format!("https://stac.overturemaps.org/{latest_release}/{theme}/{typ}/collection.json");
    let collection: Value = client
        .get(&col_url)
        .send()
        .map_err(|e| format!("OVERTURE_COLLECTION_FAILED ({col_url}): {e}"))?
        .error_for_status()
        .map_err(|e| format!("OVERTURE_COLLECTION_HTTP_ERROR ({col_url}): {e}"))?
        .json()
        .map_err(|e| format!("OVERTURE_COLLECTION_JSON_FAILED: {e}"))?;

    let item_links: Vec<String> = collection
        .get("links")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|l| l.get("rel").and_then(|r| r.as_str()) == Some("item"))
                .filter_map(|l| l.get("href").and_then(|h| h.as_str()))
                .map(|href| {
                    if href.starts_with("https://") {
                        href.to_owned()
                    } else {
                        format!(
                            "https://stac.overturemaps.org/{latest_release}/{theme}/{typ}/{}",
                            href.trim_start_matches("./")
                        )
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let matches = overture_intersecting_item_indices(&collection, bbox);
    let mut items = Vec::new();
    for (item_idx, part_bb) in matches.into_iter().take(8) {
        let Some(item_url) = item_links.get(item_idx) else {
            continue;
        };
        if !item_url.starts_with("https://stac.overturemaps.org/") {
            continue;
        }
        let Ok(resp) = client.get(item_url).send() else {
            continue;
        };
        let Ok(ok_resp) = resp.error_for_status() else {
            continue;
        };
        let Ok(stac_item) = ok_resp.json::<Value>() else {
            continue;
        };
        let Some(aws_asset) = stac_item.get("assets").and_then(|a| a.get("aws")) else {
            continue;
        };
        let Some(aws_href) = aws_asset.get("href").and_then(|h| h.as_str()) else {
            continue;
        };
        if !aws_href.starts_with("https://overturemaps-us-west-2.s3.") {
            continue;
        }
        let file_size = aws_asset.get("file:size").and_then(|v| v.as_u64());
        let fname = Path::new(aws_href)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("overture.zstd.parquet")
            .to_owned();
        let part_id = stac_item
            .get("id")
            .and_then(|v| v.as_str())
            .map(|id| format!("overture_{latest_release}_{theme}_{typ}_{id}"))
            .unwrap_or_else(|| format!("overture_{latest_release}_{theme}_{typ}_{item_idx:05}"));
        let date_prefix = latest_release
            .split('.')
            .next()
            .filter(|d| d.len() == 10)
            .unwrap_or("2026-01-01");
        items.push(json!({
            "id": part_id,
            "title": format!("Overture {latest_release} {theme}/{typ} ({fname})"),
            "filename": fname,
            "datetime": format!("{date_prefix}T00:00:00Z"),
            "provider": "Overture Maps Foundation (AWS S3)",
            "platform": format!("Overture {latest_release}"),
            "product_type": format!("{theme}/{typ} GeoParquet"),
            "bbox": part_bb,
            "geometry": aws_rect_geom(part_bb),
            "href": aws_href,
            "download_url": aws_href,
            "file_size_remote": file_size,
            "overture_release": latest_release,
        }));
    }

    let should_download = aws_should_download(req);
    let mut downloaded_file: Option<String> = None;
    let mut downloaded_bytes: u64 = 0;
    let mut target_item_id_out: Option<String> = None;
    let mut already_exists = false;

    if should_download {
        let target_id = req
            .osm_tag
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let target_item = target_id
            .and_then(|tid| items.iter().find(|it| it["id"].as_str() == Some(tid)))
            .or_else(|| items.first())
            .ok_or_else(|| {
                "OVERTURE_NO_INTERSECTING_PARTITIONS: 当前区域未命中 Overture GeoParquet 分片"
                    .to_owned()
            })?;
        let item_id = target_item["id"].as_str().unwrap_or("overture").to_owned();
        let fname = target_item["filename"]
            .as_str()
            .unwrap_or("overture.zstd.parquet");
        let aws_href = target_item["download_url"]
            .as_str()
            .filter(|s| s.starts_with("https://overturemaps-us-west-2.s3."))
            .ok_or_else(|| format!("OVERTURE_INVALID_URL ({item_id})"))?;
        target_item_id_out = Some(item_id.clone());

        let raw_dest = aws_resolve_download_dest(app_data_dir, req, "OVERTURE_MAPS", fname)?;
        let (dest_parquet, geojson_dest) = if req.tool_name.as_deref() == Some("export_geojson") {
            overture_export_paths(&raw_dest)
        } else {
            let ext = raw_dest.with_extension("geojson");
            (raw_dest, ext)
        };
        let have_valid_parquet =
            dest_parquet.is_file() && validate_parquet_file_magic(&dest_parquet).is_ok();
        if !have_valid_parquet {
            let dl_client = build_http_client_with_timeout(app_data_dir, 300)?;
            let resp = http_get_with_retry(&dl_client, aws_href, None, 3)
                .map_err(|e| format!("OVERTURE_DOWNLOAD_FAILED: {e}"))?
                .error_for_status()
                .map_err(|e| format!("OVERTURE_HTTP_ERROR: {e}"))?;
            let tmp_path = dest_parquet.with_extension("parquet.download.tmp");
            let written = stream_http_to_file_verified(
                resp,
                &tmp_path,
                2 * 1024 * 1024 * 1024,
                &format!("OVERTURE ({item_id})"),
            )?;
            if let Err(err) = validate_parquet_file_magic(&tmp_path) {
                let _ = fs::remove_file(&tmp_path);
                return Err(err);
            }
            fs::rename(&tmp_path, &dest_parquet).map_err(|e| format!("RENAME_FAILED: {e}"))?;
            downloaded_bytes = written;
        } else {
            already_exists = true;
            downloaded_bytes = dest_parquet.metadata().map(|m| m.len()).unwrap_or(0);
        }

        if req.tool_name.as_deref() == Some("export_geojson") {
            let rt_root = crate::application::discover_runtime_root();
            let exe = crate::application::resolve_component_binary(
                &rt_root.join("runtime/duckdb-cli"),
                &["duckdb"],
            )
            .ok_or_else(|| {
                format!(
                    "DUCKDB_NOT_INSTALLED: 未找到 {}，无法从 Parquet 导出区域 GeoJSON",
                    rt_root.join("runtime/duckdb-cli/duckdb").display()
                )
            })?;
            let spatial_ext = rt_root.join("runtime/duckdb-cli/ext/spatial.duckdb_extension");
            let parquet_esc = dest_parquet.to_string_lossy().replace('\'', "''");
            let geo_esc = geojson_dest.to_string_lossy().replace('\'', "''");
            let mut sql = String::new();
            if spatial_ext.is_file() {
                let ext_esc = spatial_ext
                    .to_string_lossy()
                    .replace('\\', "/")
                    .replace('\'', "''");
                sql.push_str(&format!("LOAD '{ext_esc}'; "));
            }
            sql.push_str(&format!(
                "COPY (SELECT id, bbox, geometry FROM read_parquet('{parquet_esc}') WHERE bbox.xmin <= {} AND bbox.xmax >= {} AND bbox.ymin <= {} AND bbox.ymax >= {} LIMIT 50000) TO '{geo_esc}' (FORMAT GDAL, DRIVER 'GeoJSON');",
                bbox[2], bbox[0], bbox[3], bbox[1]
            ));
            let out = std::process::Command::new(&exe)
                .arg("-unsigned")
                .arg("-c")
                .arg(&sql)
                .output()
                .map_err(|e| format!("OVERTURE_DUCKDB_EXPORT_FAILED: {e}"))?;
            if !out.status.success() || !geojson_dest.is_file() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(format!("OVERTURE_DUCKDB_EXPORT_ERROR: {stderr}"));
            }
            downloaded_bytes = geojson_dest.metadata().map(|m| m.len()).unwrap_or(0);
            downloaded_file = Some(geojson_dest.to_string_lossy().to_string());
        } else {
            downloaded_file = Some(dest_parquet.to_string_lossy().to_string());
        }
    }

    Ok(json!({
        "ok": true,
        "dataset": "overture_maps",
        "release": latest_release,
        "theme": theme,
        "type": typ,
        "provider": "Overture Maps Foundation (AWS S3)",
        "count": items.len(),
        "already_exists": already_exists,
        "target_item_id": target_item_id_out,
        "downloaded_file": downloaded_file,
        "downloaded_path": downloaded_file,
        "downloaded_bytes": downloaded_bytes,
        "items": items,
    }))
}

#[cfg(test)]
mod aws_open_data_tests {
    use super::*;

    #[test]
    fn aws_sentinel1_s3_uri_rewrites_to_eu_central_https() {
        assert_eq!(
            rewrite_sentinel_s1_s3_to_https(
                "s3://sentinel-s1-l1c/GRD/2026/11/12/IW/DV/sample/measurement/iw-vv.tiff"
            ),
            "https://sentinel-s1-l1c.s3.eu-central-1.amazonaws.com/GRD/2026/11/12/IW/DV/sample/measurement/iw-vv.tiff"
        );
        assert_eq!(
            rewrite_sentinel_s1_s3_to_https("https://example.invalid/a.tif"),
            "https://example.invalid/a.tif"
        );
    }

    #[test]
    fn noaa_gfs_idx_parser_extracts_exact_byte_ranges_and_handles_eof_record() {
        let sample_idx = "\
1:0:d=2026112000:PRMSL:mean sea level:anl:
2:871269:d=2026112000:CLMR:1 hybrid level:anl:
3:961394:d=2026112000:TMP:2 m above ground:anl:
4:1562421:d=2026112000:UGRD:10 m above ground:anl:
";
        let matched = parse_gfs_idx_ranges(
            sample_idx,
            &["TMP:2 m above ground", "UGRD:10 m above ground"],
        );
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].record_no, 3);
        assert_eq!(matched[0].start_byte, 961394);
        assert_eq!(matched[0].end_byte_inclusive, Some(1562420));
        assert_eq!(matched[1].record_no, 4);
        assert_eq!(matched[1].start_byte, 1562421);
        assert_eq!(matched[1].end_byte_inclusive, None);
        let merged = merge_gfs_byte_ranges(&matched);
        assert_eq!(merged, vec![(961394, None)]);
    }

    #[test]
    fn noaa_s3_list_xml_parser_extracts_keys_and_sizes() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Contents><Key>ABI-L2-CMIPC/2026/324/12/OR_ABI-L2-CMIPC-M6C02_G19_s20263241201172.nc</Key><Size>1234567</Size></Contents>
  <Contents><Key>ABI-L2-CMIPC/2026/324/12/OR_ABI-L2-CMIPC-M6C13_G19_s20263241201172.nc</Key><Size>891011</Size></Contents>
</ListBucketResult>"#;
        let parsed = parse_s3_list_objects_xml(xml);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].1, 1234567);
        assert!(parsed[1].0.ends_with(".nc"));
        assert_eq!(parsed[1].1, 891011);
    }

    #[test]
    fn overture_collection_bbox_intersection_maps_partition_indices() {
        let col = json!({
            "extent": {
                "spatial": {
                    "bbox": [
                        [-180.0, -85.0, 180.0, 85.0],
                        [-180.0, -85.0, 0.0, 0.0],
                        [116.0, 31.0, 118.0, 33.0]
                    ]
                }
            },
            "links": [
                {"rel": "self", "href": "https://stac.overturemaps.org/col.json"},
                {"rel": "item", "href": "https://stac.overturemaps.org/00000.json"},
                {"rel": "item", "href": "https://stac.overturemaps.org/00001.json"}
            ]
        });
        let hits = overture_intersecting_item_indices(&col, [117.1, 31.7, 117.4, 31.9]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 1);
    }

    #[test]
    fn aws_open_data_magic_validators_check_tiff_grib_netcdf_parquet() {
        assert!(is_valid_tiff_magic(&[0x49, 0x49, 0x2A, 0x00, 0x10]));
        assert!(is_valid_tiff_magic(&[0x4D, 0x4D, 0x00, 0x2B]));
        assert!(!is_valid_tiff_magic(b"<html>error</html>"));
        assert!(is_valid_grib_magic(b"GRIB\x00\x00\x00\x02"));
        assert!(!is_valid_grib_magic(b"NOTGRIB"));
        assert!(is_valid_netcdf_or_hdf5_magic(b"CDF\x01\x00"));
        assert!(is_valid_netcdf_or_hdf5_magic(&[
            0x89, b'H', b'D', b'F', 0x0d, 0x0a, 0x1a, 0x0a
        ]));
        assert!(!is_valid_netcdf_or_hdf5_magic(
            b"<Error>AccessDenied</Error>"
        ));

        let tmp_dir =
            std::env::temp_dir().join(format!("eostage-aws-magic-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp_dir);
        let good_pq = tmp_dir.join("good.parquet");
        let bad_pq = tmp_dir.join("bad.parquet");
        fs::write(&good_pq, b"PAR1payloadPAR1").unwrap();
        fs::write(&bad_pq, b"PAR1payloadXXXX").unwrap();
        assert!(validate_parquet_file_magic(&good_pq).is_ok());
        assert!(validate_parquet_file_magic(&bad_pq).is_err());
        let _ = fs::remove_dir_all(&tmp_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insar_gis::build_synthetic_mapgis_binary_wl;

    static GEE_TEST_KEYRING_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct GeeKeyringTestGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    #[test]
    fn test_scan_local_slc_dates_parses_asf_py_script_and_extracts_scenes() {
        let dir = std::env::temp_dir().join(format!("eostage-asf-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let py_path = dir.join("download-all-2026-06-01.py");
        fs::write(
            &py_path,
            "# ASF Bulk Download Script\n\
files = [\n\
  'https://datapool.asf.alaska.edu/SLC/SA/S1A_IW_SLC__1SDV_20250401T101234_20250401T101301_058565_073F42_1A2B.zip',\n\
  'https://datapool.asf.alaska.edu/SLC/SB/S1B_IW_SLC__1SDV_20250413T101235_20250413T101302_047710_05BB11_3C4D.SAFE',\n\
]\n",
        )
        .expect("write py script");
        let res = scan_local_slc_dates_in_directory(&py_path).expect("scan script");
        assert_eq!(res["ok"], json!(true));
        assert_eq!(res["dates"], json!(["20250401", "20250413"]));
        assert_eq!(res["dominant_utc_hour"], json!("10"));
        assert_eq!(res["dominant_utc_minute"], json!("12"));
        assert_eq!(
            res["s1_scenes"],
            json!([
                "S1A_IW_SLC__1SDV_20250401T101234_20250401T101301_058565_073F42_1A2B",
                "S1B_IW_SLC__1SDV_20250413T101235_20250413T101302_047710_05BB11_3C4D"
            ])
        );
        let _ = fs::remove_dir_all(&dir);
    }

    impl GeeKeyringTestGuard {
        fn acquire() -> Self {
            let lock = GEE_TEST_KEYRING_MUTEX
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            clear_gee_keyring_bundle();
            Self { _lock: lock }
        }
    }

    impl Drop for GeeKeyringTestGuard {
        fn drop(&mut self) {
            clear_gee_keyring_bundle();
        }
    }

    fn accept_with_timeout(
        listener: &std::net::TcpListener,
        timeout: Duration,
    ) -> Option<std::net::TcpStream> {
        let _ = listener.set_nonblocking(true);
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    return Some(stream);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return None,
            }
        }
        None
    }

    struct LoopbackMpcServer {
        base_url: String,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl LoopbackMpcServer {
        fn spawn(work_dir: &Path) -> Self {
            use std::io::{Read, Write};
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::Arc;

            let one_px_png: Vec<u8> = vec![
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
                0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
                0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
                0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
                0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
            ];
            let seed_png_path = work_dir.join("_loopback_seed_tile.png");
            fs::write(&seed_png_path, &one_px_png).expect("write loopback seed png");
            let sample_tif_path = work_dir.join("_loopback_sample_cog.tif");
            let engine = discover_native_gis_engine();
            let sample_tif_bytes = if engine.is_discovered() {
                let wgs84_wkt = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]"#;
                engine
                    .stitch_xyz_tiles_to_geotiff(
                        &[(0, 0, seed_png_path.clone())],
                        0,
                        0,
                        0,
                        0,
                        [116.0, 3.0 / 256.0, 0.0, 33.0, 0.0, -(3.0 / 256.0)],
                        wgs84_wkt,
                        &sample_tif_path,
                        "DEFLATE",
                        false,
                        "gtiff",
                    )
                    .expect("create loopback sample GeoTIFF");
                fs::read(&sample_tif_path).expect("read loopback sample GeoTIFF")
            } else {
                let mut fallback = b"II*\0\x08\0\0\0".to_vec();
                fallback.resize(512, 0x42);
                fallback
            };

            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback MPC");
            listener
                .set_nonblocking(true)
                .expect("nonblocking MPC listener");
            let addr = listener.local_addr().expect("loopback addr");
            let base_url = format!("http://{addr}");
            let base_for_srv = base_url.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stop_bg = Arc::clone(&stop);

            let handle = std::thread::spawn(move || {
                while !stop_bg.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let mut data = Vec::new();
                            let mut buf = [0u8; 4096];
                            let mut header_end = None;
                            let mut content_len = 0usize;
                            loop {
                                match stream.read(&mut buf) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        data.extend_from_slice(&buf[..n]);
                                        if header_end.is_none() {
                                            if let Some(pos) =
                                                data.windows(4).position(|w| w == b"\r\n\r\n")
                                            {
                                                let hend = pos + 4;
                                                header_end = Some(hend);
                                                let hdr_str =
                                                    String::from_utf8_lossy(&data[..hend]);
                                                for line in hdr_str.lines() {
                                                    if let Some(rest) = line
                                                        .to_ascii_lowercase()
                                                        .strip_prefix("content-length:")
                                                    {
                                                        content_len =
                                                            rest.trim().parse().unwrap_or(0);
                                                    }
                                                }
                                            }
                                        }
                                        if let Some(hend) = header_end {
                                            if data.len() >= hend + content_len {
                                                break;
                                            }
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                            let req_str = String::from_utf8_lossy(&data);
                            let first_line = req_str.lines().next().unwrap_or("");
                            let body_slice =
                                header_end.and_then(|hend| data.get(hend..)).unwrap_or(&[]);

                            if first_line.contains("/api/stac/v1/search") {
                                let req_json: Value =
                                    serde_json::from_slice(body_slice).unwrap_or(json!({}));
                                let coll = req_json["collections"]
                                    .as_array()
                                    .and_then(|a| a.first())
                                    .and_then(Value::as_str)
                                    .unwrap_or("sentinel-2-l2a");
                                let item_id = if coll == "esa-worldcover" {
                                    "ESA_WorldCover_10m_2021_v200_N30E117_Map".to_owned()
                                } else {
                                    format!(
                                        "MOCK_{}_20240110",
                                        coll.replace('-', "_").to_ascii_uppercase()
                                    )
                                };
                                let asset_href = format!("{base_for_srv}/assets/sample.tif");
                                let stac = json!({
                                    "type": "FeatureCollection",
                                    "features": [{
                                        "id": item_id,
                                        "collection": coll,
                                        "bbox": [116.0, 30.0, 119.0, 33.0],
                                        "geometry": {
                                            "type": "Polygon",
                                            "coordinates": [[[116.0, 30.0], [119.0, 30.0], [119.0, 33.0], [116.0, 33.0], [116.0, 30.0]]]
                                        },
                                        "properties": {
                                            "datetime": "2024-01-10T02:59:51Z",
                                            "esa_worldcover:product_version": "V2.0.0"
                                        },
                                        "assets": {
                                            "visual": { "href": asset_href },
                                            "B04": { "href": asset_href },
                                            "B4": { "href": asset_href },
                                            "red": { "href": asset_href },
                                            "data": { "href": asset_href },
                                            "map": { "href": asset_href },
                                            "v200": { "href": asset_href },
                                            "elevation": { "href": asset_href },
                                            "vv": { "href": asset_href },
                                            "occurrence": { "href": asset_href },
                                            "HH": { "href": asset_href },
                                            "250m_16_days_NDVI": { "href": asset_href }
                                        }
                                    }]
                                });
                                let out = stac.to_string();
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    out.len(),
                                    out
                                );
                                let _ = stream.write_all(resp.as_bytes());
                            } else if first_line.contains("/api/data/v1/item/tilejson.json") {
                                let tj = json!({
                                    "tiles": [
                                        "https://planetarycomputer.microsoft.com/api/data/v1/item/tiles/WebMercatorQuad/{z}/{x}/{y}@1x?collection=sentinel-2-l2a&item=MOCK_ITEM_20240110&assets=visual"
                                    ]
                                });
                                let out = tj.to_string();
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    out.len(),
                                    out
                                );
                                let _ = stream.write_all(resp.as_bytes());
                            } else if first_line.contains("/api/sas/v1/token/") {
                                let tok = json!({
                                    "msft:expiry": "2099-01-01T00:00:00Z",
                                    "token": "sv=2021-06-08&st=2026-01-01T00%3A00%3A00Z&se=2099-01-01T00%3A00%3A00Z&sr=c&sp=rl&sig=loopbacksig"
                                });
                                let out = tok.to_string();
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    out.len(),
                                    out
                                );
                                let _ = stream.write_all(resp.as_bytes());
                            } else if first_line.contains("/assets/sample.tif") {
                                let total = sample_tif_bytes.len();
                                if first_line.starts_with("HEAD ") {
                                    let hdr = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nAccept-Ranges: bytes\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
                                    );
                                    let _ = stream.write_all(hdr.as_bytes());
                                } else {
                                    let mut range_opt: Option<(usize, usize)> = None;
                                    for line in req_str.lines() {
                                        if let Some(rval) =
                                            line.to_ascii_lowercase().strip_prefix("range: bytes=")
                                        {
                                            if let Some((s_str, e_str)) =
                                                rval.trim().split_once('-')
                                            {
                                                if let Ok(start) = s_str.trim().parse::<usize>() {
                                                    let end = e_str
                                                        .trim()
                                                        .parse::<usize>()
                                                        .unwrap_or(total.saturating_sub(1))
                                                        .min(total.saturating_sub(1));
                                                    if start <= end && start < total {
                                                        range_opt = Some((start, end));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if let Some((start, end)) = range_opt {
                                        let slice = &sample_tif_bytes[start..=end];
                                        let hdr = format!(
                                            "HTTP/1.1 206 Partial Content\r\nContent-Type: image/tiff\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                            slice.len()
                                        );
                                        let _ = stream.write_all(hdr.as_bytes());
                                        let _ = stream.write_all(slice);
                                    } else {
                                        let hdr = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nAccept-Ranges: bytes\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
                                        );
                                        let _ = stream.write_all(hdr.as_bytes());
                                        let _ = stream.write_all(&sample_tif_bytes);
                                    }
                                }
                            } else {
                                let hdr = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    one_px_png.len()
                                );
                                let _ = stream.write_all(hdr.as_bytes());
                                let _ = stream.write_all(&one_px_png);
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });

            fs::write(
                work_dir.join("network-settings.json"),
                json!({
                    "mode": "direct",
                    "test_loopback": true,
                    "mpc_test_base_url": base_url
                })
                .to_string(),
            )
            .expect("write loopback network-settings.json");

            Self {
                base_url,
                stop,
                handle: Some(handle),
            }
        }
    }

    impl Drop for LoopbackMpcServer {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    #[test]
    fn geodata_tools_open_dem_lbs_and_mapgis_offline_unit_pipeline() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dem_res = resolve_open_dem_tiles("cop30", [117.15, 31.75, 117.35, 31.90])
            .expect("resolve cop30 tiles for Hefei");
        assert_eq!(dem_res["tile_count"], 1);
        let tile = &dem_res["tiles"][0];
        assert!(tile["vsicurl_path"]
            .as_str()
            .unwrap()
            .starts_with("/vsicurl/https://copernicus-dem-30m.s3.amazonaws.com/"));

        let lbs_res = execute_lbs_coord_transform(&LbsCoordTransformRequest {
            from: "wgs84".to_owned(),
            to: "gcj02".to_owned(),
            points: vec![[117.227239, 31.820586]],
        })
        .expect("lbs transform");
        assert_eq!(lbs_res["count"], 1);
        assert_eq!(lbs_res["results"][0]["in_china_bounds"], true);

        let dir = std::env::temp_dir().join(format!(
            "eostage-geodata-tools-offline-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mpc = LoopbackMpcServer::spawn(&dir);

        let wl_path = dir.join("sample.wl");
        let shp_path = dir.join("sample.shp");
        let wl_bytes = build_synthetic_mapgis_binary_wl(&[vec![[117.20, 31.80], [117.25, 31.85]]]);
        fs::write(&wl_path, &wl_bytes).unwrap();

        let inspected = execute_mapgis_inspect(&MapGisInspectRequest {
            path: wl_path.to_string_lossy().to_string(),
        })
        .expect("mapgis inspect");
        assert_eq!(inspected["feature_count"], 1);

        let converted = execute_mapgis_convert(&MapGisConvertRequest {
            input_path: wl_path.to_string_lossy().to_string(),
            output_path: shp_path.to_string_lossy().to_string(),
            scale: None,
        })
        .expect("mapgis convert");
        assert_eq!(converted["ok"], true);
        assert!(shp_path.is_file());

        let sat_tiles_dir = dir.join("sat_tiles_loopback");
        let sat_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                code: Some(format!("{}/tiles/{{z}}/{{x}}/{{y}}.png", mpc.base_url)),
                keyword: Some("custom:12:mosaic:gtiff:DEFLATE:nopyramids:wgs84".to_owned()),
                bbox: Some([117.27, 31.85, 117.28, 31.86]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(sat_tiles_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("loopback xyz_satellite_tiles download");
        assert_eq!(sat_out["ok"], true);
        assert!(sat_out["tile_count"].as_u64().unwrap() >= 1);
        assert!(sat_tiles_dir.join("tiles_manifest.json").is_file());

        let wc_dir = dir.join("worldcover_hefei");
        let wc_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("ESA/WorldCover/v200".to_owned()),
                keyword: Some("map".to_owned()),
                bbox: Some([117.27, 31.85, 117.28, 31.86]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(wc_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: Some("download".to_owned()),
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("loopback gee_catalog ESA WorldCover 10m download");
        assert_eq!(wc_out["ok"], true);
        assert!(wc_out["count"].as_u64().unwrap() >= 1);
        let wc_file = PathBuf::from(
            wc_out["downloaded_file"]
                .as_str()
                .expect("worldcover downloaded_file"),
        );
        assert!(wc_file.is_file());
        assert!(wc_out["downloaded_bytes"].as_u64().unwrap() > 100);

        drop(mpc);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn geodata_tools_open_dem_lbs_and_mapgis_execute_end_to_end() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        // 1. Open DEM multi-mirror resolver
        let dem_res = resolve_open_dem_tiles("cop30", [117.15, 31.75, 117.35, 31.90])
            .expect("resolve cop30 tiles for Hefei");
        assert_eq!(dem_res["tile_count"], 1);
        let tile = &dem_res["tiles"][0];
        assert!(tile["vsicurl_path"]
            .as_str()
            .unwrap()
            .starts_with("/vsicurl/https://copernicus-dem-30m.s3.amazonaws.com/"));

        // 2. LBS coordinate transform
        let lbs_res = execute_lbs_coord_transform(&LbsCoordTransformRequest {
            from: "wgs84".to_owned(),
            to: "gcj02".to_owned(),
            points: vec![[117.227239, 31.820586]],
        })
        .expect("lbs transform");
        assert_eq!(lbs_res["count"], 1);
        assert_eq!(lbs_res["results"][0]["in_china_bounds"], true);

        // 3. MapGIS 6.x binary .wl inspect & convert
        let dir =
            std::env::temp_dir().join(format!("eostage-geodata-tools-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let wl_path = dir.join("sample.wl");
        let shp_path = dir.join("sample.shp");
        let wl_bytes = build_synthetic_mapgis_binary_wl(&[vec![[117.20, 31.80], [117.25, 31.85]]]);
        fs::write(&wl_path, &wl_bytes).unwrap();

        let inspected = execute_mapgis_inspect(&MapGisInspectRequest {
            path: wl_path.to_string_lossy().to_string(),
        })
        .expect("mapgis inspect");
        assert_eq!(inspected["feature_count"], 1);

        let converted = execute_mapgis_convert(&MapGisConvertRequest {
            input_path: wl_path.to_string_lossy().to_string(),
            output_path: shp_path.to_string_lossy().to_string(),
            scale: None,
        })
        .expect("mapgis convert");
        assert_eq!(converted["ok"], true);
        assert!(shp_path.is_file());

        // 4. WhiteboxTools CLI & DuckDB Spatial SQL execution if installed in runtime
        let rt_root = crate::application::discover_runtime_root();
        if crate::application::resolve_component_binary(
            &rt_root.join("runtime/whitebox-cli"),
            &["whitebox_tools"],
        )
        .is_some()
        {
            let wbt = execute_open_geodata_query(
                &dir,
                &OpenGeodataQueryRequest {
                    dataset: "whitebox_run".to_owned(),
                    code: None,
                    keyword: None,
                    bbox: None,
                    lat: None,
                    lng: None,
                    start_date: None,
                    end_date: None,
                    osm_tag: None,
                    dem_product: None,
                    iso3: None,
                    year: None,
                    output_path: None,
                    full_children: None,
                    tool_name: Some("version".to_owned()),
                    tool_args: None,
                    sql: None,
                    gee_params: None,
                    aoi_geojson: None,
                    zooms: None,
                },
            )
            .expect("whitebox_run version");
            assert_eq!(wbt["ok"], true);
            assert!(wbt["stdout"].as_str().unwrap().contains("WhiteboxTools"));
        }

        if crate::application::resolve_component_binary(
            &rt_root.join("runtime/duckdb-cli"),
            &["duckdb"],
        )
        .is_some()
        {
            let ddb = execute_open_geodata_query(
                &dir,
                &OpenGeodataQueryRequest {
                    dataset: "duckdb_sql".to_owned(),
                    code: None,
                    keyword: None,
                    bbox: None,
                    lat: None,
                    lng: None,
                    start_date: None,
                    end_date: None,
                    osm_tag: None,
                    dem_product: None,
                    iso3: None,
                    year: None,
                    output_path: None,
                    full_children: None,
                    tool_name: None,
                    tool_args: None,
                    sql: Some("SELECT ST_AsText(ST_Point(117.28, 31.86)) AS wkt;".to_owned()),
                    gee_params: None,
                    aoi_geojson: None,
                    zooms: None,
                },
            )
            .expect("duckdb_sql spatial point");
            assert_eq!(ddb["ok"], true);
            assert_eq!(ddb["rows"][0]["wkt"], "POINT (117.28 31.86)");
        }

        // 5. Real online /vsicurl/ COG streaming inspect + window slope processing
        let cog_url = tile["primary_url"].as_str().expect("primary_url");
        let cog_info = execute_gis_raster_inspect(&GisRasterInspectRequest {
            source: cog_url.to_owned(),
        })
        .expect("online /vsicurl/ COG inspect");
        assert_eq!(cog_info["ok"], true);
        assert_eq!(cog_info["is_remote_stream"], true);
        assert_eq!(cog_info["metadata"]["width"], 3600);
        assert_eq!(cog_info["metadata"]["height"], 3600);

        let remote_slope_path = dir.join("hefei_cog_window_slope.tif");
        let slope_out = execute_gis_raster_process(&RasterProcessRequest {
            sources: vec![cog_url.to_owned()],
            output_path: remote_slope_path.clone(),
            operation: "slope".to_owned(),
            crop_bbox: Some([117.27, 31.85, 117.28, 31.86]),
            band_a: Some(1),
            ..Default::default()
        })
        .expect("online /vsicurl/ window slope");
        assert_eq!(slope_out["ok"], true);
        assert_eq!(slope_out["is_remote_stream"], true);
        assert!(remote_slope_path.is_file());
        assert!(slope_out["bytes_written"].as_u64().unwrap() > 100);
        assert!(slope_out["statistics"]["valid_count"].as_u64().unwrap() > 100);

        // 6. Real satellite tile download with World File (.jgw) & projection (.prj) sidecars
        let sat_tiles_dir = dir.join("sat_tiles_hefei");
        let sat_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                code: None,
                keyword: Some("arcgis_satellite:12".to_owned()),
                bbox: Some([117.27, 31.85, 117.28, 31.86]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(sat_tiles_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("xyz_satellite_tiles download");
        assert_eq!(sat_out["ok"], true);
        assert!(sat_out["tile_count"].as_u64().unwrap() >= 1);
        assert!(sat_out["total_bytes"].as_u64().unwrap() > 500);
        assert!(sat_tiles_dir.join("tiles_manifest.json").is_file());
        let first_tile_path = PathBuf::from(
            sat_out["tiles"][0]["file_path"]
                .as_str()
                .expect("first tile file_path"),
        );
        let first_world_path = PathBuf::from(
            sat_out["tiles"][0]["world_file"]
                .as_str()
                .expect("first tile world_file"),
        );
        assert!(first_tile_path.is_file());
        assert!(first_world_path.is_file());
        let vrt_path = PathBuf::from(
            sat_out["mosaic_vrt_path"]
                .as_str()
                .expect("mosaic_vrt_path"),
        );
        assert!(vrt_path.is_file());
        if let Some(tif_str) = sat_out["mosaic_geotiff_path"].as_str() {
            assert!(PathBuf::from(tif_str).is_file());
        }

        // 7. Custom Tile URL template + Z20 super-zoom fallback + PNG export + DEFLATE + pyramids
        let custom_tiles_dir = dir.join("sat_tiles_custom_z20");
        let custom_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                code: Some("https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}".to_owned()),
                keyword: Some("custom:20:mosaic:png:DEFLATE:pyramids:wgs84".to_owned()),
                bbox: Some([117.2750, 31.8550, 117.2755, 31.8555]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(custom_tiles_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
            aoi_geojson: None,
            zooms: None,
            },
        )
        .expect("custom xyz_satellite_tiles z20 download");
        assert_eq!(custom_out["ok"], true);
        assert_eq!(custom_out["zoom"], 20);
        assert_eq!(custom_out["export_format"], "png");
        assert_eq!(custom_out["compression"], "DEFLATE");
        assert_eq!(custom_out["pyramids_built"], true);
        if let Some(exp_str) = custom_out["exported_mosaic_path"].as_str() {
            assert!(PathBuf::from(exp_str).is_file());
        }

        // 8. GEE Account Save + Status + Route 1 Cloud-Native Direct (ESA/WorldCover/v200)
        let gee_save = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_save".to_owned(),
                code: Some("ee-test-project".to_owned()),
                keyword: Some("service_account".to_owned()),
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: Some(r#"{"client_email":"test@ee-test-project.iam.gserviceaccount.com","private_key":"-----BEGIN PRIVATE KEY-----\nMIIE...\n-----END PRIVATE KEY-----\n"}"#.to_owned()),
                gee_params: None,
            aoi_geojson: None,
            zooms: None,
            },
        )
        .expect("gee_account_save");
        assert_eq!(gee_save["ok"], true);
        assert_eq!(gee_save["configured"], true);

        let gee_status = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_status".to_owned(),
                code: None,
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_account_status");
        assert_eq!(gee_status["configured"], true);
        assert_eq!(gee_status["project_id"], "ee-test-project");

        // 9. Account-free MPC STAC (ESA WorldCover 10m via GEE Route 1) search + AOI COG clip download
        let wc_dir = dir.join("worldcover_hefei");
        let wc_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("ESA/WorldCover/v200".to_owned()),
                keyword: Some("map".to_owned()),
                bbox: Some([117.27, 31.85, 117.28, 31.86]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(wc_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: Some("download".to_owned()),
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_catalog ESA WorldCover 10m download");
        assert_eq!(wc_out["ok"], true);
        assert!(wc_out["count"].as_u64().unwrap() >= 1);
        let wc_file = PathBuf::from(
            wc_out["downloaded_file"]
                .as_str()
                .expect("worldcover downloaded_file"),
        );
        assert!(wc_file.is_file());
        assert!(wc_out["downloaded_bytes"].as_u64().unwrap() > 100);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_gee_account_save_dual_contract_and_blank_secret_reuse() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dir = std::env::temp_dir().join(format!(
            "test_gee_account_save_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let app_dir = &dir;

        // 1. Direct field contract: code, keyword, sql
        let req_direct = OpenGeodataQueryRequest {
            dataset: "gee_account_save".to_owned(),
            code: Some("ee-project-direct".to_owned()),
            keyword: Some("oauth_token".to_owned()),
            sql: Some("ya29.sample_oauth_token".to_owned()),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        let res_direct = execute_open_geodata_query(app_dir, &req_direct).expect("gee direct save");
        assert_eq!(res_direct["ok"], true);
        assert_eq!(res_direct["project_id"], "ee-project-direct");
        assert_eq!(res_direct["auth_mode"], "oauth_token");

        // 2. Check persistence via gee_account_status
        let req_status = OpenGeodataQueryRequest {
            dataset: "gee_account_status".to_owned(),
            code: None,
            keyword: None,
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        let status = execute_open_geodata_query(app_dir, &req_status).expect("gee status check");
        assert_eq!(status["ok"], true);
        assert_eq!(status["configured"], true);
        assert_eq!(status["project_id"], "ee-project-direct");

        // 3. Alternative/Legacy contract: JSON in keyword
        let req_json = OpenGeodataQueryRequest {
            dataset: "gee_account_save".to_owned(),
            code: None,
            keyword: Some(r#"{"project_id":"ee-project-json","auth_mode":"service_account","secret":"{\"client_email\":\"json-sa@ee-project.iam.gserviceaccount.com\",\"private_key\":\"fake-key\"}"}"#.to_owned()),
            sql: None,
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        let res_json = execute_open_geodata_query(app_dir, &req_json).expect("gee json save");
        assert_eq!(res_json["ok"], true);
        assert_eq!(res_json["project_id"], "ee-project-json");
        assert_eq!(
            res_json["client_email"],
            "json-sa@ee-project.iam.gserviceaccount.com"
        );

        // 4. Blank secret on update reuses existing secret
        let req_blank_secret = OpenGeodataQueryRequest {
            dataset: "gee_account_save".to_owned(),
            code: Some("ee-project-updated".to_owned()),
            keyword: Some("service_account".to_owned()),
            sql: Some("".to_owned()),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        let res_update = execute_open_geodata_query(app_dir, &req_blank_secret)
            .expect("gee blank secret update");
        assert_eq!(res_update["ok"], true);
        assert_eq!(res_update["project_id"], "ee-project-updated");
        assert_eq!(
            res_update["client_email"],
            "json-sa@ee-project.iam.gserviceaccount.com"
        );

        // 5. Test Keyring persistence fallback when gee_credentials.json is deleted from disk
        let cred_file = app_dir.join("gee_credentials.json");
        assert!(cred_file.is_file());
        fs::remove_file(&cred_file).unwrap();
        assert!(!cred_file.is_file());

        let status_after_del = execute_open_geodata_query(app_dir, &req_status)
            .expect("gee status check from keyring");
        assert_eq!(status_after_del["ok"], true);
        assert_eq!(status_after_del["configured"], true);
        assert_eq!(status_after_del["project_id"], "ee-project-updated");
        assert_eq!(
            status_after_del["client_email"],
            "json-sa@ee-project.iam.gserviceaccount.com"
        );

        // 6. Test gee_account_clear purges both file and Keyring
        let req_clear = OpenGeodataQueryRequest {
            dataset: "gee_account_clear".to_owned(),
            code: None,
            keyword: None,
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        let clear_res = execute_open_geodata_query(app_dir, &req_clear).expect("gee account clear");
        assert_eq!(clear_res["ok"], true);
        assert_eq!(clear_res["configured"], false);

        let status_after_clear =
            execute_open_geodata_query(app_dir, &req_status).expect("gee status after clear");
        assert_eq!(status_after_clear["ok"], true);
        assert_eq!(status_after_clear["configured"], false);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_custom_tile_url_clean_code_parsing_and_no_arcgis_fallback() {
        let dir = std::env::temp_dir().join(format!(
            "test_custom_tile_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mpc = LoopbackMpcServer::spawn(&dir);
        let app_dir = &dir;
        let custom_url = format!("{}/tiles/{{z}}/{{x}}/{{y}}.png", mpc.base_url);
        let req = OpenGeodataQueryRequest {
            dataset: "xyz_satellite_tiles".to_owned(),
            code: Some(custom_url.clone()),
            keyword: Some("custom|wgs84:14:nomosaic:gtiff:DEFLATE:nopyramids".to_owned()),
            bbox: Some([117.2750, 31.8550, 117.2755, 31.8555]),
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: Some(
                app_dir
                    .join("custom_osm_tiles")
                    .to_string_lossy()
                    .to_string(),
            ),
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        let res = execute_open_geodata_query(app_dir, &req).expect("custom tile query");
        assert_eq!(res["ok"], true);
        assert_eq!(res["source"], "custom");
        assert_eq!(res["zoom"], 14);
        assert_eq!(res["custom_url_template"], custom_url);
        assert_eq!(res["export_format"], "gtiff");
        assert_eq!(res["pyramids_built"], false);
        let expected_prefix = format!("{}/tiles/14/", mpc.base_url);
        if let Some(first_tile) = res["tiles"].as_array().and_then(|t| t.first()) {
            let tile_url = first_tile["url"].as_str().unwrap();
            assert!(
                tile_url.starts_with(&expected_prefix),
                "Tile URL must match custom template"
            );
            assert!(
                !tile_url.contains("arcgisonline.com"),
                "Must NOT fall back to ArcGIS!"
            );
        }

        drop(mpc);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_custom_tile_legacy_embedded_url_parsing() {
        let dir = std::env::temp_dir().join(format!(
            "test_legacy_tile_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mpc = LoopbackMpcServer::spawn(&dir);
        let app_dir = &dir;
        let expected_template = format!("{}/tiles/{{z}}/{{x}}/{{y}}.png", mpc.base_url);
        let req = OpenGeodataQueryRequest {
            dataset: "xyz_satellite_tiles".to_owned(),
            code: None,
            keyword: Some(format!(
                "custom|wgs84|{expected_template}:15:nomosaic:gtiff:DEFLATE:nopyramids"
            )),
            bbox: Some([117.2750, 31.8550, 117.2755, 31.8555]),
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: Some(
                app_dir
                    .join("legacy_embedded_tiles")
                    .to_string_lossy()
                    .to_string(),
            ),
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        let res = execute_open_geodata_query(app_dir, &req).expect("legacy tile query");
        assert_eq!(res["ok"], true);
        assert_eq!(res["source"], "custom");
        assert_eq!(res["zoom"], 15);
        assert_eq!(res["custom_url_template"], expected_template);
        let expected_prefix = format!("{}/tiles/15/", mpc.base_url);
        if let Some(first_tile) = res["tiles"].as_array().and_then(|t| t.first()) {
            let tile_url = first_tile["url"].as_str().unwrap();
            assert!(tile_url.starts_with(&expected_prefix));
            assert!(
                !tile_url.contains("arcgisonline.com"),
                "Must NOT fall back to ArcGIS!"
            );
        }

        drop(mpc);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn test_point3_opera_aria_cmr_keyword_parsing() {
        let dir = std::env::temp_dir().join(format!(
            "test_cmr_kw_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 1. Pure search on OPERA DISP-S1 returns real CMR granules with zero fake manifest/PNG download
        let out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "opera_disp_s1".to_owned(),
                code: None,
                keyword: Some("2024-01-01..2024-12-31|DISP-S1|search".to_owned()),
                bbox: Some([-120.0, 34.0, -118.0, 36.0]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("search opera_disp_s1");
        assert_eq!(out["ok"], true);
        assert!(out["count"].as_u64().unwrap() > 0);
        assert!(out["downloaded_file"].is_null());

        // 2. Pure search on ARIA GUNW returns real CMR granules
        let aria_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "aria_s1_gunw".to_owned(),
                code: None,
                keyword: Some("2020-01-01..2022-12-31|GUNW|search".to_owned()),
                bbox: Some([-120.0, 34.0, -118.0, 36.0]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("search aria_s1_gunw");
        assert_eq!(aria_out["ok"], true);
        assert!(aria_out["count"].as_u64().unwrap() > 0);
        assert!(aria_out["downloaded_file"].is_null());

        // 3. Unauthenticated RTC-S1 download MUST fail closed with EARTHDATA_AUTH_REQUIRED (never return .png or _cmr_manifest.json!)
        std::env::set_var("EOSTAGE_FORCE_NO_EARTHDATA_AUTH", "1");
        let rtc_err = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "opera_disp_s1".to_owned(),
                code: None,
                keyword: Some("2024-01-01..2024-12-31|RTC-S1|download".to_owned()),
                bbox: Some([-120.0, 34.0, -118.0, 36.0]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .unwrap_err();
        std::env::remove_var("EOSTAGE_FORCE_NO_EARTHDATA_AUTH");
        assert!(
            rtc_err.contains("EARTHDATA_AUTH_REQUIRED"),
            "Expected EARTHDATA_AUTH_REQUIRED, got: {rtc_err}"
        );
        assert!(!dir.join("opera_disp_s1_cmr_manifest.json").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_nisar_products_catalog_exposes_l_band_and_s_band_status() {
        let dir = std::env::temp_dir().join(format!("test_nisar_cat_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "nisar_products".to_owned(),
                code: None,
                keyword: Some("catalog".to_owned()),
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("nisar catalog query succeeds offline");
        assert_eq!(out["ok"], true);
        let selectable = out["l_band_selectable_products"]
            .as_array()
            .expect("selectable products list");
        for code in ["RSLC", "GSLC", "GCOV", "GUNW", "SME2"] {
            assert!(selectable.iter().any(|v| v.as_str() == Some(code)));
        }
        assert!(out["catalog"].as_array().is_some_and(|arr| arr.len() >= 18));
        assert!(out["s_band_bhoonidhi"]
            .as_array()
            .is_some_and(|arr| arr.len() >= 4));

        // Validated-only SME2 must return an explicit empty response instead of leaking Provisional.
        let sme2_val = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "nisar_products".to_owned(),
                code: None,
                keyword: Some("2026-01-01..2026-11-19|SME2|tier:validated|search".to_owned()),
                bbox: Some([-120.0, 34.0, -118.0, 36.0]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("SME2 validated-only returns honest empty offline");
        assert_eq!(sme2_val["ok"], true);
        assert_eq!(sme2_val["count"], 0);
        assert_eq!(sme2_val["items"].as_array().map(|a| a.len()), Some(0));
        assert_eq!(sme2_val["requested_tiers"], json!(["validated"]));
        assert!(sme2_val["empty_reason_zh"]
            .as_str()
            .is_some_and(|s| s.contains("Validated") && s.contains("Provisional")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn test_live_nisar_validated_vs_provisional_tier_filtering() {
        let dir =
            std::env::temp_dir().join(format!("test_nisar_live_tiers_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let make_req = |kw: &str| OpenGeodataQueryRequest {
            dataset: "nisar_products".to_owned(),
            code: None,
            keyword: Some(kw.to_owned()),
            bbox: Some([-120.0, 34.0, -118.0, 36.0]),
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };

        let val_res = execute_open_geodata_query(
            &dir,
            &make_req("2026-01-01..2026-11-19|RSLC|tier:validated|search"),
        )
        .expect("live validated NISAR query succeeds");
        let val_items = val_res["items"].as_array().expect("validated items array");
        assert!(
            !val_items.is_empty(),
            "expected live Validated RSLC granules"
        );
        assert!(
            val_items.iter().all(|item| {
                item["release_tier"] == "validated"
                    && item["tier"] == "Validated"
                    && item["collection_short_name"] == "NISAR_L1_RSLC_V1"
            }),
            "every Validated-only result must be tagged validated (even with _L1_PR_ in GranuleUR)"
        );

        let prov_res = execute_open_geodata_query(
            &dir,
            &make_req("2026-01-01..2026-11-19|RSLC|tier:provisional|search"),
        )
        .expect("live provisional NISAR query succeeds");
        let prov_items = prov_res["items"]
            .as_array()
            .expect("provisional items array");
        assert!(
            !prov_items.is_empty(),
            "expected live Provisional RSLC granules"
        );
        assert!(
            prov_items.iter().all(|item| {
                item["release_tier"] == "provisional"
                    && item["tier"] == "Provisional"
                    && item["collection_short_name"] == "NISAR_L1_RSLC_PROVISIONAL_V1"
            }),
            "every Provisional-only result must be tagged provisional"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn test_point3_open_meteo_era5_dynamic_bbox_and_hourly_csv() {
        let dir = std::env::temp_dir().join(format!(
            "test_era5_bbox_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "open_meteo_era5".to_owned(),
                code: None,
                keyword: Some("2023-06-01..2023-06-02|hourly_insar|download".to_owned()),
                bbox: Some([116.3, 39.8, 116.5, 40.0]), // Beijing AOI (not Hefei)
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("query open_meteo_era5 with bbox");

        assert_eq!(out["ok"], true);
        assert_eq!(out["grid_points_count"], 5);
        assert!(out["total_hours"].as_u64().unwrap() >= 24);

        let csv_path = PathBuf::from(out["csv_path"].as_str().unwrap());
        assert!(csv_path.is_file());
        let csv_text = fs::read_to_string(&csv_path).unwrap();
        assert!(csv_text.contains("point_tag,latitude,longitude"));
        assert!(csv_text.contains("geopotential_height_m"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn test_point3_ionospheric_ionex_standard_format_and_vtec_grid() {
        let dir = std::env::temp_dir().join(format!(
            "test_ionex_grid_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "ionospheric_ionex".to_owned(),
                code: None,
                keyword: Some("COD".to_owned()),
                bbox: Some([117.20, 31.80, 117.26, 31.85]),
                lat: None,
                lng: None,
                start_date: Some("2024-05-15".to_owned()),
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("generate ionospheric_ionex");

        assert_eq!(out["ok"], true);
        assert_eq!(out["doy"], 136); // 2024 is leap year, May 15 is DOY 136
        assert_eq!(out["filename"], "CODG1360.24I");
        assert!(out["maps_count"].as_u64().unwrap() >= 13);

        let file_path = PathBuf::from(out["downloaded_file"].as_str().unwrap());
        assert!(file_path.is_file());
        let content = fs::read_to_string(&file_path).unwrap();

        // Strict IONEX 1.0 format assertions on REAL decompressed CODE archive
        assert!(content.contains("IONEX VERSION / TYPE"));
        assert!(content.contains("EPOCH OF FIRST MAP"));
        assert!(content.contains("EPOCH OF LAST MAP"));
        assert!(content.contains("START OF TEC MAP"));
        assert!(content.contains("LAT/LON1/LON2/DLON/H"));
        assert!(content.contains("END OF TEC MAP"));
        assert!(content.contains("END OF FILE"));
        assert!(content.len() > 100_000);
        if let Some(dcb_s) = out["dcb_file"].as_str() {
            let dcb_txt = fs::read_to_string(dcb_s).unwrap();
            assert!(dcb_txt.contains("DIFFERENTIAL CODE BIASES"));
            assert!(
                dcb_txt.len() < content.len() / 2,
                "DCB sidecar must contain ONLY the extracted DIFFERENTIAL CODE BIASES block, never a copy of the full IONEX file"
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_h1_build_http_client_proxy_settings_from_app_data_dir() {
        let dir = std::env::temp_dir().join(format!(
            "test_proxy_settings_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 1. No network-settings.json -> succeeds with default client
        assert!(build_http_client(&dir).is_ok());

        // 2. Direct mode -> succeeds with no_proxy
        fs::write(
            dir.join("network-settings.json"),
            r#"{"mode":"direct","http_proxy":"http://127.0.0.1:7897"}"#,
        )
        .unwrap();
        assert!(build_http_client(&dir).is_ok());

        // 3. Manual mode with valid proxy URL -> succeeds
        fs::write(
            dir.join("network-settings.json"),
            r#"{"mode":"manual","http_proxy":"http://127.0.0.1:7897","https_proxy":"http://127.0.0.1:7897"}"#,
        )
        .unwrap();
        assert!(build_http_client(&dir).is_ok());

        // 4. Manual mode with malformed proxy URL -> returns INVALID_PROXY_URL error
        fs::write(
            dir.join("network-settings.json"),
            r#"{"mode":"manual","http_proxy":"http://[::1:bad_port"}"#,
        )
        .unwrap();
        let err = build_http_client(&dir).unwrap_err();
        assert!(
            err.contains("INVALID_PROXY_URL"),
            "Expected INVALID_PROXY_URL, got: {err}"
        );

        // 5. ApplicationNetworkSettings schema with proxy_enabled=false and non-empty proxy_url -> must use no_proxy() and ignore proxy_url
        fs::write(
            dir.join("network-settings.json"),
            r#"{"proxy_enabled":false,"proxy_url":"http://[::1:bad_port"}"#,
        )
        .unwrap();
        assert!(
            build_http_client(&dir).is_ok(),
            "proxy_enabled=false must ignore proxy_url and use no_proxy()"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn test_c1_gee_catalog_live_era5_open_meteo_delegation() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dir = std::env::temp_dir().join(format!(
            "test_c1_gee_era5_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let era5_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: None,
                keyword: Some("2024-05-01..2024-05-02|ECMWF/ERA5_LAND/HOURLY|download".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("gee_era5").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("GEE catalog ECMWF/ERA5_LAND/HOURLY delegation");
        assert_eq!(era5_out["ok"], true);
        assert_eq!(era5_out["gee_asset_id"], "ECMWF/ERA5_LAND/HOURLY");
        assert_eq!(era5_out["route_mode"], "cloud_native_direct");
        assert_eq!(era5_out["variable_set"], "hourly_insar");
        assert!(era5_out["downloaded_file"].as_str().is_some());
        assert_eq!(era5_out["downloaded_file"], era5_out["downloaded_path"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_c1_gee_catalog_pipe_delimited_and_all_presets_delegation() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dir = std::env::temp_dir().join(format!(
            "test_c1_gee_catalog_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mpc = LoopbackMpcServer::spawn(&dir);

        // 1. UI-style pipe-delimited keyword with code: Some("COPERNICUS/S2_SR_HARMONIZED")
        let s2_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("COPERNICUS/S2_SR_HARMONIZED".to_owned()),
                keyword: Some(
                    "2024-05-01..2024-06-01|COPERNICUS/S2_SR_HARMONIZED|search".to_owned(),
                ),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("GEE catalog COPERNICUS/S2_SR_HARMONIZED delegation");
        assert_eq!(s2_out["ok"], true);
        assert_eq!(s2_out["gee_asset_id"], "COPERNICUS/S2_SR_HARMONIZED");
        assert_eq!(s2_out["route_mode"], "cloud_native_direct");
        assert_eq!(s2_out["collection"], "sentinel-2-l2a");
        assert_eq!(s2_out["sub_product"], "visual");

        // 1b. Pipe-delimited keyword containing BOTH GEE asset ID and explicit band ("COPERNICUS/S2_SR_HARMONIZED|2024-05-01..2024-06-01|B04|search")
        let s2_band_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: None,
                keyword: Some(
                    "COPERNICUS/S2_SR_HARMONIZED|2024-05-01..2024-06-01|B04|search".to_owned(),
                ),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("GEE catalog COPERNICUS/S2_SR_HARMONIZED with B04 band in keyword");
        assert_eq!(s2_band_out["ok"], true);
        assert_eq!(s2_band_out["gee_asset_id"], "COPERNICUS/S2_SR_HARMONIZED");
        assert_eq!(s2_band_out["sub_product"], "B04");

        // 2. Verify canonical ERA5 mapping without external network I/O (live Open-Meteo E2E is in test_c1_gee_catalog_live_era5_open_meteo_delegation)
        assert_eq!(
            map_canonical_gee_asset("ECMWF/ERA5_LAND/HOURLY"),
            Some(("open_meteo_era5", "era5-land", "hourly_insar"))
        );

        // 3. GEE preset COPERNICUS/DEM/GLO30 delegation
        let dem_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("COPERNICUS/DEM/GLO30".to_owned()),
                keyword: Some("2024-01-01..2024-12-31|COPERNICUS/DEM/GLO30|search".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("GEE catalog COPERNICUS/DEM/GLO30 delegation");
        assert_eq!(dem_out["ok"], true);
        assert_eq!(dem_out["collection"], "cop-dem-glo-30");
        assert_eq!(dem_out["sub_product"], "data");
        drop(mpc);

        // 4. Route 2 (gee_official_rest): pure-Rust Service Account RS256 JWT exchange + image:computePixels against local mock server
        let rsa_key_path = dir.join("test_rsa.pem");
        let genrsa_ok = std::process::Command::new("openssl")
            .args(["genrsa", "-out", &rsa_key_path.to_string_lossy(), "2048"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if genrsa_ok {
            let private_key_pem = fs::read_to_string(&rsa_key_path).unwrap();
            let _ = fs::remove_file(&rsa_key_path);
            fs::write(
                dir.join("network-settings.json"),
                r#"{"mode":"direct","test_loopback":true}"#,
            )
            .unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let base_url = format!("http://{addr}");
            let token_uri = format!("{base_url}/token");

            let server_handle = std::thread::spawn(move || {
                use std::io::{Read, Write};
                for _ in 0..2 {
                    if let Some(mut stream) =
                        accept_with_timeout(&listener, Duration::from_secs(10))
                    {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let mut buf = [0u8; 8192];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let req_str = String::from_utf8_lossy(&buf[..n]);
                        if req_str.starts_with("POST /token") {
                            assert!(req_str.contains("grant_type="));
                            assert!(req_str.contains("assertion="));
                            let body = r#"{"access_token":"mock_gee_sa_oauth2_token_xyz","expires_in":3600,"token_type":"Bearer"}"#;
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = stream.write_all(resp.as_bytes());
                        } else if req_str
                            .starts_with("POST /v1/projects/test-gee-proj/image:computePixels")
                        {
                            assert!(req_str.contains("Bearer mock_gee_sa_oauth2_token_xyz"));
                            assert!(req_str.contains("\"crsCode\":\"EPSG:4326\""));
                            assert!(req_str.contains("\"affineTransform\""));
                            let fake_tiff: &[u8] = b"II*\0\x08\0\0\0MOCK_GEOTIFF_BYTES";
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                fake_tiff.len()
                            );
                            let _ = stream.write_all(header.as_bytes());
                            let _ = stream.write_all(fake_tiff);
                        }
                    }
                }
            });

            let sa_json = json!({
                "type": "service_account",
                "client_email": "test-sa@test-gee-proj.iam.gserviceaccount.com",
                "private_key": private_key_pem,
                "token_uri": token_uri
            });
            store_gee_keyring_bundle(
                "test-gee-proj",
                "service_account",
                &sa_json.to_string(),
                "test-sa@test-gee-proj.iam.gserviceaccount.com",
            )
            .expect("store mock SA in OS keyring");
            let cred_json = json!({
                "configured": true,
                "project_id": "test-gee-proj",
                "api_base_url": base_url
            });
            fs::write(dir.join("gee_credentials.json"), cred_json.to_string()).unwrap();

            let custom_out = execute_open_geodata_query(
                &dir,
                &OpenGeodataQueryRequest {
                    dataset: "gee_catalog".to_owned(),
                    code: Some("projects/test-gee-proj/assets/custom_dem".to_owned()),
                    keyword: Some(
                        "2024-01-01..2024-12-31|projects/test-gee-proj/assets/custom_dem|download"
                            .to_owned(),
                    ),
                    bbox: Some([117.20, 31.80, 117.30, 31.90]),
                    lat: None,
                    lng: None,
                    start_date: None,
                    end_date: None,
                    osm_tag: None,
                    dem_product: None,
                    iso3: None,
                    year: None,
                    output_path: Some(dir.join("gee_custom_out").to_string_lossy().to_string()),
                    full_children: None,
                    tool_name: None,
                    tool_args: None,
                    sql: None,
                    gee_params: None,
                    aoi_geojson: None,
                    zooms: None,
                },
            )
            .expect("Custom GEE asset export via Route 2 official REST API");
            let _ = server_handle.join();
            assert_eq!(custom_out["ok"], true);
            assert_eq!(custom_out["route_mode"], "gee_official_rest");
            assert_eq!(
                custom_out["gee_asset_id"],
                "projects/test-gee-proj/assets/custom_dem"
            );
            let custom_tif = PathBuf::from(custom_out["downloaded_file"].as_str().unwrap());
            assert!(custom_tif.is_file());
            assert_eq!(custom_out["downloaded_file"], custom_out["downloaded_path"]);
            assert_eq!(custom_out["magic_bytes"], "49492a00");
            assert!(custom_out["size_bytes"].as_u64().unwrap() > 0);
            assert_eq!(custom_out["grid_height"], 1024);
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_gee_map_tiles_and_compute_pixels_validation() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dir = std::env::temp_dir().join(format!(
            "test_gee_tiles_val_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mpc = LoopbackMpcServer::spawn(&dir);

        // 1. Route 1 Tile fetcher returns REAL Planetary Computer Data API TileJSON URL (zero fake SHA-256 map_id!)
        let tile_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_map_tiles".to_owned(),
                code: Some("COPERNICUS/S2_SR_HARMONIZED".to_owned()),
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_map_tiles default query");

        assert_eq!(tile_res["ok"], true);
        assert_eq!(tile_res["dataset"], "gee_map_tiles");
        assert_eq!(tile_res["gee_asset_id"], "COPERNICUS/S2_SR_HARMONIZED");
        assert_eq!(tile_res["route_mode"], "cloud_native_direct");
        let tpl = tile_res["tile_url_template"].as_str().unwrap();
        assert!(tpl.starts_with("https://planetarycomputer.microsoft.com/api/data/v1/item/tiles/"));
        assert!(tpl.contains("{z}/{x}/{y}"));
        assert_eq!(tile_res["url_format"], tile_res["tile_url_template"]);

        // 1b. Unauthenticated Route 2 custom asset on gee_map_tiles MUST return Err (zero fake SHA-256 map_id!)
        let unauth_tile_err = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_map_tiles".to_owned(),
                code: Some("projects/unauth-proj/assets/private_dem".to_owned()),
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .unwrap_err();
        assert!(unauth_tile_err.contains("GEE_ACCOUNT_REQUIRED"));

        // 2. Tile fetcher with pipe-delimited visualization options
        let tile_vis = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_map_tiles".to_owned(),
                code: Some("LANDSAT/LC09/C02/T1_L2".to_owned()),
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: Some("B5,B4,B3|100..4500".to_owned()),
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_map_tiles custom vis");

        assert_eq!(tile_vis["ok"], true);
        assert_eq!(tile_vis["min"], 100.0);
        assert_eq!(tile_vis["max"], 4500.0);
        let bands = tile_vis["bands"].as_array().unwrap();
        assert_eq!(bands.len(), 3);
        assert_eq!(bands[0], "B5");
        assert_eq!(bands[1], "B4");
        assert_eq!(bands[2], "B3");

        // 3. computePixels invalid GeoTIFF header rejection (secret stored in OS Keyring only)
        drop(mpc);
        fs::write(
            dir.join("network-settings.json"),
            r#"{"mode":"direct","test_loopback":true}"#,
        )
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let server_handle = std::thread::spawn(move || {
            use std::io::{Read, Write};
            if let Some(mut stream) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                // Return 200 OK with non-TIFF HTML error payload
                let body = "<html><body>502 Bad Gateway from upstream</body></html>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        store_gee_keyring_bundle("test-bad-tiff-proj", "oauth_token", "mock_token", "")
            .expect("store token in keyring");
        let cred_json = json!({
            "configured": true,
            "project_id": "test-bad-tiff-proj",
            "api_base_url": base_url,
            "auth_mode": "oauth_token"
        });
        fs::write(dir.join("gee_credentials.json"), cred_json.to_string()).unwrap();

        let bad_tiff_err = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("projects/test-bad-tiff-proj/assets/bad_raster".to_owned()),
                keyword: Some("projects/test-bad-tiff-proj/assets/bad_raster|download".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("bad_out").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .unwrap_err();

        let _ = server_handle.join();
        assert!(bad_tiff_err.contains("GEE_INVALID_TIFF_HEADER"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn test_c2_mpc_stac_and_usgs_earthquake_pipe_delimited_subproducts() {
        let dir = std::env::temp_dir().join(format!(
            "test_c2_subproducts_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 1. ESA WorldCover v100 vs v200 filtering
        let wc_v100 = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "esa_worldcover".to_owned(),
                code: None,
                keyword: Some("2020-01-01..2020-12-31|v100|search".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("esa_worldcover v100");
        let wc_v200 = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "esa_worldcover".to_owned(),
                code: None,
                keyword: Some("2021-01-01..2021-12-31|v200|search".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("esa_worldcover v200");
        let id_v100 = wc_v100["items"][0]["id"].as_str().unwrap_or("");
        let id_v200 = wc_v200["items"][0]["id"].as_str().unwrap_or("");
        assert_ne!(
            id_v100, id_v200,
            "WorldCover v100 and v200 must resolve to distinct STAC items"
        );
        assert!(id_v100.contains("2020") || id_v100.to_ascii_lowercase().contains("v100"));
        assert!(id_v200.contains("2021") || id_v200.to_ascii_lowercase().contains("v200"));

        // 2. MODIS 13Q1 EVI sub-product selection
        let modis_evi = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "modis_ndvi".to_owned(),
                code: None,
                keyword: Some("2023-05-01..2023-06-01|250m_16_days_EVI|search".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("modis_ndvi EVI");
        assert_eq!(modis_evi["ok"], true);
        assert_eq!(modis_evi["sub_product"], "250m_16_days_EVI");
        if let Some(first_item) = modis_evi["items"].as_array().and_then(|a| a.first()) {
            assert_eq!(first_item["asset_key"], "250m_16_days_EVI");
        }

        // 3. USGS Earthquake pipe-delimited keyword (start..end|min_magnitude|download)
        let eq_out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "usgs_earthquake".to_owned(),
                code: None,
                keyword: Some("2023-01-01..2024-06-01|5.5|download".to_owned()),
                bbox: Some([120.0, 20.0, 145.0, 45.0]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("usgs_eq").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("usgs_earthquake pipe-delimited query");
        assert_eq!(eq_out["ok"], true);
        assert_eq!(eq_out["min_magnitude"], 5.5);
        assert_eq!(eq_out["start_date"], "2023-01-01");
        assert_eq!(eq_out["end_date"], "2024-06-01");
        assert!(eq_out["downloaded_file"].as_str().is_some());
        assert_eq!(eq_out["downloaded_file"], eq_out["downloaded_path"]);
        for item in eq_out["items"].as_array().unwrap_or(&vec![]) {
            assert!(item["magnitude"].as_f64().unwrap() >= 5.5);
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn test_c4_super_zoom_z20_subtile_crop_and_upsample() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        fn crc32(data: &[u8]) -> u32 {
            let mut crc: u32 = 0xFFFF_FFFF;
            for &b in data {
                crc ^= b as u32;
                for _ in 0..8 {
                    let mask = (crc & 1).wrapping_neg();
                    crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
                }
            }
            !crc
        }
        fn png_chunk(tag: &[u8; 4], payload: &[u8], out: &mut Vec<u8>) {
            out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            out.extend_from_slice(tag);
            out.extend_from_slice(payload);
            let mut crc_in = Vec::with_capacity(4 + payload.len());
            crc_in.extend_from_slice(tag);
            crc_in.extend_from_slice(payload);
            out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
        }
        fn make_quadrant_256_png() -> Vec<u8> {
            use flate2::write::ZlibEncoder;
            use flate2::Compression;
            let mut raw = Vec::with_capacity(256 * (1 + 256 * 3));
            for y in 0..256u32 {
                raw.push(0); // filter type 0
                for x in 0..256u32 {
                    let (r, g, b) = match (x >= 128, y >= 128) {
                        (false, false) => (220u8, 40u8, 40u8),
                        (true, false) => (40u8, 220u8, 40u8),
                        (false, true) => (40u8, 40u8, 220u8),
                        (true, true) => (220u8, 220u8, 40u8),
                    };
                    raw.extend_from_slice(&[r, g, b]);
                }
            }
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
            enc.write_all(&raw).unwrap();
            let idat = enc.finish().unwrap();
            let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
            let mut ihdr = Vec::with_capacity(13);
            ihdr.extend_from_slice(&256u32.to_be_bytes());
            ihdr.extend_from_slice(&256u32.to_be_bytes());
            ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
            png_chunk(b"IHDR", &ihdr, &mut png);
            png_chunk(b"IDAT", &idat, &mut png);
            png_chunk(b"IEND", &[], &mut png);
            png
        }

        let dir = std::env::temp_dir().join(format!(
            "test_c4_superzoom_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("network-settings.json"),
            r#"{"mode":"direct","test_loopback":true}"#,
        )
        .unwrap();

        let parent_png = make_quadrant_256_png();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);

        let srv = std::thread::spawn(move || {
            while !stop_bg.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let tile = parent_png.clone();
                        std::thread::spawn(move || {
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let mut buf = [0u8; 2048];
                            let n = stream.read(&mut buf).unwrap_or(0);
                            let req_head = String::from_utf8_lossy(&buf[..n]);
                            if req_head.contains("/tiles/20/") {
                                let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found";
                                let _ = stream.write_all(resp.as_bytes());
                            } else {
                                let hdr = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    tile.len()
                                );
                                let _ = stream.write_all(hdr.as_bytes());
                                let _ = stream.write_all(&tile);
                            }
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        // Request adjacent Z20 tiles from custom loopback server returning 404 at Z20 and valid 256x256 PNG at Z19
        let out = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                code: Some(format!(
                    "http://127.0.0.1:{port}/tiles/{{z}}/{{x}}/{{y}}.png"
                )),
                keyword: Some("custom:20:mosaic:gtiff:DEFLATE:nopyramids:wgs84".to_owned()),
                bbox: Some([117.2750, 31.8550, 117.2755, 31.8553]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(
                    dir.join("z20_custom_superzoom")
                        .to_string_lossy()
                        .to_string(),
                ),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("Z20 super-zoom fallback query");
        stop.store(true, Ordering::Relaxed);
        let _ = srv.join();
        assert_eq!(out["ok"], true);
        assert_eq!(out["zoom"], 20);
        assert!(out["downloaded_file"].as_str().is_some());
        assert_eq!(out["downloaded_file"], out["downloaded_path"]);

        let tiles = out["tiles"].as_array().expect("tiles array");
        assert!(!tiles.is_empty());
        for t in tiles {
            assert!(t["fallback_zoom"].as_u64().unwrap() <= 19);
            assert!(t["subtile_quadrant"].is_object());
            assert_eq!(
                t["subtile_cropped"], true,
                "GDAL sub-tile crop+upsample should succeed"
            );
        }
        if tiles.len() >= 2 {
            let p0 = PathBuf::from(tiles[0]["file_path"].as_str().unwrap());
            let p1 = PathBuf::from(tiles[1]["file_path"].as_str().unwrap());
            let b0 = fs::read(&p0).unwrap();
            let b1 = fs::read(&p1).unwrap();
            assert_ne!(
                b0, b1,
                "Adjacent Z20 child tiles from the same fallback parent must be cropped to distinct sub-quadrants"
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn test_h3_h4_h5_era5_variable_sets_ionex_pipe_keyword_and_licsar_live_query() {
        let dir = std::env::temp_dir().join(format!(
            "test_h3_h4_h5_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 1. [H-3] open_meteo_era5 daily_summary variable set
        let era5_daily = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "open_meteo_era5".to_owned(),
                code: None,
                keyword: Some("2023-06-01..2023-06-03|daily_summary|download".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("era5_daily").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("open_meteo_era5 daily_summary");
        assert_eq!(era5_daily["ok"], true);
        assert_eq!(era5_daily["variable_set"], "daily_summary");
        let daily_csv =
            fs::read_to_string(era5_daily["downloaded_file"].as_str().unwrap()).unwrap();
        assert!(daily_csv.contains("temperature_2m_mean_degC,precipitation_sum_mm,rain_sum_mm,wind_speed_10m_max_kmh,shortwave_radiation_sum_MJm2"));

        // 2. [H-4] ionospheric_ionex with UI pipe-delimited keyword & guaranteed uncompressed file
        let ionex_jpl = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "ionospheric_ionex".to_owned(),
                code: None,
                keyword: Some("2024-05-15..2024-05-16|JPL|download".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("ionex_jpl").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("ionospheric_ionex pipe-delimited JPL");
        assert_eq!(ionex_jpl["ok"], true);
        assert_eq!(ionex_jpl["analysis_center"], "JPL");
        assert_eq!(ionex_jpl["date"], "2024-05-15");
        assert_eq!(ionex_jpl["filename"], "JPLG1360.24I");
        let ionex_path = PathBuf::from(ionex_jpl["downloaded_file"].as_str().unwrap());
        assert!(
            ionex_path.is_file(),
            "Uncompressed IONEX file must exist on disk"
        );
        assert_eq!(ionex_jpl["downloaded_file"], ionex_jpl["downloaded_path"]);

        // 3. [H-5] comet_licsar live query (real JASMIN polygon + real GeoTIFF magic, zero metadata.txt/JSON fallback)
        let licsar = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "comet_licsar".to_owned(),
                code: None,
                keyword: Some("2024-01-01..2024-12-31|GEOCODED_UNW_COH|download".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("licsar").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("comet_licsar live query");
        assert_eq!(licsar["ok"], true);
        assert!(licsar["count"].as_u64().unwrap() >= 1);
        assert!(licsar["live_query_endpoint"]
            .as_str()
            .unwrap()
            .starts_with("https://gws-access.jasmin.ac.uk/"));
        let poly_coords = licsar["items"][0]["geometry"]["coordinates"][0]
            .as_array()
            .expect("licsar frame real polygon ring");
        assert!(
            poly_coords.len() > 5,
            "LiCSAR geometry must come from real JASMIN -poly.txt polygon, not stamped user bbox"
        );
        let licsar_file = PathBuf::from(licsar["downloaded_file"].as_str().unwrap());
        assert!(licsar_file.is_file());
        assert!(
            licsar_file.to_string_lossy().ends_with(".tif"),
            "Downloaded LiCSAR science product must be a .tif GeoTIFF, got: {:?}",
            licsar_file
        );
        let licsar_head = fs::read(&licsar_file).expect("read licsar geotiff");
        assert!(
            licsar_head.starts_with(b"II*\0") || licsar_head.starts_with(b"MM\0*"),
            "Downloaded LiCSAR file must have valid TIFF magic bytes"
        );
        assert!(
            !dir.join("licsar").join("comet_licsar_frames.json").exists(),
            "Synthetic comet_licsar_frames.json fallback must never be written"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_gee_engine_auth_and_tile_delivery() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dir = std::env::temp_dir().join(format!(
            "test_gee_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let _mpc = LoopbackMpcServer::spawn(&dir);

        // 1. Initial GEE account status is unconfigured
        let status_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_status".to_owned(),
                code: None,
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_account_status check");
        assert_eq!(status_res["ok"], true);
        assert_eq!(status_res["configured"], false);

        // 2. Save GEE service account credentials (secret MUST go only to OS Keyring, never plaintext disk!)
        let sa_json = json!({
            "type": "service_account",
            "project_id": "ee-sceneport-test",
            "client_email": "test-sa@ee-sceneport-test.iam.gserviceaccount.com",
            "private_key": "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC3\n-----END PRIVATE KEY-----\n"
        }).to_string();

        let save_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_save".to_owned(),
                code: Some("ee-sceneport-test".to_owned()),
                keyword: Some("service_account".to_owned()),
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: Some(sa_json.clone()),
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_account_save");
        assert_eq!(save_res["ok"], true);
        assert_eq!(save_res["configured"], true);
        assert_eq!(save_res["project_id"], "ee-sceneport-test");
        assert_eq!(
            save_res["client_email"],
            "test-sa@ee-sceneport-test.iam.gserviceaccount.com"
        );

        let disk_cred_raw = fs::read_to_string(dir.join("gee_credentials.json"))
            .expect("gee_credentials.json metadata file");
        assert!(
            !disk_cred_raw.contains("BEGIN PRIVATE KEY") && !disk_cred_raw.contains("\"secret\""),
            "gee_credentials.json on disk must NEVER store plaintext secret/private_key: {disk_cred_raw}"
        );

        // 3. Status now reports configured with correct project_id and client_email
        let status_res2 = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_status".to_owned(),
                code: None,
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_account_status after save");
        assert_eq!(status_res2["ok"], true);
        assert_eq!(status_res2["configured"], true);
        assert_eq!(status_res2["project_id"], "ee-sceneport-test");
        assert_eq!(
            status_res2["client_email"],
            "test-sa@ee-sceneport-test.iam.gserviceaccount.com"
        );

        // 4. Test GEE tile template generation for Route 1 canonical dataset (real Planetary Computer TileJSON)
        let tiles_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_map_tiles".to_owned(),
                code: Some("COPERNICUS/S2_SR_HARMONIZED".to_owned()),
                keyword: Some("B4,B3,B2|0..3000".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_map_tiles");
        assert_eq!(tiles_res["ok"], true);
        assert_eq!(tiles_res["gee_asset_id"], "COPERNICUS/S2_SR_HARMONIZED");
        let template = tiles_res["tile_url_template"].as_str().unwrap();
        assert!(
            template.contains("/tiles/") && template.contains("{z}/{x}/{y}"),
            "Tile template must be a real TileJSON URL containing /tiles/ and {{z}}/{{x}}/{{y}}, got: {template}"
        );
        assert!(
            !template.contains("earthengine.googleapis.com/v1/projects/earthengine-public/maps/"),
            "Fake SHA-256 earthengine-public map_id URL must never be emitted"
        );
        assert_eq!(tiles_res["bands"], json!(["B4", "B3", "B2"]));
        assert_eq!(tiles_res["min"], 0.0);
        assert_eq!(tiles_res["max"], 3000.0);

        // 5. Test Canonical GEE dataset delegate (Route 1)
        let canon_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("COPERNICUS/S2_SR_HARMONIZED".to_owned()),
                keyword: Some("B4".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: Some("2024-01-01".to_owned()),
                end_date: Some("2024-01-15".to_owned()),
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("gee_canon").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_catalog canonical delegate");
        assert_eq!(canon_res["ok"], true);
        assert_eq!(canon_res["gee_asset_id"], "COPERNICUS/S2_SR_HARMONIZED");
        assert_eq!(canon_res["route_mode"], "cloud_native_direct");
        let canon_tpl = canon_res["tile_url_template"].as_str().unwrap();
        assert!(canon_tpl.contains("/tiles/") && canon_tpl.contains("{z}/{x}/{y}"));

        // 6. Test clear credentials
        let clear_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_clear".to_owned(),
                code: None,
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_account_clear");
        assert_eq!(clear_res["ok"], true);
        assert_eq!(clear_res["configured"], false);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_lane_b_ionex_day_of_year_and_gps_week_math() {
        assert_eq!(calculate_day_of_year(2024, 5, 15).unwrap(), 136);
        assert_eq!(gps_week_and_dow(2024, 5, 15).unwrap(), (2314, 3));
        assert_eq!(gps_week_and_dow(1980, 1, 6).unwrap(), (0, 0));
        assert!(decompress_unix_lzw_z(&[0x00, 0x01, 0x02]).is_err());
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn test_lane_b_ionex_real_unix_lzw_z_decompression_and_zero_synthetic_fallback() {
        let dir = std::env::temp_dir().join(format!(
            "test_lane_b_ionex_z_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 1. Verify exact DOY and GPS week/day-of-week math
        assert_eq!(calculate_day_of_year(2024, 5, 15).unwrap(), 136);
        assert_eq!(gps_week_and_dow(2024, 5, 15).unwrap(), (2314, 3));
        assert_eq!(gps_week_and_dow(1980, 1, 6).unwrap(), (0, 0));

        // 2. Download a REAL `.Z` (Unix LZW `0x1f 0x9d`) IONEX archive from public CODE / SOPAC archive and test `decompress_unix_lzw_z`
        let client = build_http_client(&dir).expect("http client");
        let z_urls = [
            "http://ftp.aiub.unibe.ch/CODE/2024/CODG1360.24I.Z",
            "http://garner.ucsd.edu/pub/products/2314/CODKOPSFIN_20241360000_01D_01H_GIM.INX.Z",
            "http://garner.ucsd.edu/pub/products/2314/codg1360.24i.Z",
        ];
        let mut fetched_z: Option<(String, Vec<u8>)> = None;
        for u in z_urls {
            if let Ok(resp) = client.get(u).timeout(Duration::from_secs(15)).send() {
                if resp.status().is_success() {
                    if let Ok(b) = resp.bytes() {
                        if b.starts_with(&[0x1f, 0x9d]) {
                            fetched_z = Some((u.to_owned(), b.to_vec()));
                            break;
                        }
                    }
                }
            }
        }
        let (z_source_url, z_bytes) = fetched_z
            .expect("Must fetch real 0x1f 0x9d Unix LZW .Z IONEX file from AIUB CODE or SOPAC");
        assert_eq!(
            &z_bytes[..2],
            &[0x1f, 0x9d],
            "Archive from {z_source_url} must be Unix LZW .Z"
        );
        let decompressed = decompress_unix_lzw_z(&z_bytes)
            .expect("Real RFC/Unix compress LZW .Z decompression must succeed");
        let ionex_str = String::from_utf8(decompressed).expect("Decompressed IONEX UTF-8");
        assert!(
            ionex_str.contains("IONEX VERSION / TYPE"),
            "Must contain IONEX VERSION / TYPE header"
        );
        assert!(
            ionex_str.contains("START OF TEC MAP"),
            "Must contain START OF TEC MAP"
        );
        assert!(
            ionex_str.contains("END OF FILE"),
            "Must decompress cleanly through END OF FILE"
        );
        if let Some(dcb) = extract_ionex_dcb_block(&ionex_str) {
            assert!(dcb.contains("DIFFERENTIAL CODE BIASES"));
            assert!(
                !dcb.contains("START OF TEC MAP"),
                "DCB extract must NEVER duplicate full TEC maps"
            );
        }

        // 3. Future date MUST fail honestly (`Err("IONEX_ALL_MIRRORS_FAILED...")`) with ZERO synthetic Klobuchar file
        let future_err = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "ionospheric_ionex".to_owned(),
                code: Some("COD".to_owned()),
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: Some("2099-01-01".to_owned()),
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("future_ionex").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .unwrap_err();
        assert!(
            future_err.contains("IONEX_ALL_MIRRORS_FAILED"),
            "Unavailable IONEX date must fail closed with IONEX_ALL_MIRRORS_FAILED, got: {future_err}"
        );
        assert!(
            !dir.join("future_ionex").join("CODG0010.99I").exists(),
            "Zero synthetic IONEX file may be created when mirrors fail"
        );

        // 4. [HIGH-07] Future date range in `sentinel_2_l1c` MUST return 0 items (never silently substitute Oct 2024 scenes!)
        let s2_future = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "sentinel_2_l1c".to_owned(),
                code: None,
                keyword: None,
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: Some("2099-01-01".to_owned()),
                end_date: Some("2099-01-05".to_owned()),
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("s2_future").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("sentinel_2_l1c future search");
        assert_eq!(
            s2_future["count"].as_u64().unwrap(),
            0,
            "Future date window must return 0 scenes without hardcoded October 2024 fallback"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn test_lane_b_gee_verify_strict_auth_and_collection_mosaic_retry() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dir = std::env::temp_dir().join(format!(
            "test_lane_b_gee_strict_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mpc = LoopbackMpcServer::spawn(&dir);

        // 1. Verify newly mapped canonical GEE assets (`COPERNICUS/S2_HARMONIZED` and `GOOGLE/DYNAMICWORLD/V1`)
        assert_eq!(
            map_canonical_gee_asset("COPERNICUS/S2_HARMONIZED"),
            Some(("mpc_stac_science", "sentinel-2-l2a", "visual"))
        );
        assert_eq!(
            map_canonical_gee_asset("GOOGLE/DYNAMICWORLD/V1"),
            Some(("mpc_stac_science", "io-lulc-annual-v02", "data"))
        );

        let lulc_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("GOOGLE/DYNAMICWORLD/V1".to_owned()),
                keyword: None,
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: Some("2023-01-01".to_owned()),
                end_date: Some("2023-12-31".to_owned()),
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("lulc").to_string_lossy().to_string()),
                full_children: None,
                tool_name: Some("search".to_owned()),
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("GOOGLE/DYNAMICWORLD/V1 Route 1 query");
        assert_eq!(lulc_res["ok"], true);
        assert_eq!(lulc_res["collection"], "io-lulc-annual-v02");
        assert!(lulc_res["count"].as_u64().unwrap() >= 1);
        assert!(lulc_res["tile_url_template"]
            .as_str()
            .unwrap()
            .contains("{z}/{x}/{y}"));

        // 2. `gee_account_verify` MUST reject invalid RSA private key (never swallow error or hit $discovery/rest)
        let bad_sa = json!({
            "type": "service_account",
            "project_id": "ee-invalid-key",
            "client_email": "bad@ee-invalid-key.iam.gserviceaccount.com",
            "private_key": "-----BEGIN PRIVATE KEY-----\nNOT_A_VALID_PKCS8_KEY\n-----END PRIVATE KEY-----\n"
        })
        .to_string();
        let bad_verify_err = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_verify".to_owned(),
                code: Some("ee-invalid-key".to_owned()),
                keyword: Some("service_account".to_owned()),
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: Some(bad_sa),
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .unwrap_err();
        assert!(
            bad_verify_err.contains("GEE_SA_TOKEN_EXCHANGE_FAILED"),
            "Invalid RSA key must fail with GEE_SA_TOKEN_EXCHANGE_FAILED, got: {bad_verify_err}"
        );

        // Prepare a real GeoTIFF payload for mock GEE computePixels response
        let sample_tif_path = dir.join("sample_source.tif");
        let engine = discover_native_gis_engine();
        assert!(
            engine.is_discovered(),
            "Native GIS engine must be discovered for GeoTIFF test"
        );
        let tile_png_path = dir.join("seed_tile.png");
        // Minimal valid 1x1 RGB PNG upsampled / stitched into a real GeoTIFF
        let one_px_png: [u8; 69] = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        fs::write(&tile_png_path, one_px_png).unwrap();
        let wgs84_wkt = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]"#;
        engine
            .stitch_xyz_tiles_to_geotiff(
                &[(0, 0, tile_png_path.clone())],
                0,
                0,
                0,
                0,
                [117.0, 0.6 / 256.0, 0.0, 31.9, 0.0, -(0.1 / 256.0)],
                wgs84_wkt,
                &sample_tif_path,
                "DEFLATE",
                false,
                "gtiff",
            )
            .expect("create sample GeoTIFF");
        let valid_tiff_bytes = fs::read(&sample_tif_path).expect("read sample GeoTIFF");

        // 3. Spin up local mock GEE REST endpoint handling 4 sequential requests:
        drop(mpc);
        fs::write(
            dir.join("network-settings.json"),
            r#"{"mode":"direct","test_loopback":true}"#,
        )
        .unwrap();
        //    Req 1: value:compute with bad token -> 401
        //    Req 2: value:compute with valid token -> 200
        //    Req 3: image:computePixels (Image.load) -> verify dynamic grid 1024x171 -> return 400 ImageCollection error
        //    Req 4: image:computePixels (ImageCollection.mosaic retry) -> return 200 valid GeoTIFF
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");
        let tiff_for_srv = valid_tiff_bytes.clone();

        let srv = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let read_full_http = |stream: &mut std::net::TcpStream| -> String {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                let mut header_end = None;
                let mut content_len = 0usize;
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            data.extend_from_slice(&buf[..n]);
                            if header_end.is_none() {
                                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                                    let hend = pos + 4;
                                    header_end = Some(hend);
                                    let hdr_str = String::from_utf8_lossy(&data[..hend]);
                                    for line in hdr_str.lines() {
                                        if let Some(rest) = line
                                            .to_ascii_lowercase()
                                            .strip_prefix("content-length:")
                                        {
                                            content_len = rest.trim().parse::<usize>().unwrap_or(0);
                                        }
                                    }
                                }
                            }
                            if let Some(hend) = header_end {
                                if data.len() >= hend + content_len {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                String::from_utf8_lossy(&data).into_owned()
            };

            // Req 1: 401 Unauthorized
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let req_txt = read_full_http(&mut s);
                assert!(req_txt.contains("/v1/projects/ee-mock/value:compute"));
                let body = r#"{"error":{"code":401,"message":"Invalid Credentials"}}"#;
                let resp = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }

            // Req 2: 200 OK for valid Bearer token
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let req_txt = read_full_http(&mut s);
                assert!(req_txt.contains("/v1/projects/ee-mock/value:compute"));
                assert!(
                    req_txt.contains("Authorization: Bearer valid-oauth-token")
                        || req_txt.contains("authorization: Bearer valid-oauth-token")
                );
                let body = r#"{"result":1}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }

            // Req 3: Image.load -> verify dynamic metric grid 1024x201 (0.6*cos(31.85°) : 0.1) -> return 400
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let req_txt = read_full_http(&mut s);
                assert!(req_txt.contains("/v1/projects/ee-mock/image:computePixels"));
                assert!(req_txt.contains("\"Image.load\""));
                assert!(req_txt.contains("\"width\":1024") || req_txt.contains("\"width\": 1024"));
                assert!(req_txt.contains("\"height\":201") || req_txt.contains("\"height\": 201"));
                let body = r#"{"error":{"code":400,"message":"Asset is an ImageCollection"}}"#;
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }

            // Req 4: ImageCollection.mosaic retry -> return 200 GeoTIFF
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let req_txt = read_full_http(&mut s);
                assert!(req_txt.contains("\"ImageCollection.mosaic\""));
                assert!(req_txt.contains("\"ImageCollection.load\""));
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    tiff_for_srv.len()
                );
                let _ = s.write_all(hdr.as_bytes());
                let _ = s.write_all(&tiff_for_srv);
            }
        });

        fs::write(
            dir.join("gee_credentials.json"),
            json!({
                "configured": true,
                "project_id": "ee-mock",
                "auth_mode": "oauth_token",
                "api_base_url": base_url,
            })
            .to_string(),
        )
        .unwrap();

        // Test 401 rejection
        store_gee_keyring_bundle("ee-mock", "oauth_token", "expired-token", "").unwrap();
        let err_401 = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_verify".to_owned(),
                code: None,
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .unwrap_err();
        assert!(
            err_401.contains("GEE_VERIFY_HTTP_401"),
            "Expected 401 failure, got: {err_401}"
        );

        // Test 200 success via keyring-stored secret (preserving api_base_url)
        let save_ok = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_save".to_owned(),
                code: Some("ee-mock".to_owned()),
                keyword: Some("oauth_token".to_owned()),
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: Some("valid-oauth-token".to_owned()),
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("save valid token to keyring");
        assert_eq!(save_ok["ok"], true);

        let verify_ok = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_verify".to_owned(),
                code: None,
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("gee_account_verify must pass on authenticated 200");
        assert_eq!(verify_ok["verified"], true);

        // Test Route 2 dynamic grid aspect ratio (6:1 -> 1024x171) + ImageCollection.mosaic retry + PNG preview
        let route2_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("projects/ee-mock/assets/custom_ic".to_owned()),
                keyword: None,
                bbox: Some([117.0, 31.8, 117.6, 31.9]),
                lat: None,
                lng: None,
                start_date: Some("2024-01-01".to_owned()),
                end_date: Some("2024-06-01".to_owned()),
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("route2_out").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("Route 2 ImageCollection.mosaic fallback");
        let _ = srv.join();

        assert_eq!(route2_res["ok"], true);
        assert_eq!(route2_res["expression_mode"], "ImageCollection.mosaic");
        assert_eq!(route2_res["grid_width"], 1024);
        assert_eq!(route2_res["grid_height"], 201);
        assert!(
            route2_res["preview_data_url"]
                .as_str()
                .unwrap_or("")
                .starts_with("data:image/png;base64,"),
            "Route 2 GeoTIFF must include PNG preview_data_url for WorkbenchMap ImageOverlay"
        );

        clear_gee_keyring_bundle_for_project(Some("ee-mock"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn test_lane_b_satellite_tiles_port_parser_mbtiles_zip_cog_webp_and_preview() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let dir = std::env::temp_dir().join(format!(
            "test_lane_b_tiles_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("network-settings.json"),
            r#"{"mode":"direct","test_loopback":true}"#,
        )
        .unwrap();

        // Valid 1x1 RGB PNG served for every tile request on localhost:{port}
        let png_tile: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let tile_bg = png_tile.clone();

        let srv = std::thread::spawn(move || {
            use std::io::{Read, Write};
            while !stop_bg.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut buf = [0u8; 2048];
                        let _ = stream.read(&mut buf);
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            tile_bg.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&tile_bg);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        // 1. Test `http://127.0.0.1:{port}/tiles/{z}/{x}/{y}.png:20:mosaic:cog:WEBP:pyramids`
        //    - `:port` must NOT be split by spec parser
        //    - Z20 AOI spanning >2x2 tiles must NOT be clamped to 4 tiles (`max_axis_span == 7`, `tiles_clamped == false`)
        //    - Native GDAL COG + WEBP compression + PNG map preview (`preview_data_url` + `mosaic_bbox`)
        let cog_dir = dir.join("cog_webp_z20");
        let cog_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                code: None,
                keyword: Some(format!(
                    "http://127.0.0.1:{port}/tiles/{{z}}/{{x}}/{{y}}.png:20:mosaic:cog:WEBP:pyramids"
                )),
                bbox: Some([117.2800, 31.8600, 117.2810, 31.8609]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(cog_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
            aoi_geojson: None,
            zooms: None,
            },
        )
        .expect("Z20 COG + WEBP mosaic with localhost:port URL");

        assert_eq!(cog_res["ok"], true);
        let z20_count = cog_res["tile_count"].as_u64().unwrap();
        assert!(
            z20_count > 4,
            "Z20 max_axis_span=7 must allow >4 tiles (got {z20_count})"
        );
        assert_eq!(cog_res["tiles_clamped"], false);
        assert!(
            cog_res["preview_data_url"]
                .as_str()
                .unwrap_or("")
                .starts_with("data:image/png;base64,"),
            "Stitched mosaic must emit preview_data_url"
        );
        assert_eq!(cog_res["mosaic_bbox"].as_array().unwrap().len(), 4);

        let cog_tif = PathBuf::from(cog_res["downloaded_file"].as_str().unwrap());
        assert!(cog_tif.is_file());
        let inspect = execute_gis_raster_inspect(&GisRasterInspectRequest {
            source: cog_tif.to_string_lossy().to_string(),
        })
        .expect("GDAL inspect COG WEBP output");
        println!("COG+WEBP GDAL inspect metadata: {}", inspect["metadata"]);
        assert_eq!(inspect["metadata"]["driver"], "GTiff");
        assert_eq!(inspect["metadata"]["layout"], "COG");
        assert_eq!(inspect["metadata"]["compression"], "WEBP");
        assert!(inspect["metadata"]["count"].as_u64().unwrap() >= 3);
        if let Ok(gdalinfo_out) = std::process::Command::new("gdalinfo")
            .arg(&cog_tif)
            .output()
        {
            if gdalinfo_out.status.success() {
                let info_txt = String::from_utf8_lossy(&gdalinfo_out.stdout);
                println!(
                    "gdalinfo COG+WEBP excerpt:\n{}",
                    info_txt.lines().take(28).collect::<Vec<_>>().join("\n")
                );
                assert!(info_txt.contains("LAYOUT=COG"));
                assert!(info_txt.contains("COMPRESSION=WEBP"));
            }
        }

        // 2. Test MBTiles 1.3 SQLite export (`metadata` + TMS `tiles`)
        let mbt_dir = dir.join("mbtiles_z19");
        let mbt_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                code: None,
                keyword: Some(format!(
                    "http://127.0.0.1:{port}/tiles/{{z}}/{{x}}/{{y}}.png:19:mosaic:mbtiles:DEFLATE"
                )),
                bbox: Some([117.2800, 31.8600, 117.2806, 31.8605]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(mbt_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("MBTiles 1.3 export");
        let mbt_path = PathBuf::from(mbt_res["mbtiles_path"].as_str().expect("mbtiles_path"));
        assert!(mbt_path.is_file());
        assert_eq!(
            mbt_res["downloaded_file"].as_str().unwrap(),
            mbt_path.to_string_lossy()
        );
        let conn = rusqlite::Connection::open(&mbt_path).expect("open mbtiles sqlite");
        let ver: String = conn
            .query_row("SELECT value FROM metadata WHERE name='version'", [], |r| {
                r.get(0)
            })
            .expect("metadata version");
        assert_eq!(ver, "1.3");
        let row_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tiles WHERE zoom_level=19", [], |r| {
                r.get(0)
            })
            .expect("tiles count");
        assert_eq!(row_count as u64, mbt_res["tile_count"].as_u64().unwrap());
        let first_xyz_y = mbt_res["tiles"][0]["y"].as_u64().unwrap() as i64;
        let first_xyz_x = mbt_res["tiles"][0]["x"].as_u64().unwrap() as i64;
        let expected_tms_y = (1i64 << 19) - 1 - first_xyz_y;
        let blob_len: i64 = conn
            .query_row(
                "SELECT length(tile_data) FROM tiles WHERE zoom_level=19 AND tile_column=?1 AND tile_row=?2",
                rusqlite::params![first_xyz_x, expected_tms_y],
                |r| r.get(0),
            )
            .expect("TMS flipped tile_row must exist in tiles table");
        assert!(blob_len > 0);

        // 3. Test ZIP archive export (`PK\x03\x04` magic + `{z}/{x}/{y}.png` + `.tif` + `tiles_manifest.json`)
        let zip_dir = dir.join("zip_z19");
        let zip_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                code: None,
                keyword: Some(format!(
                    "http://127.0.0.1:{port}/tiles/{{z}}/{{x}}/{{y}}.png:19:mosaic:zip:DEFLATE"
                )),
                bbox: Some([117.2800, 31.8600, 117.2806, 31.8605]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(zip_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("ZIP archive export");
        let zip_path = PathBuf::from(
            zip_res["zip_archive_path"]
                .as_str()
                .expect("zip_archive_path"),
        );
        assert!(zip_path.is_file());
        assert_eq!(
            zip_res["downloaded_file"].as_str().unwrap(),
            zip_path.to_string_lossy()
        );
        let zip_raw = fs::read(&zip_path).expect("read zip file");
        assert_eq!(
            &zip_raw[..4],
            b"PK\x03\x04",
            "ZIP archive must start with PK\\x03\\x04 magic"
        );
        let zf = fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(zf).expect("open valid ZipArchive");
        let mut names = Vec::new();
        for i in 0..archive.len() {
            names.push(archive.by_index(i).unwrap().name().unwrap().into_owned());
        }
        assert!(
            names.iter().any(|n| n == "tiles_manifest.json"),
            "ZIP must contain tiles_manifest.json"
        );
        assert!(
            names.iter().any(|n| n.ends_with(".tif")),
            "ZIP must contain stitched .tif"
        );
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("19/") && n.ends_with(".png")),
            "ZIP must contain 19/{{x}}/{{y}}.png"
        );

        stop.store(true, Ordering::Relaxed);
        let _ = srv.join();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_gee_isolated_empty_keychain_mode_preserves_existing_credentials() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dir = std::env::temp_dir().join(format!(
            "test_gee_isolated_empty_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 1. Seed a real GEE account into keyring + metadata file
        let save_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_save".to_owned(),
                code: Some("ee-owner-preserved".to_owned()),
                keyword: Some("oauth_token".to_owned()),
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: Some("ya29.owner-live-secret-do-not-touch".to_owned()),
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("seed owner gee account");
        assert_eq!(save_res["configured"], true);

        assert!(parse_isolated_empty_keychain_flag("1"));
        assert!(parse_isolated_empty_keychain_flag(" TRUE "));
        assert!(!parse_isolated_empty_keychain_flag("0"));
        assert!(!parse_isolated_empty_keychain_flag(""));

        struct IsolateReset;
        impl Drop for IsolateReset {
            fn drop(&mut self) {
                TEST_ISOLATE_EMPTY_KEYCHAIN_OVERRIDE
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        {
            let _reset = IsolateReset;
            TEST_ISOLATE_EMPTY_KEYCHAIN_OVERRIDE.store(true, std::sync::atomic::Ordering::SeqCst);

            // Status MUST appear completely unconfigured (zero project_id / email leakage)
            let iso_status = execute_open_geodata_query(
                &dir,
                &OpenGeodataQueryRequest {
                    dataset: "gee_account_status".to_owned(),
                    code: None,
                    keyword: None,
                    bbox: None,
                    lat: None,
                    lng: None,
                    start_date: None,
                    end_date: None,
                    osm_tag: None,
                    dem_product: None,
                    iso3: None,
                    year: None,
                    output_path: None,
                    full_children: None,
                    tool_name: None,
                    tool_args: None,
                    sql: None,
                    gee_params: None,
                    aoi_geojson: None,
                    zooms: None,
                },
            )
            .expect("isolated gee_account_status");
            assert_eq!(iso_status["configured"], false);
            assert_eq!(iso_status["has_secret"], false);
            assert_eq!(iso_status["project_id"], "");

            // Reveal, Save, and Clear MUST all fail closed without mutating disk or keyring
            let reveal_err = execute_open_geodata_query(
                &dir,
                &OpenGeodataQueryRequest {
                    dataset: "gee_account_reveal".to_owned(),
                    code: None,
                    keyword: None,
                    bbox: None,
                    lat: None,
                    lng: None,
                    start_date: None,
                    end_date: None,
                    osm_tag: None,
                    dem_product: None,
                    iso3: None,
                    year: None,
                    output_path: None,
                    full_children: None,
                    tool_name: None,
                    tool_args: None,
                    sql: None,
                    gee_params: None,
                    aoi_geojson: None,
                    zooms: None,
                },
            )
            .unwrap_err();
            assert!(reveal_err.contains("GEE_ISOLATED_EMPTY_KEYCHAIN_READONLY"));

            let clear_err = execute_open_geodata_query(
                &dir,
                &OpenGeodataQueryRequest {
                    dataset: "gee_account_clear".to_owned(),
                    code: None,
                    keyword: None,
                    bbox: None,
                    lat: None,
                    lng: None,
                    start_date: None,
                    end_date: None,
                    osm_tag: None,
                    dem_product: None,
                    iso3: None,
                    year: None,
                    output_path: None,
                    full_children: None,
                    tool_name: None,
                    tool_args: None,
                    sql: None,
                    gee_params: None,
                    aoi_geojson: None,
                    zooms: None,
                },
            )
            .unwrap_err();
            assert!(clear_err.contains("GEE_ISOLATED_EMPTY_KEYCHAIN_READONLY"));
        }

        // 2. Once isolated mode exits, the original GEE credential + metadata file remain 100% intact
        let restored_status = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_status".to_owned(),
                code: None,
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("restored gee_account_status");
        assert_eq!(restored_status["configured"], true);
        assert_eq!(restored_status["project_id"], "ee-owner-preserved");

        let revealed = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_reveal".to_owned(),
                code: None,
                keyword: None,
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("restored gee_account_reveal");
        assert_eq!(revealed["value"], "ya29.owner-live-secret-do-not-touch");

        clear_gee_keyring_bundle_for_project(Some("ee-owner-preserved"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn test_gee_v1_expression_dag_route2_search_download_and_scutil_proxy() {
        // 1. Verify macOS scutil --proxy parser priority (HTTPS -> HTTP -> SOCKS5 -> disabled)
        let scutil_https = "\
<dictionary> {
  ExceptionsList : <array> {
    0 : 127.0.0.1
    1 : localhost
  }
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7897
  HTTPSProxy : 127.0.0.1
}";
        assert_eq!(
            parse_scutil_proxy_output(scutil_https).as_deref(),
            Some("http://127.0.0.1:7897")
        );
        let scutil_disabled = "<dictionary> {\n  HTTPEnable : 0\n  HTTPSEnable : 0\n}\n";
        assert_eq!(parse_scutil_proxy_output(scutil_disabled), None);

        // 2. Verify Expression DAG shapes match Google Earth Engine REST v1 schema
        let expr_one = gee_expr_constant_one();
        assert_eq!(expr_one["result"], "0");
        assert_eq!(expr_one["values"]["0"]["constantValue"], 1);
        assert!(expr_one.get("constantValue").is_none());

        let expr_img = gee_expr_image_load("USGS/SRTMGL1_003");
        assert_eq!(expr_img["result"], "0");
        assert_eq!(
            expr_img["values"]["0"]["functionInvocationValue"]["functionName"],
            "Image.load"
        );

        let expr_coll = gee_expr_collection_mosaic("COPERNICUS/DEM/GLO30");
        assert_eq!(expr_coll["result"], "0");
        assert_eq!(
            expr_coll["values"]["0"]["functionInvocationValue"]["functionName"],
            "ImageCollection.mosaic"
        );
        assert_eq!(
            expr_coll["values"]["0"]["functionInvocationValue"]["arguments"]["collection"]
                ["valueReference"],
            "1"
        );
        assert_eq!(
            expr_coll["values"]["1"]["functionInvocationValue"]["functionName"],
            "ImageCollection.load"
        );

        let map_payload = build_gee_map_payload(
            &expr_img,
            Some(&["B4".to_owned(), "B3".to_owned(), "B2".to_owned()]),
            0.0,
            3000.0,
        );
        assert_eq!(map_payload["fileFormat"], "AUTO_JPEG_PNG");
        assert_eq!(map_payload["bandIds"], json!(["B4", "B3", "B2"]));
        assert!(map_payload["visualizationOptions"].get("bands").is_none());

        // 3. Verify Route 2 `|search` does NOT write .tif, and `|download` homogenizes mixed-dtype bands via Image.bandNames
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dir = std::env::temp_dir().join(format!(
            "test_gee_v1_dag_route2_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("network-settings.json"),
            r#"{"mode":"direct","test_loopback":true}"#,
        )
        .unwrap();

        let sample_tif_path = dir.join("sample_source.tif");
        let engine = discover_native_gis_engine();
        let tile_png_path = dir.join("seed_tile.png");
        let one_px_png: [u8; 69] = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        fs::write(&tile_png_path, one_px_png).unwrap();
        let wgs84_wkt = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]"#;
        engine
            .stitch_xyz_tiles_to_geotiff(
                &[(0, 0, tile_png_path)],
                0,
                0,
                0,
                0,
                [117.25, 0.03 / 256.0, 0.0, 31.87, 0.0, -(0.03 / 256.0)],
                wgs84_wkt,
                &sample_tif_path,
                "DEFLATE",
                false,
                "gtiff",
            )
            .expect("create sample GeoTIFF");
        let valid_tiff_bytes = fs::read(&sample_tif_path).unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");
        let tiff_for_srv = valid_tiff_bytes.clone();

        let srv = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let read_full_http = |stream: &mut std::net::TcpStream| -> (String, Value) {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                let mut header_end = None;
                let mut content_len = 0usize;
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            data.extend_from_slice(&buf[..n]);
                            if header_end.is_none() {
                                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                                    let hend = pos + 4;
                                    header_end = Some(hend);
                                    let hdr_str = String::from_utf8_lossy(&data[..hend]);
                                    for line in hdr_str.lines() {
                                        if let Some(rest) = line
                                            .to_ascii_lowercase()
                                            .strip_prefix("content-length:")
                                        {
                                            content_len = rest.trim().parse::<usize>().unwrap_or(0);
                                        }
                                    }
                                }
                            }
                            if let Some(hend) = header_end {
                                if data.len() >= hend + content_len {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let full = String::from_utf8_lossy(&data).into_owned();
                let body_val = header_end
                    .and_then(|hend| serde_json::from_slice::<Value>(&data[hend..]).ok())
                    .unwrap_or(json!({}));
                (full, body_val)
            };

            // Req 1 (Search): value:compute Image.bandNames on Image.load -> 400 ImageCollection
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let (req_txt, body_json) = read_full_http(&mut s);
                assert!(req_txt.contains("/v1/projects/ee-dag/value:compute"));
                assert_eq!(body_json["expression"]["result"], "bands_out");
                assert_eq!(
                    body_json["expression"]["values"]["0"]["functionInvocationValue"]
                        ["functionName"],
                    "Image.load"
                );
                let body = r#"{"error":{"code":400,"message":"Asset is an ImageCollection"}}"#;
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }

            // Req 2 (Search retry): value:compute Image.bandNames on filtered ImageCollection.mosaic -> 200 bands
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let (_req_txt, body_json) = read_full_http(&mut s);
                assert_eq!(body_json["expression"]["result"], "bands_out");
                assert_eq!(
                    body_json["expression"]["values"]["0"]["functionInvocationValue"]
                        ["functionName"],
                    "ImageCollection.mosaic"
                );
                assert_eq!(
                    body_json["expression"]["values"]["2"]["functionInvocationValue"]
                        ["functionName"],
                    "Collection.filter"
                );
                assert_eq!(
                    body_json["expression"]["values"]["3"]["functionInvocationValue"]
                        ["functionName"],
                    "Filter.dateRangeContains"
                );
                assert_eq!(
                    body_json["expression"]["values"]["4"]["functionInvocationValue"]
                        ["functionName"],
                    "DateRange"
                );
                assert_eq!(
                    body_json["expression"]["values"]["4"]["functionInvocationValue"]["arguments"]
                        ["start"]["constantValue"],
                    "2024-01-01T00:00:00Z"
                );
                assert_eq!(
                    body_json["expression"]["values"]["4"]["functionInvocationValue"]["arguments"]
                        ["end"]["constantValue"],
                    "2024-02-01T23:59:59.999Z"
                );
                let body = r#"{"result":["B1","B2","B3","B4","MSK_CLDPRB"]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }

            // Req 3 (Download attempt 1): image:computePixels Image.load -> 400 ImageCollection
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let (req_txt, body_json) = read_full_http(&mut s);
                assert!(req_txt.contains("/v1/projects/ee-dag/image:computePixels"));
                assert_eq!(body_json["expression"]["result"], "0");
                let body = r#"{"error":{"code":400,"message":"Asset is an ImageCollection"}}"#;
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }

            // Req 4 (Download attempt 2): image:computePixels ImageCollection.mosaic without bandIds -> 400 mixed dtype
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let (_req_txt, body_json) = read_full_http(&mut s);
                assert!(body_json.get("bandIds").is_none());
                let body = r#"{"error":{"code":400,"message":"All bands must have compatible data types."}}"#;
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }

            // Req 5 (Download band homogenization lookup): value:compute Image.bandNames -> 200
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let (req_txt, _body_json) = read_full_http(&mut s);
                assert!(req_txt.contains("/v1/projects/ee-dag/value:compute"));
                let body = r#"{"result":["B1","B2","B3","B4","MSK_CLDPRB"]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }

            // Req 6 (Download attempt 3 with homogeneous bandIds): image:computePixels -> 200 GeoTIFF
            if let Some(mut s) = accept_with_timeout(&listener, Duration::from_secs(10)) {
                let (_req_txt, body_json) = read_full_http(&mut s);
                assert_eq!(body_json["bandIds"], json!(["B4", "B3", "B2"]));
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    tiff_for_srv.len()
                );
                let _ = s.write_all(hdr.as_bytes());
                let _ = s.write_all(&tiff_for_srv);
            }
        });

        fs::write(
            dir.join("gee_credentials.json"),
            json!({
                "configured": true,
                "project_id": "ee-dag",
                "auth_mode": "oauth_token",
                "api_base_url": base_url,
            })
            .to_string(),
        )
        .unwrap();
        store_gee_keyring_bundle("ee-dag", "oauth_token", "valid-dag-token", "").unwrap();

        let search_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("COPERNICUS/S2_SR_HARMONIZED".to_owned()),
                keyword: Some(
                    "COPERNICUS/S2_SR_HARMONIZED|2024-01-01..2024-02-01|search".to_owned(),
                ),
                bbox: Some([117.25, 31.84, 117.28, 31.87]),
                lat: None,
                lng: None,
                start_date: Some("2024-01-01".to_owned()),
                end_date: Some("2024-02-01".to_owned()),
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: Some("gee_official_rest".to_owned()),
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("Route 2 search_only must succeed without writing GeoTIFF");
        assert_eq!(search_res["ok"], true);
        assert_eq!(search_res["search_only"], true);
        assert_eq!(search_res["count"], 1);
        assert_eq!(search_res["items"][0]["already_downloaded"], false);
        assert!(search_res.get("downloaded_file").is_none());
        assert!(
            !dir.join("geodata_downloads")
                .join("gee_exports")
                .join("GEE_COPERNICUS_S2_SR_HARMONIZED.tif")
                .exists(),
            "Search must NEVER download .tif to gee_exports"
        );

        let dl_out_dir = dir.join("explicit_gee_dl");
        let dl_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("COPERNICUS/S2_SR_HARMONIZED".to_owned()),
                keyword: Some(
                    "COPERNICUS/S2_SR_HARMONIZED|2024-01-01..2024-02-01|download".to_owned(),
                ),
                bbox: Some([117.25, 31.84, 117.28, 31.87]),
                lat: None,
                lng: None,
                start_date: Some("2024-01-01".to_owned()),
                end_date: Some("2024-02-01".to_owned()),
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dl_out_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: Some("download".to_owned()),
                tool_args: None,
                sql: Some("gee_official_rest".to_owned()),
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        )
        .expect("Route 2 download with mixed-dtype bandIds homogenization must succeed");
        let _ = srv.join();

        assert_eq!(dl_res["ok"], true);
        assert_eq!(dl_res["items"][0]["already_downloaded"], true);
        let saved_tif = PathBuf::from(
            dl_res["downloaded_file"]
                .as_str()
                .expect("downloaded_file path must be present"),
        );
        let saved_name = saved_tif.file_name().unwrap().to_string_lossy();
        assert!(
            saved_name.starts_with("GEE_COPERNICUS_S2_SR_HARMONIZED_")
                && saved_name.ends_with(".tif"),
            "Expected AOI/date-scoped filename, got {saved_name}"
        );
        assert!(
            saved_tif.is_file(),
            "Explicit download must write physical GeoTIFF"
        );
        assert!(fs::metadata(&saved_tif).unwrap().len() > 0);

        clear_gee_keyring_bundle_for_project(Some("ee-dag"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_gee_route2_aoi_date_scoped_filename_and_adc_secret_guard() {
        let _keyring_guard = GeeKeyringTestGuard::acquire();
        let dir = std::env::temp_dir().join(format!(
            "gee_scope_and_adc_guard_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        clear_gee_keyring_bundle();

        // 1. Verify cross-AOI and cross-date non-collision in the same output folder
        let (id_hefei, file_hefei) = compute_gee_route2_scope_id(
            "USGS/SRTMGL1_003",
            "USGS_SRTMGL1_003",
            117.20,
            31.80,
            117.30,
            31.90,
            Some("2024-05-01"),
            Some("2024-10-01"),
            None,
        );
        let (id_chengdu, file_chengdu) = compute_gee_route2_scope_id(
            "USGS/SRTMGL1_003",
            "USGS_SRTMGL1_003",
            104.05,
            30.65,
            104.15,
            30.75,
            Some("2024-05-01"),
            Some("2024-10-01"),
            None,
        );
        let (id_hefei_win2, file_hefei_win2) = compute_gee_route2_scope_id(
            "USGS/SRTMGL1_003",
            "USGS_SRTMGL1_003",
            117.20,
            31.80,
            117.30,
            31.90,
            Some("2024-10-02"),
            Some("2024-12-01"),
            None,
        );
        assert_ne!(id_hefei, id_chengdu);
        assert_ne!(file_hefei, file_chengdu);
        assert_ne!(id_hefei, id_hefei_win2);
        assert_ne!(file_hefei, file_hefei_win2);

        // Static DEM collection skips DateRange filter, temporal collection emits DateRange filter
        let static_dag = gee_expr_collection_mosaic_filtered(
            "COPERNICUS/DEM/GLO30",
            Some("2026-08-01"),
            Some("2026-11-12"),
        );
        assert!(static_dag["values"].get("2").is_none());

        assert!(is_static_gee_image_collection(
            "projects/climate-and-sustainability/assets/gem-15_v1"
        ));
        let gem15_static_dag = gee_expr_collection_mosaic_filtered(
            "projects/climate-and-sustainability/assets/gem-15_v1",
            Some("2026-01-01"),
            Some("2026-11-15"),
        );
        assert!(gem15_static_dag["values"].get("2").is_none());
        assert_eq!(
            select_homogeneous_gee_bands(&[
                "dsm".to_owned(),
                "dsm_percentile_5".to_owned(),
                "dsm_percentile_50".to_owned(),
                "dsm_percentile_95".to_owned(),
                "dsm_std".to_owned(),
                "dtm".to_owned(),
            ]),
            Some(vec!["dtm".to_owned()])
        );
        let temporal_dag = gee_expr_collection_mosaic_filtered(
            "COPERNICUS/S2_SR_HARMONIZED",
            Some("2024-05-01"),
            Some("2024-10-01"),
        );
        assert_eq!(
            temporal_dag["values"]["2"]["functionInvocationValue"]["functionName"],
            "Collection.filter"
        );
        let inverted_dag = gee_expr_collection_mosaic_filtered(
            "COPERNICUS/S2_SR_HARMONIZED",
            Some("2024-10-01"),
            Some("2024-05-01"),
        );
        assert_eq!(
            inverted_dag["values"]["4"]["functionInvocationValue"]["arguments"]["start"]
                ["constantValue"],
            "2024-05-01T00:00:00Z"
        );
        assert_eq!(
            inverted_dag["values"]["4"]["functionInvocationValue"]["arguments"]["end"]
                ["constantValue"],
            "2024-10-01T23:59:59.999Z"
        );
        assert_eq!(
            sanitize_gee_route2_safe_name("USGS/SRTMGL1_003"),
            "USGS_SRTMGL1_003"
        );
        let sanitized_traversal =
            sanitize_gee_route2_safe_name(r"..\..\projects\evil:stream/asset*?");
        assert!(!sanitized_traversal.contains(".."));
        assert!(!sanitized_traversal.contains('\\'));
        assert!(!sanitized_traversal.contains('/'));
        assert!(!sanitized_traversal.contains(':'));

        // 2. Verify authorized_user JSON missing client_secret is rejected without hardcoded fallback
        let client = build_http_client(&dir).unwrap();
        let err = resolve_gee_bearer_token(
            &dir,
            &client,
            r#"{"type":"authorized_user","refresh_token":"1//fake-refresh","client_id":"123.apps.googleusercontent.com"}"#,
        )
        .expect_err("authorized_user without client_secret must fail closed");
        assert!(err.contains("INVALID_SA_JSON") && err.contains("client_secret"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_loopback_mpc_base_url_enforces_strict_loopback_http_only_in_debug() {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!(
            "mpc_loopback_guard_{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let net = dir.join("network-settings.json");

        fs::write(&net, r#"{"mpc_test_base_url":"http://127.0.0.1:18423/"}"#).unwrap();
        assert_eq!(
            read_loopback_mpc_base_url(&dir).as_deref(),
            Some("http://127.0.0.1:18423")
        );

        fs::write(&net, r#"{"mpc_test_base_url":"http://[::1]:18423/"}"#).unwrap();
        assert_eq!(
            read_loopback_mpc_base_url(&dir).as_deref(),
            Some("http://[::1]:18423")
        );

        for rejected in [
            r#"{"mpc_test_base_url":"https://127.0.0.1:18423"}"#,
            r#"{"mpc_test_base_url":"http://127.0.0.1"}"#,
            r#"{"mpc_test_base_url":"http://localhost:18423"}"#,
            r#"{"mpc_test_base_url":"http://192.168.1.10:18423"}"#,
            r#"{"mpc_test_base_url":"http://user:pass@127.0.0.1:18423"}"#,
        ] {
            fs::write(&net, rejected).unwrap();
            assert_eq!(
                read_loopback_mpc_base_url(&dir),
                None,
                "must reject {rejected}"
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_gee_local_vault_roundtrip_across_status_reveal_verify_route2_and_clear() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let _guard = GeeKeyringTestGuard::acquire();
        clear_gee_keyring_bundle();

        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!(
            "gee_local_vault_roundtrip_{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let srv = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            let mut served = 0usize;
            while std::time::Instant::now() < deadline && served < 2 {
                if let Ok((mut s, _)) = listener.accept() {
                    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buf = [0u8; 4096];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let req_txt = String::from_utf8_lossy(&buf[..n]);
                    assert!(req_txt.contains("/v1/projects/ee-vault-proj/value:compute"));
                    assert!(
                        req_txt.contains("Authorization: Bearer vault-oauth-secret-xyz")
                            || req_txt.contains("authorization: Bearer vault-oauth-secret-xyz")
                    );
                    let body = if served == 0 {
                        r#"{"result":1}"#
                    } else {
                        r#"{"result":["elevation"]}"#
                    };
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = s.write_all(resp.as_bytes());
                    served += 1;
                } else {
                    std::thread::sleep(Duration::from_millis(15));
                }
            }
        });

        // Write ONLY local `credentials-vault.v1.json` + metadata `api_base_url` (empty OS keyring)
        let vault_doc = json!({
            "eostage:agent-model:gee-credentials": {
                "gee-credentials": json!({
                    "project_id": "ee-vault-proj",
                    "auth_mode": "oauth_token",
                    "secret": "vault-oauth-secret-xyz",
                }).to_string()
            },
            "eostage:cdse": {
                "username": "keep-other-credentials@example.com"
            }
        });
        fs::write(
            dir.join("credentials-vault.v1.json"),
            serde_json::to_vec_pretty(&vault_doc).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("network-settings.json"),
            r#"{"proxy_enabled":false,"proxy_url":"","test_loopback":true}"#,
        )
        .unwrap();
        fs::write(
            dir.join("gee_credentials.json"),
            json!({
                "configured": true,
                "project_id": "ee-vault-proj",
                "auth_mode": "oauth_token",
                "api_base_url": base_url,
            })
            .to_string(),
        )
        .unwrap();

        // 1. status resolves configured=true
        let st = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_status".to_owned(),
                ..Default::default()
            },
        )
        .expect("gee_account_status ok");
        assert_eq!(st["configured"], true);
        assert_eq!(st["project_id"], "ee-vault-proj");

        // 2. reveal resolves secret from local vault
        let rev = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_reveal".to_owned(),
                ..Default::default()
            },
        )
        .expect("gee_account_reveal from local vault ok");
        assert_eq!(rev["value"], "vault-oauth-secret-xyz");

        // 3. verify authenticates against loopback GEE server using local vault secret
        let ver = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_verify".to_owned(),
                ..Default::default()
            },
        )
        .expect("gee_account_verify from local vault ok");
        assert_eq!(ver["verified"], true);

        // 4. Route 2 search_only succeeds using local vault secret (without GEE_ACCOUNT_REQUIRED)
        let r2 = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("projects/ee-vault-proj/assets/custom_dem".to_owned()),
                keyword: Some("search".to_owned()),
                bbox: Some([117.2, 31.8, 117.3, 31.9]),
                ..Default::default()
            },
        )
        .expect("Route 2 search_only with local vault ok");
        assert_eq!(r2["ok"], true);
        assert_eq!(r2["route_mode"], "gee_official_rest");
        assert_eq!(r2["bands"][0], "elevation");
        let _ = srv.join();

        // 5. clear removes gee_credentials.json AND scrubs GEE keys from isolated local vault while preserving non-GEE keys
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                dir.join("credentials-vault.v1.json"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }
        let clr = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_clear".to_owned(),
                ..Default::default()
            },
        )
        .expect("gee_account_clear ok");
        assert_eq!(clr["configured"], false);

        let st_after = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_status".to_owned(),
                ..Default::default()
            },
        )
        .expect("status after clear ok");
        assert_eq!(st_after["configured"], false);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("credentials-vault.v1.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(
                mode, 0o600,
                "scrubbed credentials-vault.v1.json must be rewritten with 0o600 permissions, got {mode:o}"
            );
        }
        let leftover_tmps: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(
            leftover_tmps.is_empty(),
            "atomic vault scrub must leave zero .tmp-* files, found: {leftover_tmps:?}"
        );

        let remaining_vault: Value = serde_json::from_str(
            &fs::read_to_string(dir.join("credentials-vault.v1.json")).unwrap(),
        )
        .unwrap();
        assert!(remaining_vault
            .get("eostage:agent-model:gee-credentials")
            .is_none());
        assert_eq!(
            remaining_vault["eostage:cdse"]["username"],
            "keep-other-credentials@example.com"
        );

        // 6. Concurrent scrubs serialize cleanly and refuse to overwrite or follow a symlinked vault
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let conc_dir = dir.join("concurrent_vault");
            fs::create_dir_all(&conc_dir).unwrap();
            fs::write(
                conc_dir.join("credentials-vault.v1.json"),
                serde_json::to_vec_pretty(&json!({
                    "eostage:earthengine": { "token": "drop-1" },
                    "eostage:agent-model:gee-credentials": { "gee-credentials": "drop-2" },
                    "eostage:agent-model:gee-proj-demo": { "secret": "drop-3" },
                    "eostage:earthdata": { "username": "preserve-earthdata" }
                }))
                .unwrap(),
            )
            .unwrap();
            let mut handles = Vec::new();
            for _ in 0..4 {
                let d = conc_dir.clone();
                handles.push(std::thread::spawn(move || {
                    scrub_gee_entries_from_local_vault(&d);
                }));
            }
            for h in handles {
                h.join().unwrap();
            }
            let c_mode = fs::metadata(conc_dir.join("credentials-vault.v1.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(c_mode, 0o600);
            let c_doc: Value = serde_json::from_str(
                &fs::read_to_string(conc_dir.join("credentials-vault.v1.json")).unwrap(),
            )
            .unwrap();
            assert!(c_doc.get("eostage:earthengine").is_none());
            assert_eq!(c_doc["eostage:earthdata"]["username"], "preserve-earthdata");

            // Symlinked vault target must never be replaced or modified
            let sym_dir = dir.join("symlink_vault");
            fs::create_dir_all(&sym_dir).unwrap();
            let real_outside = dir.join("outside_real_vault.json");
            let sentinel = serde_json::to_vec_pretty(&json!({
                "eostage:earthengine": { "token": "untouched-through-symlink" }
            }))
            .unwrap();
            fs::write(&real_outside, &sentinel).unwrap();
            std::os::unix::fs::symlink(&real_outside, sym_dir.join("credentials-vault.v1.json"))
                .unwrap();
            scrub_gee_entries_from_local_vault(&sym_dir);
            assert_eq!(fs::read(&real_outside).unwrap(), sentinel);
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_open_geodata_items_retains_edge_straddling_and_excludes_disjoint() {
        // Irregular L-shaped AOI: [110, 18]..[112, 22] union [110, 18]..[115, 20]
        // Envelope is [110, 18, 115, 22], leaving northeast quadrant [112, 20]..[115, 22] empty.
        let aoi = json!({
            "type": "Polygon",
            "coordinates": [[
                [110.0, 18.0],
                [115.0, 18.0],
                [115.0, 20.0],
                [112.0, 20.0],
                [112.0, 22.0],
                [110.0, 22.0],
                [110.0, 18.0]
            ]]
        });
        let mut items = vec![
            // 1. Interior scene
            json!({
                "id": "interior_scene",
                "bbox": [110.5, 18.5, 111.5, 19.5]
            }),
            // 2. Edge-straddling scene: straddles eastern boundary [114.5..116.0, 18.5..19.5]
            json!({
                "id": "edge_straddling_east",
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [114.5, 18.5],
                        [116.0, 18.5],
                        [116.0, 19.5],
                        [114.5, 19.5],
                        [114.5, 18.5]
                    ]]
                }
            }),
            // 3. Edge-straddling concave corner scene: [111.8..113.2, 19.8..21.2]
            json!({
                "id": "edge_straddling_concave",
                "bbox": [111.8, 19.8, 113.2, 21.2]
            }),
            // 4. Disjoint scene inside envelope concavity [112.5..114.5, 20.5..21.8]
            json!({
                "id": "disjoint_in_concavity",
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [112.5, 20.5],
                        [114.5, 20.5],
                        [114.5, 21.8],
                        [112.5, 21.8],
                        [112.5, 20.5]
                    ]]
                }
            }),
        ];

        filter_open_geodata_items_by_aoi(&mut items, Some(&aoi));
        let ids: Vec<&str> = items
            .iter()
            .filter_map(|v| v.get("id").and_then(Value::as_str))
            .collect();
        assert_eq!(
            ids,
            vec![
                "interior_scene",
                "edge_straddling_east",
                "edge_straddling_concave"
            ]
        );
    }

    #[test]
    fn test_xyz_tiles_multi_zoom_over_64_concurrent_with_progress_and_mbtiles() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        let dir = std::env::temp_dir().join(format!(
            "test_xyz_multizoom_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("network-settings.json"),
            r#"{"mode":"direct","test_loopback":true}"#,
        )
        .unwrap();

        let png_tile: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let req_count = Arc::new(AtomicUsize::new(0));
        let stop_bg = Arc::clone(&stop);
        let count_bg = Arc::clone(&req_count);
        let tile_bg = png_tile;

        let srv = std::thread::spawn(move || {
            use std::io::{Read, Write};
            while !stop_bg.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let tile = tile_bg.clone();
                        let counter = Arc::clone(&count_bg);
                        std::thread::spawn(move || {
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let mut buf = [0u8; 2048];
                            let _ = stream.read(&mut buf);
                            counter.fetch_add(1, Ordering::Relaxed);
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                tile.len()
                            );
                            let _ = stream.write_all(hdr.as_bytes());
                            let _ = stream.write_all(&tile);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        let events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let events_cb = Arc::clone(&events);
        let out_dir = dir.join("multi_zoom_mbtiles");
        let req = OpenGeodataQueryRequest {
            dataset: "xyz_satellite_tiles".to_owned(),
            code: Some(format!(
                "http://127.0.0.1:{port}/tiles/{{z}}/{{x}}/{{y}}.png"
            )),
            keyword: Some("custom:17:nomosaic:mbtiles:DEFLATE:nopyramids:wgs84".to_owned()),
            bbox: Some([117.2700, 31.8500, 117.2850, 31.8650]),
            output_path: Some(out_dir.to_string_lossy().to_string()),
            zooms: Some(vec![17, 18]),
            ..Default::default()
        };

        let res = execute_open_geodata_query_with_progress(&dir, &req, &|ev| {
            if let Ok(mut guard) = events_cb.lock() {
                guard.push(ev);
            }
        })
        .expect("multi-zoom concurrent xyz download");

        stop.store(true, Ordering::Relaxed);
        let _ = srv.join();

        assert_eq!(res["ok"], true);
        assert_eq!(res["tiles_clamped"], false);
        assert_eq!(res["zoom"], 18);
        assert_eq!(res["zooms"], json!([17, 18]));
        assert_eq!(res["max_tiles_limit"], 200_000);
        let tile_count = res["tile_count"].as_u64().unwrap();
        assert!(
            tile_count > 64,
            "expected > 64 tiles after lifting silent 64-tile crop, got {tile_count}"
        );
        assert!(req_count.load(Ordering::Relaxed) >= tile_count as usize);

        let tiles = res["tiles"].as_array().unwrap();
        assert!(tiles.iter().any(|t| t["z"] == 17));
        assert!(tiles.iter().any(|t| t["z"] == 18));

        let mbt_path = PathBuf::from(res["mbtiles_path"].as_str().unwrap());
        assert!(mbt_path.is_file());
        let conn = rusqlite::Connection::open(&mbt_path).unwrap();
        let zoom_levels: Vec<i64> = conn
            .prepare("SELECT DISTINCT zoom_level FROM tiles ORDER BY zoom_level")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(zoom_levels, vec![17, 18]);

        let recorded_events = events.lock().unwrap();
        assert!(recorded_events.iter().any(|e| e["phase"] == "planning"));
        assert!(recorded_events.iter().any(|e| e["phase"] == "downloading"));
        assert!(recorded_events.iter().any(|e| e["phase"] == "done"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_nisar_oe_cmr_entries_filters_level_and_picks_xml_link() {
        let entries = vec![
            json!({
                "id": "G1-ASF",
                "title": "NISAR_ANC_J_PR_FOE_20261115T230512_20261115T224728_20261122T224727",
                "time_start": "2026-11-15T22:47:28.000Z",
                "time_end": "2026-11-22T22:47:27.000Z",
                "links": [
                    {
                        "rel": "http://esipfed.org/ns/fedsearch/1.1/s3#",
                        "href": "s3://bucket/FOE/file.xml"
                    },
                    {
                        "rel": "http://esipfed.org/ns/fedsearch/1.1/data#",
                        "href": "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/FOE/file.xml"
                    }
                ]
            }),
            json!({
                "id": "G2-ASF",
                "title": "NISAR_ANC_J_PR_POE_20261101T000000_20261031T000000_20261102T000000",
                "time_start": "2026-10-31T00:00:00.000Z",
                "time_end": "2026-11-02T00:00:00.000Z",
                "links": [
                    {
                        "rel": "http://esipfed.org/ns/fedsearch/1.1/data#",
                        "href": "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/POE/file_poe.xml"
                    }
                ]
            }),
        ];

        let all_items = parse_nisar_oe_cmr_entries(&entries, "ALL");
        assert_eq!(all_items.len(), 2);
        assert_eq!(all_items[0]["orbit_level"], "FOE");
        assert_eq!(
            all_items[0]["href"],
            "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/FOE/file.xml"
        );
        assert_eq!(all_items[1]["product_type"], "NISAR_POE");

        let poe_only = parse_nisar_oe_cmr_entries(&entries, "POE");
        assert_eq!(poe_only.len(), 1);
        assert_eq!(poe_only[0]["orbit_level"], "POE");
    }

    #[test]
    fn parse_copernicus_pod_odata_entries_maps_s2_and_s3_orbits() {
        let entries = vec![
            json!({
                "Id": "eb7eecd1-b5dd-4ce4-a7ca-f05ddc3ec74b",
                "Name": "S2C_OPER_AUX_POEORB_OPOD_20261115T073918_V20261025T225942_20261027T005942.EOF",
                "ContentLength": 4_653_802_u64,
                "ContentDate": {
                    "Start": "2026-10-25T22:59:42.000000Z",
                    "End": "2026-10-27T00:59:42.000000Z"
                }
            }),
            json!({
                "Id": "0bf23bd5-387a-46ea-a18a-14378bd99d1b",
                "Name": "S3B_SR___POE_AX_20261025T215523_20261027T002323_20261110T152514___________________CNE_O_NT____.SEN3",
                "ContentLength": 103_661_u64,
                "ContentDate": {
                    "Start": "2026-10-25T21:55:23.000000Z",
                    "End": "2026-10-27T00:23:23.000000Z"
                }
            }),
        ];

        let parsed = parse_copernicus_pod_odata_entries(&entries);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["platform"], "Sentinel-2C");
        assert_eq!(parsed[0]["product_type"], "AUX_POEORB");
        assert_eq!(
            parsed[0]["href"],
            "https://download.dataspace.copernicus.eu/odata/v1/Products(eb7eecd1-b5dd-4ce4-a7ca-f05ddc3ec74b)/$value"
        );
        assert_eq!(parsed[1]["platform"], "Sentinel-3B");
        assert_eq!(parsed[1]["product_type"], "SR___POE_AX");
        assert_eq!(parsed[1]["size_bytes"], 103_661_u64);
    }

    #[test]
    fn test_scan_local_slc_dates_extracts_s1_nisar_and_dominant_utc() {
        let tmp = std::env::temp_dir().join(format!(
            "eostage_scan_slc_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sub = tmp.join("track142");
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            tmp.join("S1A_IW_SLC__1SDV_20250915T102345_20250915T102412_060999_0799A1_1111.zip"),
            b"zip",
        )
        .unwrap();
        fs::write(
            sub.join("S1A_IW_SLC__1SDV_20250927T102346_20250927T102413_061174_07A0B2_2222.zip"),
            b"zip",
        )
        .unwrap();
        fs::write(
            sub.join("NISAR_L1_PR_RSLC_001_002_A_001_20251009T041122_20251009T041135.h5"),
            b"h5",
        )
        .unwrap();
        fs::write(tmp.join("readme_20240101.txt"), b"ignore").unwrap();

        let res = scan_local_slc_dates_in_directory(&tmp).expect("scan local slc dates");
        assert_eq!(res["ok"], json!(true));
        assert_eq!(res["dates"], json!(["20250915", "20250927", "20251009"]));
        assert_eq!(res["file_count"], json!(3));
        assert_eq!(res["dominant_utc_hour"], json!("10"));
        assert_eq!(res["dominant_utc_minute"], json!("23"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_bhoonidhi_orbit_direction_extracts_descending_and_ascending_from_node_and_id() {
        let feat_desc = json!({
            "Node": "Descending",
            "Track": 77,
            "Frame": 86
        });
        assert_eq!(
            parse_bhoonidhi_orbit_direction(
                &feat_desc,
                "NISAR_S1_PR_RSLC_029_077_D_086_2500_CRNA_A_20260830T132132_20260830T132141_P00500_N_P_I_001"
            ),
            Some("DESCENDING")
        );
        // When Node is omitted, infer from NISAR granule ID `_D_` / `_A_` token — never default to ASCENDING.
        let props_empty = json!({});
        assert_eq!(
            parse_bhoonidhi_orbit_direction(
                &props_empty,
                "NISAR_S1_PR_GSLC_029_077_D_086_2500_CRNA_A_20260830T132132_20260830T132141_P00500_N_P_I_001"
            ),
            Some("DESCENDING")
        );
        assert_eq!(
            parse_bhoonidhi_orbit_direction(
                &props_empty,
                "NISAR_S1_PR_GSLC_029_098_A_018_2500_CRNA_A_20261006T233951_20261006T234024_P00500_N_P_I_001"
            ),
            Some("ASCENDING")
        );
        assert_eq!(
            parse_orbit_filter_token(Some("2026-08-01..2026-11-16|GSLC|DESCENDING|search")),
            Some("DESCENDING")
        );
        assert_eq!(
            parse_orbit_filter_token(Some("2026-08-01..2026-11-16|GSLC|ASCENDING|search")),
            Some("ASCENDING")
        );
        assert_eq!(
            parse_orbit_filter_token(Some("2026-08-01..2026-11-16|GSLC|search")),
            None
        );
    }

    #[test]
    fn test_split_bhoonidhi_date_windows_splits_newest_first_within_limit() {
        let windows = split_bhoonidhi_date_windows("2025-10-01", "2026-11-16");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].1, "2026-11-16");
        assert_eq!(windows[1].0, "2025-10-01");
        let w0_start = chrono::NaiveDate::parse_from_str(&windows[0].0, "%Y-%m-%d").unwrap();
        let w0_end = chrono::NaiveDate::parse_from_str(&windows[0].1, "%Y-%m-%d").unwrap();
        let w1_start = chrono::NaiveDate::parse_from_str(&windows[1].0, "%Y-%m-%d").unwrap();
        let w1_end = chrono::NaiveDate::parse_from_str(&windows[1].1, "%Y-%m-%d").unwrap();
        assert!((w0_end - w0_start).num_days() <= 360);
        assert!((w1_end - w1_start).num_days() <= 360);
        assert_eq!(w1_end + chrono::Duration::days(1), w0_start);

        let single = split_bhoonidhi_date_windows("2026-08-01", "2026-11-16");
        assert_eq!(
            single,
            vec![("2026-08-01".to_owned(), "2026-11-16".to_owned())]
        );
    }

    #[test]
    fn test_gee_catalog_availability_search_returns_zero_items_when_empty() {
        let empty_avail = json!({
            "ok": true,
            "asset_type": "image_collection",
            "total_scenes": 0,
            "cloud_filtered_scenes": 0,
            "bands": ["dtm", "dsm"],
            "scenes": [],
            "empty_reason_zh": "当前研究区不在 Google GEM-15 官方覆盖范围内（0 景）",
        });
        let empty_res = build_gee_catalog_search_response(
            "projects/climate-and-sustainability/assets/gem-15_v1",
            [117.2, 31.8, 117.3, 31.9],
            "gem15_hefei",
            "GEE_gem15_hefei.tif",
            None,
            None,
            "2026-11-16T00:00:00Z",
            &empty_avail,
        );
        assert_eq!(empty_res["count"], 0);
        assert_eq!(empty_res["items"].as_array().unwrap().len(), 0);
        assert!(empty_res["empty_reason_zh"]
            .as_str()
            .unwrap()
            .contains("0 景"));

        let non_empty_avail = json!({
            "ok": true,
            "asset_type": "image_collection",
            "total_scenes": 5,
            "cloud_filtered_scenes": 2,
            "bands": ["B4", "B3", "B2"],
            "scenes": [
                {"id": "S2A_1", "image_id": "COPERNICUS/S2_SR_HARMONIZED/S2A_1", "acquisition_date": "2026-10-01T02:30:00Z", "cloud_cover": 4.2},
                {"id": "S2B_2", "image_id": "COPERNICUS/S2_SR_HARMONIZED/S2B_2", "acquisition_date": "2026-10-11T02:30:00Z", "cloud_cover": 8.9}
            ],
        });
        let pop_res = build_gee_catalog_search_response(
            "COPERNICUS/S2_SR_HARMONIZED",
            [117.2, 31.8, 117.3, 31.9],
            "s2_hefei",
            "GEE_s2_hefei.tif",
            None,
            None,
            "2026-11-16T00:00:00Z",
            &non_empty_avail,
        );
        assert_eq!(pop_res["count"], 2);
        assert_eq!(pop_res["items"].as_array().unwrap().len(), 2);
        assert_eq!(
            pop_res["items"][0]["id"],
            "COPERNICUS/S2_SR_HARMONIZED/S2A_1"
        );
    }

    #[test]
    fn open_earth_env_datasets_resolve_real_tiles() {
        let dir = std::env::temp_dir().join(format!(
            "eostage_open_ds_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mk_req = |dataset: &str, keyword: &str, bbox: [f64; 4]| OpenGeodataQueryRequest {
            dataset: dataset.to_owned(),
            code: None,
            keyword: Some(keyword.to_owned()),
            bbox: Some(bbox),
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };

        let clcd = execute_open_geodata_query(
            &dir,
            &mk_req(
                "whu_clcd",
                "2020-01-01..2023-12-31|search",
                [117.1, 31.7, 117.4, 32.0],
            ),
        )
        .expect("whu_clcd Hefei search");
        assert_eq!(clcd["count"], 4);
        assert!(clcd["items"][0]["download_url"]
            .as_str()
            .unwrap()
            .contains("zenodo.org/records/18180184"));

        let eot = execute_open_geodata_query(
            &dir,
            &mk_req("tum_eot20", "search", [120.0, 29.0, 122.0, 31.0]),
        )
        .expect("tum_eot20 search");
        assert_eq!(eot["count"], 2);
        assert!(eot["items"][0]["download_url"]
            .as_str()
            .unwrap()
            .contains("seanoe.org"));

        let etopo = execute_open_geodata_query(
            &dir,
            &mk_req("noaa_etopo_2022", "search", [117.1, 31.7, 117.4, 32.0]),
        )
        .expect("noaa_etopo_2022 search");
        assert_eq!(etopo["count"], 1);
        assert!(etopo["items"][0]["id"]
            .as_str()
            .unwrap()
            .contains("N45E105_surface"));
        assert!(etopo["items"][0]["download_url"]
            .as_str()
            .unwrap()
            .contains("15s_surface_elev_gtif/"));

        let gebco = execute_open_geodata_query(
            &dir,
            &mk_req("gebco_2026", "search", [117.1, 31.7, 117.4, 32.0]),
        )
        .expect("gebco_2026 search");
        assert_eq!(gebco["count"], 1);
        assert!(gebco["items"][0]["download_url"]
            .as_str()
            .unwrap()
            .contains("dap.ceda.ac.uk/bodc/gebco/global/gebco_2026/"));

        // Verify GEM-15 public GCS tile filter excludes 404 regions cleanly and maps HTTP 200 regions
        let gem_haikou = resolve_open_dem_tiles("gem15_dtm", [110.2, 19.9, 110.4, 20.1]).unwrap();
        let filtered_haikou = filter_gem15_live_tiles_with_probe(
            gem_haikou["tiles"].as_array().unwrap(),
            "DTM",
            &mut |_| Ok(false),
        )
        .unwrap();
        assert_eq!(filtered_haikou.len(), 0);

        let gem_la = resolve_open_dem_tiles("gem15_dtm", [-118.4, 34.0, -118.1, 34.2]).unwrap();
        let filtered_la = filter_gem15_live_tiles_with_probe(
            gem_la["tiles"].as_array().unwrap(),
            "DTM",
            &mut |_| Ok(true),
        )
        .unwrap();
        assert!(!filtered_la.is_empty());
        assert!(filtered_la[0]["download_url"]
            .as_str()
            .unwrap()
            .contains("storage.googleapis.com/climate-and-sustainability/gem-15/"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_eo_target_selector_rejects_missing_requested_granule_instead_of_substituting() {
        let items = vec![
            json!({
                "id": "VNP46A3.A2025001.h29v05.002",
                "granule_id": "LAADS:1001",
                "href": "https://example.invalid/a.h5"
            }),
            json!({
                "id": "VNP46A3.A2025032.h29v05.002",
                "granule_id": "LAADS:1002",
                "href": "https://example.invalid/b.h5"
            }),
        ];
        let matched = select_open_eo_target_item(&items, Some("VNP46A3.A2025032.h29v05.002"))
            .expect("explicit id present should resolve");
        assert_eq!(matched["id"], "VNP46A3.A2025032.h29v05.002");

        let err = select_open_eo_target_item(&items, Some("VNP46A3.A2024336.MISSING"))
            .expect_err("missing explicit id must fail closed instead of substituting items[0]");
        assert!(
            err.contains("TARGET_GRANULE_NOT_FOUND"),
            "expected explicit TARGET_GRANULE_NOT_FOUND error, got: {err}"
        );
    }

    #[test]
    fn unit_xyz_tiles_aborts_early_on_http_429() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 429 loopback");
        let addr = listener.local_addr().unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let stop_srv = std::sync::Arc::clone(&stop);
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let hits_srv = std::sync::Arc::clone(&hits);

        let srv = std::thread::spawn(move || {
            for stream_res in listener.incoming() {
                if stop_srv.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(mut stream) = stream_res {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf);
                    hits_srv.fetch_add(1, Ordering::AcqRel);
                    let _ = stream.write_all(
                        b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 12\r\nConnection: close\r\n\r\nrate limited",
                    );
                }
            }
        });

        let dir = std::env::temp_dir().join(format!("eostage-xyz-429-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        insar_download::save_transfer_engine_prefs_in_dir(
            &dir,
            insar_download::TransferEnginePrefs {
                tile_chunk_concurrency: 8,
                ..insar_download::TransferEnginePrefs::default()
            },
        )
        .unwrap();

        let out_dir = dir.join("tiles_429_out");
        let res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                code: Some(format!("http://{addr}/tiles/{{z}}/{{x}}/{{y}}.png")),
                keyword: Some("custom:19:mosaic:gtiff:DEFLATE:nopyramids:wgs84".to_owned()),
                bbox: Some([117.27, 31.85, 117.274, 31.854]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(out_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        );

        stop.store(true, Ordering::Release);
        let _ = std::net::TcpStream::connect(addr);
        let _ = srv.join();
        let _ = fs::remove_dir_all(&dir);

        let err = res.expect_err("HTTP 429 flood must return an explicit abort error");
        assert!(
            err.contains("XYZ_UPSTREAM_RATE_LIMITED"),
            "expected XYZ_UPSTREAM_RATE_LIMITED error, got: {err}"
        );
        let total_hits = hits.load(Ordering::Acquire);
        assert!(
            total_hits <= 12,
            "HTTP 429 must abort early across 8 workers without exhausting 36+ z19 tiles & parent fallbacks, hits={total_hits}"
        );
    }

    #[test]
    fn unit_xyz_source_max_zoom_tianditu_routing_and_redaction() {
        assert_eq!(xyz_source_max_zoom("carto_light"), 16);
        assert_eq!(xyz_source_max_zoom("carto_dark"), 16);
        assert_eq!(xyz_source_max_zoom("opentopomap"), 17);
        assert_eq!(xyz_source_max_zoom("tianditu_satellite"), 18);
        assert_eq!(xyz_source_max_zoom("tianditu_vector"), 18);
        assert_eq!(xyz_source_max_zoom("gaode_satellite"), 18);
        assert_eq!(xyz_source_max_zoom("gaode_map"), 18);
        assert_eq!(xyz_source_max_zoom("osm"), 19);
        assert_eq!(xyz_source_max_zoom("arcgis_topo"), 19);
        assert_eq!(xyz_source_max_zoom("arcgis_street"), 19);
        assert_eq!(xyz_source_max_zoom("arcgis_satellite"), 20);
        assert_eq!(xyz_source_max_zoom("google_satellite"), 21);
        assert_eq!(xyz_source_max_zoom("google_hybrid"), 21);
        assert_eq!(xyz_source_max_zoom("google_map"), 21);
        assert_eq!(xyz_source_max_zoom("custom"), 22);
        assert_eq!(xyz_source_max_zoom("custom_tms"), 22);

        let sat_url = build_xyz_tile_url("tianditu_satellite", "", 12, 3382, 1665, "user+tk=1");
        assert!(sat_url.contains("tianditu.gov.cn/img_w/wmts"));
        assert!(sat_url.contains("LAYER=img"));
        assert!(sat_url.contains("TILEMATRIX=12"));
        assert!(sat_url.contains("TILEROW=1665"));
        assert!(sat_url.contains("TILECOL=3382"));
        assert!(sat_url.contains("tk=user%2Btk%3D1"));
        assert!(!sat_url.contains("arcgisonline.com"));

        let vec_url = build_xyz_tile_url("tianditu_vector", "", 12, 3382, 1665, "my_secret");
        assert!(vec_url.contains("tianditu.gov.cn/vec_w/wmts"));
        assert!(vec_url.contains("LAYER=vec"));
        assert!(!vec_url.contains("openstreetmap.org"));

        let redacted = redact_tile_url_secrets(&sat_url);
        assert!(redacted.contains("tk=[REDACTED]"));
        assert!(!redacted.contains("user%2Btk%3D1"));

        let templated_redacted = redact_tile_url_secrets(
            "https://example.com/tiles/{z}/{x}/{y}.png?tk=secret123&layer=img",
        );
        assert_eq!(
            templated_redacted,
            "https://example.com/tiles/{z}/{x}/{y}.png?tk=[REDACTED]&layer=img"
        );

        let dir = std::env::temp_dir().join(format!("eostage-tdt-tk-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(resolve_tianditu_token(&dir), "");
        fs::write(
            dir.join("network-settings.json"),
            r#"{"tianditu_token":"   "}"#,
        )
        .unwrap();
        assert_eq!(resolve_tianditu_token(&dir), "");
        fs::write(
            dir.join("network-settings.json"),
            r#"{"tianditu_token":"custom_tdt_999"}"#,
        )
        .unwrap();
        assert_eq!(resolve_tianditu_token(&dir), "custom_tdt_999");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unit_xyz_tiles_limit_exceeded_and_jpeg_dimension_guard() {
        let dir =
            std::env::temp_dir().join(format!("eostage-xyz-limits-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 1. Tianditu Z19 exceeds Z18 maxZoom without creating output_dir
        let tdt_out = dir.join("tdt_z19_should_not_exist");
        let err_tdt = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                keyword: Some(
                    "tianditu_satellite:19:mosaic:gtiff:DEFLATE:nopyramids:wgs84".to_owned(),
                ),
                bbox: Some([117.27, 31.85, 117.28, 31.86]),
                output_path: Some(tdt_out.to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .expect_err("tianditu_satellite:19 must fail explicitly");
        assert!(err_tdt.contains("XYZ_ZOOM_OUT_OF_RANGE"), "got: {err_tdt}");
        assert!(
            !tdt_out.exists(),
            "output_dir must not be created on invalid zoom"
        );

        // 2. OSM Z20 via req.zooms exceeds Z19 maxZoom
        let osm_out = dir.join("osm_z20_should_not_exist");
        let err_osm = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                keyword: Some("osm:18:nomosaic:mbtiles:DEFLATE:nopyramids:wgs84".to_owned()),
                bbox: Some([117.27, 31.85, 117.28, 31.86]),
                zooms: Some(vec![18, 20]),
                output_path: Some(osm_out.to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .expect_err("osm Z20 must fail explicitly");
        assert!(err_osm.contains("XYZ_ZOOM_OUT_OF_RANGE"), "got: {err_osm}");
        assert!(
            !osm_out.exists(),
            "output_dir must not be created on out-of-range req.zooms"
        );

        // 3. Stitched GeoTIFF > 50,000 tiles fails explicitly with XYZ_TILES_LIMIT_EXCEEDED
        let over50k_out = dir.join("over50k_gtiff_should_not_exist");
        let err_50k = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                keyword: Some(
                    "arcgis_satellite:18:mosaic:gtiff:DEFLATE:nopyramids:wgs84".to_owned(),
                ),
                bbox: Some([116.0, 31.0, 117.0, 32.0]),
                output_path: Some(over50k_out.to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .expect_err(">50k stitched tiles must fail explicitly");
        assert!(
            err_50k.contains("XYZ_TILES_LIMIT_EXCEEDED"),
            "got: {err_50k}"
        );
        assert!(err_50k.contains("50000"), "got: {err_50k}");
        assert!(!over50k_out.exists());

        // 4. Archive MBTiles > 200,000 tiles fails explicitly with 200000 cap (not 50000)
        let over200k_out = dir.join("over200k_mbtiles_should_not_exist");
        let err_200k = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                keyword: Some(
                    "arcgis_satellite:18:nomosaic:mbtiles:DEFLATE:nopyramids:wgs84".to_owned(),
                ),
                bbox: Some([115.0, 30.0, 118.0, 33.0]),
                output_path: Some(over200k_out.to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .expect_err(">200k mbtiles tiles must fail explicitly");
        assert!(
            err_200k.contains("XYZ_TILES_LIMIT_EXCEEDED"),
            "got: {err_200k}"
        );
        assert!(err_200k.contains("200000"), "got: {err_200k}");
        assert!(!over200k_out.exists());

        // 5. Wide thin strip under 50k tiles but > 65,535 px width in stitched JPEG fails explicitly
        let jpg_out = dir.join("jpg_dim_should_not_exist");
        let err_jpg = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                keyword: Some("arcgis_satellite:18:mosaic:jpg:JPEG:nopyramids:wgs84".to_owned()),
                bbox: Some([117.0, 31.85, 117.45, 31.8505]),
                output_path: Some(jpg_out.to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .expect_err(">65535 px JPEG stitch must fail explicitly");
        assert!(
            err_jpg.contains("XYZ_JPEG_DIMENSION_EXCEEDED"),
            "got: {err_jpg}"
        );
        assert!(!jpg_out.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unit_xyz_tile_magic_bytes_and_tianditu_200_json_rejection() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        assert!(is_valid_tile_image_bytes(&[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00
        ]));
        assert!(is_valid_tile_image_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(is_valid_tile_image_bytes(b"RIFF\x00\x00\x00\x00WEBP"));
        assert!(!is_valid_tile_image_bytes(
            r#"{"msg":"非法key","code":403}"#.as_bytes()
        ));
        assert!(!is_valid_tile_image_bytes(
            b"<html><body>Forbidden</body></html>"
        ));
        assert!(!is_valid_tile_image_bytes(&[]));

        assert!(is_tianditu_host(
            "https://t0.tianditu.gov.cn/img_w/wmts?tk=abc"
        ));
        assert!(is_tianditu_host("https://tianditu.gov.cn/vec_w/wmts"));
        assert!(!is_tianditu_host(
            "https://evil.com/?redirect=t0.tianditu.gov.cn"
        ));
        assert!(!is_tianditu_host("https://eviltianditu.gov.cn/img_w/wmts"));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let srv = std::thread::spawn(move || {
            for stream_res in listener.incoming() {
                if stop_bg.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(mut stream) = stream_res {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);
                    let body = r#"{"msg":"非法key","resolve":"err"}"#.as_bytes();
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(hdr.as_bytes());
                    let _ = stream.write_all(body);
                }
            }
        });

        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let abort = AtomicBool::new(false);
        let outcome =
            fetch_xyz_tile_bytes(&client, &format!("http://{addr}/img_w/wmts?tk=bad"), &abort);
        stop.store(true, Ordering::Release);
        let _ = std::net::TcpStream::connect(addr);
        let _ = srv.join();

        assert!(
            matches!(outcome, XyzTileFetchOutcome::Forbidden(403)),
            "HTTP 200 auth JSON must classify as Forbidden(403), got {outcome:?}"
        );
    }

    #[test]
    #[ignore = "live network acceptance test"]
    fn test_live_tianditu_satellite_and_vector_wmts_download_and_redaction() {
        let test_token = std::env::var("EOSTAGE_TEST_TIANDITU_TOKEN")
            .expect("set EOSTAGE_TEST_TIANDITU_TOKEN to run live Tianditu WMTS acceptance test");
        let dir = std::env::temp_dir().join(format!(
            "eostage-live-tianditu-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("network-settings.json"),
            serde_json::json!({
                "proxy_enabled": false,
                "proxy_url": "",
                "tianditu_token": test_token
            })
            .to_string(),
        )
        .unwrap();

        let sat_dir = dir.join("tdt_sat_z12");
        let sat_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                keyword: Some(
                    "tianditu_satellite:12:nomosaic:gtiff:DEFLATE:nopyramids:wgs84".to_owned(),
                ),
                bbox: Some([117.275, 31.855, 117.280, 31.860]),
                output_path: Some(sat_dir.to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .expect("live tianditu_satellite Z12 download");
        assert_eq!(sat_res["ok"], true);
        assert!(sat_res["tile_count"].as_u64().unwrap() >= 1);
        let sat_tile = PathBuf::from(sat_res["tiles"][0]["file_path"].as_str().unwrap());
        let sat_bytes = fs::read(&sat_tile).unwrap();
        assert!(
            is_valid_tile_image_bytes(&sat_bytes) && sat_bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
            "expected JPEG magic bytes for tianditu_satellite, got first 8 bytes {:?}",
            &sat_bytes[..sat_bytes.len().min(8)]
        );
        let sat_manifest_txt = fs::read_to_string(sat_dir.join("tiles_manifest.json")).unwrap();
        assert!(!sat_manifest_txt.contains(&test_token));
        assert!(sat_manifest_txt.contains("tianditu.gov.cn/img_w/wmts"));
        assert!(sat_manifest_txt.contains("tk=[REDACTED]"));
        assert!(!sat_manifest_txt.contains("arcgisonline.com"));

        let vec_dir = dir.join("tdt_vec_z12");
        let vec_res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                keyword: Some(
                    "tianditu_vector:12:nomosaic:gtiff:DEFLATE:nopyramids:wgs84".to_owned(),
                ),
                bbox: Some([117.275, 31.855, 117.280, 31.860]),
                output_path: Some(vec_dir.to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .expect("live tianditu_vector Z12 download");
        assert_eq!(vec_res["ok"], true);
        assert!(vec_res["tile_count"].as_u64().unwrap() >= 1);
        let vec_tile = PathBuf::from(vec_res["tiles"][0]["file_path"].as_str().unwrap());
        let vec_bytes = fs::read(&vec_tile).unwrap();
        assert!(
            is_valid_tile_image_bytes(&vec_bytes)
                && vec_bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
            "expected PNG magic bytes for tianditu_vector, got first 8 bytes {:?}",
            &vec_bytes[..vec_bytes.len().min(8)]
        );
        let vec_manifest_txt = fs::read_to_string(vec_dir.join("tiles_manifest.json")).unwrap();
        assert!(!vec_manifest_txt.contains(&test_token));
        assert!(vec_manifest_txt.contains("tianditu.gov.cn/vec_w/wmts"));
        assert!(vec_manifest_txt.contains("tk=[REDACTED]"));
        assert!(!vec_manifest_txt.contains("openstreetmap.org"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_geodata_dispatch_routes_new_gee_advisor_and_preflight_commands() {
        let dir = std::env::temp_dir().join(format!(
            "eostage-geodata-gee-dispatch-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let make_req = |dataset: &str, params: Value| OpenGeodataQueryRequest {
            dataset: dataset.to_owned(),
            code: None,
            keyword: None,
            bbox: Some([117.24, 31.81, 117.29, 31.86]),
            lat: None,
            lng: None,
            start_date: Some("2025-05-01".to_owned()),
            end_date: Some("2025-09-01".to_owned()),
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: Some(params),
            aoi_geojson: None,
            zooms: None,
        };

        let rec = execute_open_geodata_query(
            &dir,
            &make_req(
                "gee_recommend_datasets",
                json!({"task": "洪涝风险评估", "limit_per_role": 2}),
            ),
        )
        .expect("gee_recommend_datasets dispatches cleanly");
        assert_eq!(rec["ok"], true);
        assert_eq!(rec["recipe"], "flood-risk");

        let cmp = execute_open_geodata_query(
            &dir,
            &make_req(
                "gee_compare_datasets",
                json!({"ids": ["COPERNICUS/DEM/GLO30", "USGS/SRTMGL1_003"]}),
            ),
        )
        .expect("gee_compare_datasets dispatches cleanly");
        assert_eq!(cmp["ok"], true);

        let ver = execute_open_geodata_query(
            &dir,
            &make_req(
                "gee_verify_dataset",
                json!({"asset_id": "COPERNICUS/S2_SR_HARMONIZED", "live": false}),
            ),
        )
        .expect("gee_verify_dataset dispatches cleanly");
        assert_eq!(ver["ok"], true);
        assert_eq!(ver["catalog_found"], true);

        let adv = execute_open_geodata_query(
            &dir,
            &make_req(
                "gee_dataset_advisor",
                json!({"mode": "recommend", "task": "vegetation monitoring"}),
            ),
        )
        .expect("gee_dataset_advisor dispatches cleanly");
        assert_eq!(adv["ok"], true);
        assert_eq!(adv["recipe"], "vegetation-monitoring");

        let pre = execute_open_geodata_query(
            &dir,
            &make_req(
                "gee_export_preflight",
                json!({"asset_id": "COPERNICUS/S2_SR_HARMONIZED", "scale_m": 10.0}),
            ),
        )
        .expect("gee_export_preflight dispatches cleanly");
        assert_eq!(pre["ok"], true);
        assert_eq!(pre["ready"], true);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn aws_should_download_and_noaa_filters_and_overture_paths_guard_search_mode() {
        let search_req_with_dir = OpenGeodataQueryRequest {
            dataset: "aws_sentinel1_grd".to_owned(),
            code: None,
            keyword: Some("2026-10-22..2026-11-21|vv|search".to_owned()),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: Some("/tmp/user-configured-dir".to_owned()),
            full_children: None,
            tool_name: Some("search".to_owned()),
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        assert!(
            !aws_should_download(&search_req_with_dir),
            "configured output_path with |search MUST NOT trigger automatic download during UI Search"
        );

        let mut dl_req = search_req_with_dir.clone();
        dl_req.keyword = Some("2026-10-22..2026-11-21|vv|download".to_owned());
        dl_req.tool_name = Some("download".to_owned());
        assert!(aws_should_download(&dl_req));

        assert!(
            noaa_gfs_custom_filters(Some("2026-10-22..2026-11-21|gfs|search")).is_empty(),
            "Desktop UI date-range|gfs|search keyword must cleanly fall back to default GFS variables"
        );
        assert_eq!(
            noaa_gfs_custom_filters(Some("2026-10-22..2026-11-21|TMP:2 m above ground|search")),
            vec!["TMP:2 m above ground".to_owned()]
        );

        let (parquet_p, geojson_p) = overture_export_paths(Path::new("/tmp/out/hefei.geojson"));
        assert_ne!(parquet_p, geojson_p);
        assert_eq!(geojson_p, PathBuf::from("/tmp/out/hefei.geojson"));
        assert!(parquet_p.to_string_lossy().ends_with(".zstd.parquet"));
    }

    #[test]
    fn allowlisted_geodata_preview_url_validates_strictly_and_cache_bounds_memory() {
        assert!(validate_allowlisted_geodata_preview_url(
            "https://sentinel-s1-l1c.s3.eu-central-1.amazonaws.com/GRD/2026/11/24/preview/quick-look.png"
        )
        .is_ok());
        assert!(validate_allowlisted_geodata_preview_url(
            "https://sentinel-s1-l1c.s3.amazonaws.com/GRD/quick-look.png"
        )
        .is_ok());

        for bad in [
            "",
            "http://sentinel-s1-l1c.s3.eu-central-1.amazonaws.com/quick-look.png",
            "https://user:pass@sentinel-s1-l1c.s3.eu-central-1.amazonaws.com/quick-look.png",
            "https://sentinel-s1-l1c.s3.eu-central-1.amazonaws.com:8443/quick-look.png",
            "https://sentinel-cogs.s3.us-west-2.amazonaws.com/other.png",
            "https://127.0.0.1/secret.png",
            "https://evil.example.invalid/quick-look.png",
        ] {
            assert!(
                validate_allowlisted_geodata_preview_url(bad).is_err(),
                "must reject {bad}"
            );
        }

        let mut cache = GeodataPreviewCache::default();
        for i in 0..70 {
            cache.insert(format!("k{i}"), format!("data:image/png;base64,{i}"));
        }
        assert!(cache.entries.len() <= GEODATA_PREVIEW_CACHE_MAX_ENTRIES);
        assert!(cache.get("k0").is_none(), "oldest entries must be evicted");
        assert!(cache.get("k69").is_some(), "newest entry must remain");

        let large_payload = format!(
            "data:image/png;base64,{}",
            "A".repeat(GEODATA_PREVIEW_CACHE_MAX_BYTES + 8)
        );
        cache.insert("oversized".to_owned(), large_payload);
        assert!(cache.get("oversized").is_none());
    }
}
