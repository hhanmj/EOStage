//! Public catalog builders, response parsers, multi-asset link selectors, and
//! science binary validators for:
//! - NASA VIIRS Black Marble Nighttime Lights (`VNP46A2`, `VNP46A3`, `VNP46A4`)
//! - NASA JPL EMIT Hyperspectral (`EMITL2ARFL`, `EMITL2BMIN`)
//! - NASA JPL ECOSTRESS Thermal (`ECO_L2T_LSTE` LST & Wideband Emissivity, `ECO_L3T_JET` ET)
//! - German DLR EnMAP Hyperspectral (`ENMAP_HSI_L2A`, public STAC `search_only`)

use reqwest::Url;
use serde_json::{json, Value};

pub const NASA_CMR_GRANULES_JSON_URL: &str = "https://cmr.earthdata.nasa.gov/search/granules.json";
pub const DLR_EOC_STAC_SEARCH_URL: &str = "https://geoservice.dlr.de/eoc/ogc/stac/v1/search";
pub const DLR_EOWEB_PORTAL_URL: &str = "https://eoweb.dlr.de";

pub const DLR_ENMAP_SSO_DOWNLOAD_ERROR: &str =
    "DLR_EOWEB_SSO_REQUIRED: 德国宇航中心 DLR EnMAP L2A 原始高光谱条带受 DLR EOC CAS 单点登录门户（sso.eoc.dlr.de）保护，当前工作台仅支持免登录公开 STAC 检索与空间足迹预览；如需下载原始光谱立方体，请前往 https://eoweb.dlr.de 登录获取。";

pub const EARTHDATA_BLACK_MARBLE_AUTH_ERROR: &str =
    "EARTHDATA_AUTH_REQUIRED: 下载 NASA Black Marble 夜光原始 HDF5 科学产品需先在「设置 -> 数据服务账号」登录 NASA Earthdata 账号";

pub const EARTHDATA_EMIT_AUTH_ERROR: &str =
    "EARTHDATA_AUTH_REQUIRED: 下载 NASA JPL EMIT 高光谱原始 NetCDF4 科学产品需先在「设置 -> 数据服务账号」登录 NASA Earthdata 账号";

pub const EARTHDATA_ECOSTRESS_AUTH_ERROR: &str =
    "EARTHDATA_AUTH_REQUIRED: 下载 NASA JPL ECOSTRESS 热红外原始 GeoTIFF 科学产品需先在「设置 -> 数据服务账号」登录 NASA Earthdata 账号";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OpenEoDataset {
    NasaBlackMarble,
    NasaEmit,
    NasaEcostress,
    DlrEnmap,
}

impl OpenEoDataset {
    pub fn from_dataset_key(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "nasa_black_marble" | "black_marble" | "viirs_black_marble" => {
                Some(Self::NasaBlackMarble)
            }
            "nasa_emit" | "emit" | "emit_l2a" => Some(Self::NasaEmit),
            "nasa_ecostress" | "ecostress" | "ecostress_l2t" => Some(Self::NasaEcostress),
            "dlr_enmap" | "enmap" | "enmap_l2a" => Some(Self::DlrEnmap),
            _ => None,
        }
    }

    pub const fn dataset_key(self) -> &'static str {
        match self {
            Self::NasaBlackMarble => "nasa_black_marble",
            Self::NasaEmit => "nasa_emit",
            Self::NasaEcostress => "nasa_ecostress",
            Self::DlrEnmap => "dlr_enmap",
        }
    }

    pub const fn resource_id(self) -> &'static str {
        match self {
            Self::NasaBlackMarble => "science.nasa.black-marble.v1",
            Self::NasaEmit => "science.nasa.emit.l2a.v1",
            Self::NasaEcostress => "science.nasa.ecostress.l2t.v1",
            Self::DlrEnmap => "science.dlr.enmap.l2a.v1",
        }
    }

    pub const fn provider_label(self) -> &'static str {
        match self {
            Self::NasaBlackMarble => "NASA_LAADS_CMR",
            Self::NasaEmit | Self::NasaEcostress => "NASA_LPDAAC_CMR",
            Self::DlrEnmap => "DLR_EOC_STAC",
        }
    }

    pub const fn platform_name(self) -> &'static str {
        match self {
            Self::NasaBlackMarble => "Suomi NPP / NOAA-20 VIIRS",
            Self::NasaEmit => "ISS EMIT",
            Self::NasaEcostress => "ISS ECOSTRESS",
            Self::DlrEnmap => "EnMAP",
        }
    }

    pub const fn download_supported(self) -> bool {
        !matches!(self, Self::DlrEnmap)
    }

    pub const fn earthdata_auth_error(self) -> &'static str {
        match self {
            Self::NasaBlackMarble => EARTHDATA_BLACK_MARBLE_AUTH_ERROR,
            Self::NasaEmit => EARTHDATA_EMIT_AUTH_ERROR,
            Self::NasaEcostress => EARTHDATA_ECOSTRESS_AUTH_ERROR,
            Self::DlrEnmap => DLR_ENMAP_SSO_DOWNLOAD_ERROR,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpenEoSubproductSpec {
    pub dataset: OpenEoDataset,
    /// UI / keyword selector token (e.g. `"VNP46A3"`, `"ECO_L2T_LSTE_EMIS"`).
    pub code: &'static str,
    /// Upstream CMR `short_name` or STAC collection ID.
    pub short_name: &'static str,
    /// Exact Collection 2 / current-archive CMR `collection_concept_id` (or STAC collection ID).
    pub collection_concept_id: &'static str,
    pub version: &'static str,
    pub science_level: &'static str,
    pub default_ext: &'static str,
    pub resolution_m: f64,
    pub download_supported: bool,
    pub empty_reason_zh: &'static str,
}

pub const OPEN_EO_SUBPRODUCT_CATALOG: &[OpenEoSubproductSpec] = &[
    OpenEoSubproductSpec {
        dataset: OpenEoDataset::NasaBlackMarble,
        code: "VNP46A3",
        short_name: "VNP46A3",
        collection_concept_id: "C3860061042-LAADS",
        version: "002",
        science_level: "L3_MONTHLY",
        default_ext: "h5",
        resolution_m: 500.0,
        download_supported: true,
        empty_reason_zh: "当前研究区与时间窗口内暂无 NASA Black Marble VNP46A3 月度夜光合成产品（建议扩大时间范围至最近 3–12 个月）。",
    },
    OpenEoSubproductSpec {
        dataset: OpenEoDataset::NasaBlackMarble,
        code: "VNP46A4",
        short_name: "VNP46A4",
        collection_concept_id: "C3860065683-LAADS",
        version: "002",
        science_level: "L3_ANNUAL",
        default_ext: "h5",
        resolution_m: 500.0,
        download_supported: true,
        empty_reason_zh: "当前研究区与时间窗口内暂无 NASA Black Marble VNP46A4 年度夜光合成产品（建议包含完整自然年时间跨度）。",
    },
    OpenEoSubproductSpec {
        dataset: OpenEoDataset::NasaBlackMarble,
        code: "VNP46A2",
        short_name: "VNP46A2",
        collection_concept_id: "C3365931269-LAADS",
        version: "002",
        science_level: "L3_DAILY",
        default_ext: "h5",
        resolution_m: 500.0,
        download_supported: true,
        empty_reason_zh: "当前研究区与时间窗口内暂无 NASA Black Marble VNP46A2 逐日月光校正夜光产品。",
    },
    OpenEoSubproductSpec {
        dataset: OpenEoDataset::NasaEmit,
        code: "EMITL2ARFL",
        short_name: "EMITL2ARFL",
        collection_concept_id: "C4079844428-LPCLOUD",
        version: "001",
        science_level: "L2A_RFL",
        default_ext: "nc",
        resolution_m: 60.0,
        download_supported: true,
        empty_reason_zh: "当前研究区与时间窗口内暂无 NASA JPL EMIT L2A 地表反射率产品（EMIT 搭乘国际空间站重点观测干旱/矿物目标区，纬度覆盖约 ±52°）。",
    },
    OpenEoSubproductSpec {
        dataset: OpenEoDataset::NasaEmit,
        code: "EMITL2BMIN",
        short_name: "EMITL2BMIN",
        collection_concept_id: "C4079846859-LPCLOUD",
        version: "001",
        science_level: "L2B_MIN",
        default_ext: "nc",
        resolution_m: 60.0,
        download_supported: true,
        empty_reason_zh: "当前研究区与时间窗口内暂无 NASA JPL EMIT L2B 矿物丰度与吸收特征产品（EMIT 重点覆盖全球干旱及矿物露头目标区，纬度约 ±52°）。",
    },
    OpenEoSubproductSpec {
        dataset: OpenEoDataset::NasaEcostress,
        code: "ECO_L2T_LSTE",
        short_name: "ECO_L2T_LSTE",
        collection_concept_id: "C2076090826-LPCLOUD",
        version: "002",
        science_level: "L2T_LST",
        default_ext: "tif",
        resolution_m: 70.0,
        download_supported: true,
        empty_reason_zh: "当前研究区与时间窗口内暂无 NASA ECOSTRESS L2T 地表温度（LST）切片产品（国际空间站轨道覆盖约 ±52°）。",
    },
    OpenEoSubproductSpec {
        dataset: OpenEoDataset::NasaEcostress,
        code: "ECO_L2T_LSTE_EMIS",
        short_name: "ECO_L2T_LSTE",
        collection_concept_id: "C2076090826-LPCLOUD",
        version: "002",
        science_level: "L2T_EMIS",
        default_ext: "tif",
        resolution_m: 70.0,
        download_supported: true,
        empty_reason_zh: "当前研究区与时间窗口内暂无 NASA ECOSTRESS L2T 宽波段发射率（EmisWB）切片产品（国际空间站轨道覆盖约 ±52°）。",
    },
    OpenEoSubproductSpec {
        dataset: OpenEoDataset::NasaEcostress,
        code: "ECO_L3T_JET",
        short_name: "ECO_L3T_JET",
        collection_concept_id: "C2076106409-LPCLOUD",
        version: "002",
        science_level: "L3T_JET",
        default_ext: "tif",
        resolution_m: 70.0,
        download_supported: true,
        empty_reason_zh: "当前研究区与时间窗口内暂无 NASA ECOSTRESS L3T 蒸散发（JET ETdaily）切片产品（国际空间站轨道覆盖约 ±52°）。",
    },
    OpenEoSubproductSpec {
        dataset: OpenEoDataset::DlrEnmap,
        code: "ENMAP_HSI_L2A",
        short_name: "ENMAP_HSI_L2A",
        collection_concept_id: "ENMAP_HSI_L2A",
        version: "1.0",
        science_level: "L2A",
        default_ext: "tif",
        resolution_m: 30.0,
        download_supported: false,
        empty_reason_zh: "当前研究区与时间窗口内暂无德国 DLR EnMAP L2A 公开归档条带（EnMAP 为任务规划推扫成像，建议适当放宽时间范围）。",
    },
];

pub fn resolve_open_eo_subproduct(
    dataset: OpenEoDataset,
    raw_token: Option<&str>,
) -> &'static OpenEoSubproductSpec {
    let normalized = raw_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_uppercase().replace('-', "_"));

    let target_code = match (dataset, normalized.as_deref()) {
        (OpenEoDataset::NasaBlackMarble, Some("VNP46A4" | "ANNUAL" | "YEARLY")) => "VNP46A4",
        (OpenEoDataset::NasaBlackMarble, Some("VNP46A2" | "DAILY")) => "VNP46A2",
        (OpenEoDataset::NasaBlackMarble, _) => "VNP46A3",

        (OpenEoDataset::NasaEmit, Some("EMITL2BMIN" | "L2B" | "MIN" | "MINERALOGY")) => {
            "EMITL2BMIN"
        }
        (OpenEoDataset::NasaEmit, _) => "EMITL2ARFL",

        (
            OpenEoDataset::NasaEcostress,
            Some("ECO_L2T_LSTE_EMIS" | "EMIS" | "EMISWB" | "LSTE_EMIS"),
        ) => "ECO_L2T_LSTE_EMIS",
        (OpenEoDataset::NasaEcostress, Some("ECO_L3T_JET" | "JET" | "ET" | "ETDAILY" | "L3T")) => {
            "ECO_L3T_JET"
        }
        (OpenEoDataset::NasaEcostress, _) => "ECO_L2T_LSTE",

        (OpenEoDataset::DlrEnmap, _) => "ENMAP_HSI_L2A",
    };

    OPEN_EO_SUBPRODUCT_CATALOG
        .iter()
        .find(|entry| entry.dataset == dataset && entry.code == target_code)
        .expect("OPEN_EO_SUBPRODUCT_CATALOG contains all canonical codes")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenEoKeywordParsed {
    pub date_range: Option<(String, String)>,
    pub sub_product_token: Option<String>,
    pub download_requested: bool,
    pub search_only_token: bool,
}

pub fn parse_open_eo_keyword(raw_keyword: Option<&str>) -> OpenEoKeywordParsed {
    let mut parsed = OpenEoKeywordParsed::default();
    let Some(kw) = raw_keyword.map(str::trim).filter(|s| !s.is_empty()) else {
        return parsed;
    };
    for token in kw.split('|').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((start_raw, end_raw)) = token.split_once("..") {
            let start = start_raw.trim();
            let end = end_raw.trim();
            if !start.is_empty() && !end.is_empty() {
                parsed.date_range = Some((start.to_owned(), end.to_owned()));
            }
        } else if token.eq_ignore_ascii_case("download") {
            parsed.download_requested = true;
        } else if token.eq_ignore_ascii_case("search")
            || token.eq_ignore_ascii_case("search_only")
            || token.eq_ignore_ascii_case("preview")
        {
            if token.eq_ignore_ascii_case("search") || token.eq_ignore_ascii_case("search_only") {
                parsed.search_only_token = true;
            }
        } else if token.eq_ignore_ascii_case("ascending")
            || token.eq_ignore_ascii_case("descending")
            || token.eq_ignore_ascii_case("catalog")
        {
            // Ignore generic workbench direction/catalog tokens.
        } else if parsed.sub_product_token.is_none() {
            parsed.sub_product_token = Some(token.to_owned());
        }
    }
    parsed
}

pub fn normalize_wgs84_bbox(raw: [f64; 4]) -> Result<[f64; 4], String> {
    if !raw.iter().all(|v| v.is_finite()) {
        return Err("INVALID_BBOX: bbox 坐标必须为有限数值 [west, south, east, north]".to_owned());
    }
    let west = raw[0].min(raw[2]).clamp(-180.0, 180.0);
    let east = raw[0].max(raw[2]).clamp(-180.0, 180.0);
    let south = raw[1].min(raw[3]).clamp(-90.0, 90.0);
    let north = raw[1].max(raw[3]).clamp(-90.0, 90.0);
    Ok([west, south, east, north])
}

fn normalize_iso_start(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.contains('T') {
        trimmed.to_owned()
    } else {
        format!("{trimmed}T00:00:00Z")
    }
}

fn normalize_iso_end(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.contains('T') {
        trimmed.to_owned()
    } else {
        format!("{trimmed}T23:59:59Z")
    }
}

/// Builds a fast, exact-collection NASA CMR `granules.json` URL scoped by
/// `collection_concept_id` so multi-DAAC / legacy-version scans never stall.
pub fn build_cmr_granules_search_url(
    spec: &OpenEoSubproductSpec,
    bbox: [f64; 4],
    temporal: Option<(&str, &str)>,
    page_size: usize,
) -> Result<String, String> {
    let normalized_bbox = normalize_wgs84_bbox(bbox)?;
    let clamped_page_size = page_size.clamp(1, 100);
    let bbox_param = format!(
        "{},{},{},{}",
        normalized_bbox[0], normalized_bbox[1], normalized_bbox[2], normalized_bbox[3]
    );
    let mut url = Url::parse(NASA_CMR_GRANULES_JSON_URL)
        .map_err(|err| format!("CMR_URL_BUILD_FAILED: {err}"))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("collection_concept_id", spec.collection_concept_id);
        pairs.append_pair("bounding_box", &bbox_param);
        pairs.append_pair("page_size", &clamped_page_size.to_string());
        pairs.append_pair("sort_key", "-start_date");
        if let Some((start, end)) = temporal {
            let start_clean = start.trim();
            let end_clean = end.trim();
            if !start_clean.is_empty() && !end_clean.is_empty() {
                let temporal_param = format!(
                    "{},{}",
                    normalize_iso_start(start_clean),
                    normalize_iso_end(end_clean)
                );
                pairs.append_pair("temporal", &temporal_param);
            }
        }
    }
    Ok(url.into())
}

/// Builds a STAC search POST body for German DLR EOC `ENMAP_HSI_L2A`.
pub fn build_dlr_enmap_stac_search_payload(
    bbox: [f64; 4],
    temporal: Option<(&str, &str)>,
    limit: usize,
) -> Result<Value, String> {
    let normalized_bbox = normalize_wgs84_bbox(bbox)?;
    let clamped_limit = limit.clamp(1, 100);
    let datetime_interval = match temporal {
        Some((start, end)) if !start.trim().is_empty() && !end.trim().is_empty() => {
            format!("{}/{}", normalize_iso_start(start), normalize_iso_end(end))
        }
        _ => "2022-04-01T00:00:00Z/..".to_owned(),
    };
    Ok(json!({
        "collections": ["ENMAP_HSI_L2A"],
        "bbox": normalized_bbox,
        "datetime": datetime_interval,
        "limit": clamped_limit,
    }))
}

/// Sanitizes an upstream granule identifier or filename into a cross-platform
/// filesystem-safe stem (stripping known extensions and replacing `:` / `/` / `\`).
pub fn sanitize_granule_stem(raw: &str) -> String {
    let trimmed = raw.trim();
    let base = trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed)
        .split('?')
        .next()
        .unwrap_or(trimmed);
    let lower = base.to_ascii_lowercase();
    let stripped = [
        ".tiff", ".tif", ".hdf5", ".hdf", ".h5", ".nc4", ".nc", ".tar.gz", ".tgz", ".tar", ".zip",
    ]
    .iter()
    .find_map(|ext| {
        if lower.ends_with(ext) && base.len() > ext.len() {
            Some(&base[..base.len() - ext.len()])
        } else {
            None
        }
    })
    .unwrap_or(base);

    let mut out = String::with_capacity(stripped.len());
    for ch in stripped.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let cleaned = out.trim_matches(|c| c == '.' || c == '_');
    if cleaned.is_empty() {
        "granule".to_owned()
    } else {
        cleaned.to_owned()
    }
}

fn url_basename_lower(href: &str) -> String {
    let path_part = href.split('?').next().unwrap_or(href);
    path_part
        .rsplit('/')
        .next()
        .unwrap_or(path_part)
        .to_ascii_lowercase()
}

fn is_science_data_link_candidate(href: &str, rel: &str) -> bool {
    if !href.starts_with("https://") {
        return false;
    }
    let rel_lower = rel.to_ascii_lowercase();
    if rel_lower.contains("browse#")
        || rel_lower.contains("metadata#")
        || rel_lower.contains("documentation#")
        || rel_lower.contains("via#")
    {
        return false;
    }
    let base = url_basename_lower(href);
    if base.ends_with(".xml")
        || base.ends_with(".json")
        || base.ends_with(".cmr.cfg")
        || base.ends_with(".dmrpp")
        || base.ends_with(".png")
        || base.ends_with(".jpg")
        || base.ends_with(".jpeg")
        || base.contains("s3credentials")
    {
        return false;
    }
    base.ends_with(".h5")
        || base.ends_with(".hdf5")
        || base.ends_with(".nc")
        || base.ends_with(".tif")
        || base.ends_with(".tiff")
}

/// Selects `(primary_data_https_url, browse_image_https_url)` from a NASA CMR
/// granule `links` array according to the exact sub-product asset contract.
pub fn select_cmr_granule_links(
    spec: &OpenEoSubproductSpec,
    links: &[Value],
) -> (Option<String>, Option<String>) {
    let mut data_candidates: Vec<String> = Vec::new();
    let mut browse_candidates: Vec<String> = Vec::new();

    for link in links {
        let Some(href) = link.get("href").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if !href.starts_with("https://") {
            continue;
        }
        let rel = link.get("rel").and_then(Value::as_str).unwrap_or("");
        let base = url_basename_lower(href);
        if (rel.contains("browse")
            || base.ends_with(".png")
            || base.ends_with(".jpg")
            || base.ends_with(".jpeg"))
            && !base.ends_with(".tif")
            && !base.ends_with(".tiff")
        {
            browse_candidates.push(href.to_owned());
            continue;
        }
        if is_science_data_link_candidate(href, rel) {
            data_candidates.push(href.to_owned());
        }
    }

    // Prefer anonymous `lp-prod-public` browse PNGs when available.
    let selected_browse = browse_candidates
        .iter()
        .find(|u| u.contains("/lp-prod-public/"))
        .or_else(|| browse_candidates.first())
        .cloned();

    let selected_data = match spec.code {
        "VNP46A3" | "VNP46A4" | "VNP46A2" => data_candidates
            .iter()
            .find(|u| {
                let b = url_basename_lower(u);
                b.ends_with(".h5") || b.ends_with(".hdf5")
            })
            .cloned(),
        "EMITL2ARFL" => data_candidates
            .iter()
            .find(|u| {
                let b = url_basename_lower(u);
                b.ends_with(".nc")
                    && b.contains("_rfl_")
                    && !b.contains("uncert")
                    && !b.contains("mask")
            })
            .or_else(|| {
                data_candidates.iter().find(|u| {
                    let b = url_basename_lower(u);
                    b.ends_with(".nc") && !b.contains("uncert") && !b.contains("mask")
                })
            })
            .or_else(|| {
                data_candidates
                    .iter()
                    .find(|u| url_basename_lower(u).ends_with(".nc"))
            })
            .cloned(),
        "EMITL2BMIN" => data_candidates
            .iter()
            .find(|u| {
                let b = url_basename_lower(u);
                b.ends_with(".nc") && b.contains("_min_") && !b.contains("uncert")
            })
            .or_else(|| {
                data_candidates.iter().find(|u| {
                    let b = url_basename_lower(u);
                    b.ends_with(".nc") && !b.contains("uncert") && !b.contains("mask")
                })
            })
            .or_else(|| {
                data_candidates
                    .iter()
                    .find(|u| url_basename_lower(u).ends_with(".nc"))
            })
            .cloned(),
        "ECO_L2T_LSTE" => data_candidates
            .iter()
            .find(|u| {
                let b = url_basename_lower(u);
                b.ends_with("_lst.tif") || b.ends_with("_lst.tiff")
            })
            .cloned(),
        "ECO_L2T_LSTE_EMIS" => data_candidates
            .iter()
            .find(|u| {
                let b = url_basename_lower(u);
                b.ends_with("_emiswb.tif") || b.ends_with("_emiswb.tiff")
            })
            .cloned(),
        "ECO_L3T_JET" => data_candidates
            .iter()
            .find(|u| {
                let b = url_basename_lower(u);
                b.ends_with("_etdaily.tif") || b.ends_with("_etdaily.tiff")
            })
            .cloned(),
        _ => data_candidates.first().cloned(),
    };

    (selected_data, selected_browse)
}

fn bbox_polygon_geojson(bbox: [f64; 4]) -> Value {
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
}

fn parse_cmr_geometry_and_bbox(entry: &Value, fallback_bbox: [f64; 4]) -> ([f64; 4], Value) {
    // 1. Prefer CMR `polygons` (`[["lat1 lon1 lat2 lon2 ..."]]`).
    if let Some(first_ring_str) = entry
        .get("polygons")
        .and_then(Value::as_array)
        .and_then(|polys| polys.first())
        .and_then(Value::as_array)
        .and_then(|rings| rings.first())
        .and_then(Value::as_str)
    {
        let nums: Vec<f64> = first_ring_str
            .split_whitespace()
            .filter_map(|tok| tok.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .collect();
        let mut coords = Vec::new();
        let mut min_lon = f64::INFINITY;
        let mut min_lat = f64::INFINITY;
        let mut max_lon = f64::NEG_INFINITY;
        let mut max_lat = f64::NEG_INFINITY;
        for pair in nums.chunks(2) {
            if pair.len() == 2 {
                let lat = pair[0];
                let lon = pair[1];
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                coords.push(json!([lon, lat]));
            }
        }
        if coords.len() >= 3 && coords.first() != coords.last() {
            if let Some(first) = coords.first().cloned() {
                coords.push(first);
            }
        }
        if coords.len() >= 4 && min_lon.is_finite() && min_lat.is_finite() {
            let item_bbox = [min_lon, min_lat, max_lon, max_lat];
            return (
                item_bbox,
                json!({ "type": "Polygon", "coordinates": [coords] }),
            );
        }
    }

    // 2. Fallback to CMR `boxes` (`["south west north east"]`), common on tiled grids like VNP46A*.
    if let Some(first_box_str) = entry
        .get("boxes")
        .and_then(Value::as_array)
        .and_then(|boxes| boxes.first())
        .and_then(Value::as_str)
    {
        let nums: Vec<f64> = first_box_str
            .split_whitespace()
            .filter_map(|tok| tok.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .collect();
        if nums.len() == 4 {
            let south = nums[0].min(nums[2]);
            let north = nums[0].max(nums[2]);
            let west = nums[1].min(nums[3]);
            let east = nums[1].max(nums[3]);
            let item_bbox = [west, south, east, north];
            return (item_bbox, bbox_polygon_geojson(item_bbox));
        }
    }

    (fallback_bbox, bbox_polygon_geojson(fallback_bbox))
}

fn parse_cmr_granule_size_bytes(entry: &Value) -> Option<u64> {
    let mb = match entry.get("granule_size") {
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        Some(Value::Number(n)) => n.as_f64(),
        _ => None,
    }?;
    if mb.is_finite() && mb > 0.0 {
        Some((mb * 1024.0 * 1024.0).round() as u64)
    } else {
        None
    }
}

pub fn infer_file_extension(href: &str, default_ext: &str) -> &'static str {
    let base = url_basename_lower(href);
    if base.ends_with(".tif") || base.ends_with(".tiff") {
        "tif"
    } else if base.ends_with(".h5") || base.ends_with(".hdf5") || base.ends_with(".hdf") {
        "h5"
    } else if base.ends_with(".nc") || base.ends_with(".nc4") {
        "nc"
    } else {
        match default_ext {
            "tif" => "tif",
            "h5" => "h5",
            _ => "nc",
        }
    }
}

/// Parses a NASA CMR `granules.json` feed into normalized workbench/agent items.
pub fn parse_cmr_granules_response(
    spec: &OpenEoSubproductSpec,
    cmr_json: &Value,
    fallback_bbox: [f64; 4],
) -> Vec<Value> {
    let entries = cmr_json
        .get("feed")
        .and_then(|f| f.get("entry"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::with_capacity(entries.len());
    for entry in &entries {
        let links = entry
            .get("links")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let (data_url_opt, browse_url_opt) = select_cmr_granule_links(spec, links);
        let Some(data_url) = data_url_opt else {
            // Skip incomplete catalog records that lack the requested sub-product file
            // (e.g., an ECOSTRESS granule without `_LST.tif` or `_EmisWB.tif`).
            continue;
        };

        let raw_stem_source = if matches!(spec.dataset, OpenEoDataset::NasaEcostress) {
            // Use the exact layer filename stem (`..._LST`, `..._EmisWB`, `..._ETdaily`)
            // so LST and EmisWB layers from the same granule have distinct IDs and filenames.
            data_url.as_str()
        } else {
            entry
                .get("producer_granule_id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(data_url.as_str())
        };
        let id = sanitize_granule_stem(raw_stem_source);
        let ext = infer_file_extension(&data_url, spec.default_ext);
        let file_name = format!("{id}.{ext}");

        let datetime = entry
            .get("time_start")
            .and_then(Value::as_str)
            .or_else(|| entry.get("time_end").and_then(Value::as_str))
            .unwrap_or("2024-01-01T00:00:00Z")
            .to_owned();
        let end_date = entry
            .get("time_end")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let (item_bbox, geometry) = parse_cmr_geometry_and_bbox(entry, fallback_bbox);
        let size_bytes = parse_cmr_granule_size_bytes(entry);

        let public_preview_url = browse_url_opt
            .as_deref()
            .filter(|u| {
                u.starts_with("https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-public/")
            })
            .map(str::to_owned);

        let day_night = entry
            .get("day_night_flag")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let cloud_cover = entry.get("cloud_cover").and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str()?.trim().parse::<f64>().ok())
        });

        out.push(json!({
            "id": id,
            "title": file_name,
            "short_name": spec.short_name,
            "sub_product": spec.code,
            "product_type": spec.code,
            "science_level": spec.science_level,
            "collection_concept_id": spec.collection_concept_id,
            "platform": spec.dataset.platform_name(),
            "resolution_m": spec.resolution_m,
            "datetime": datetime,
            "end_date": end_date,
            "day_night_flag": day_night,
            "cloud_cover": cloud_cover,
            "file_size_bytes": size_bytes,
            "file_ext": ext,
            "href": data_url,
            "data_url": data_url,
            "browse_url": browse_url_opt,
            "thumbnail_url": public_preview_url,
            "preview_url": public_preview_url,
            "search_only": false,
            "bbox": item_bbox,
            "geometry": geometry,
        }));
    }
    out
}

/// Parses German DLR EOC STAC `ENMAP_HSI_L2A` features into honest `search_only`
/// workbench/agent items (never exposing CAS SSO redirect links as direct binary downloads).
pub fn parse_dlr_enmap_stac_response(stac_json: &Value, fallback_bbox: [f64; 4]) -> Vec<Value> {
    let features = stac_json
        .get("features")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::with_capacity(features.len());
    for feat in &features {
        let raw_id = feat
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("ENMAP_HSI_L2A_SCENE");
        let id = sanitize_granule_stem(raw_id);
        let props = feat.get("properties").unwrap_or(&Value::Null);
        let datetime = props
            .get("datetime")
            .and_then(Value::as_str)
            .or_else(|| props.get("start_datetime").and_then(Value::as_str))
            .unwrap_or("2024-01-01T00:00:00Z")
            .to_owned();
        let cloud_cover = props.get("eo:cloud_cover").and_then(Value::as_f64);

        let item_bbox = feat
            .get("bbox")
            .and_then(Value::as_array)
            .and_then(|arr| {
                if arr.len() >= 4 {
                    let w = arr[0].as_f64()?;
                    let s = arr[1].as_f64()?;
                    let e = arr[2].as_f64()?;
                    let n = arr[3].as_f64()?;
                    if w.is_finite() && s.is_finite() && e.is_finite() && n.is_finite() {
                        Some([w.min(e), s.min(n), w.max(e), s.max(n)])
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or(fallback_bbox);

        let geometry = feat
            .get("geometry")
            .filter(|g| g.is_object())
            .cloned()
            .unwrap_or_else(|| bbox_polygon_geojson(item_bbox));

        let portal_spectral_href = feat
            .get("assets")
            .and_then(Value::as_object)
            .and_then(|assets| {
                assets
                    .get("image")
                    .or_else(|| assets.get("VNIR"))
                    .or_else(|| assets.get("data"))
                    .and_then(|a| a.get("href"))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned);

        out.push(json!({
            "id": id,
            "title": format!("{id}-SPECTRAL_IMAGE.TIF"),
            "short_name": "ENMAP_HSI_L2A",
            "sub_product": "ENMAP_HSI_L2A",
            "product_type": "ENMAP_HSI_L2A",
            "science_level": "L2A",
            "platform": "EnMAP",
            "resolution_m": 30.0,
            "datetime": datetime,
            "eo:cloud_cover": cloud_cover,
            "cloud_cover": cloud_cover,
            "href": "",
            "data_url": "",
            "portal_href": portal_spectral_href,
            "auth_portal_url": DLR_EOWEB_PORTAL_URL,
            "search_only": true,
            "bbox": item_bbox,
            "geometry": geometry,
        }));
    }
    out
}

/// Validates the first bytes of a downloaded file to reject HTML login/SSO pages
/// and verify genuine GeoTIFF / BigTIFF / HDF5 / NetCDF binary magic signatures.
pub fn validate_science_binary_magic(
    head: &[u8],
    item_id: &str,
    auth_err_msg: &str,
) -> Result<(), String> {
    let head_ascii = String::from_utf8_lossy(head)
        .trim_start()
        .to_ascii_lowercase();
    if head_ascii.starts_with("<!doctype")
        || head_ascii.starts_with("<html")
        || head_ascii.contains("earthdata login")
        || head_ascii.contains("sso.eoc.dlr.de")
    {
        return Err(auth_err_msg.to_owned());
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
        return Err(format!(
            "SCIENCE_BINARY_MAGIC_INVALID: 下载的 {item_id} 非合法 GeoTIFF/HDF5/NetCDF 二进制文件"
        ));
    }
    Ok(())
}

/// Executes a live public metadata search against NASA CMR (`granules.json`) or
/// German DLR EOC STAC (`/eoc/ogc/stac/v1/search`).
pub fn fetch_open_eo_catalog_items(
    client: &reqwest::blocking::Client,
    spec: &OpenEoSubproductSpec,
    bbox: [f64; 4],
    temporal: Option<(&str, &str)>,
    limit: usize,
) -> Result<Vec<Value>, String> {
    fetch_open_eo_catalog_items_with_endpoints(
        client,
        spec,
        bbox,
        temporal,
        limit,
        NASA_CMR_GRANULES_JSON_URL,
        DLR_EOC_STAC_SEARCH_URL,
    )
}

pub fn fetch_open_eo_catalog_items_with_endpoints(
    client: &reqwest::blocking::Client,
    spec: &OpenEoSubproductSpec,
    bbox: [f64; 4],
    temporal: Option<(&str, &str)>,
    limit: usize,
    cmr_granules_url: &str,
    dlr_stac_search_url: &str,
) -> Result<Vec<Value>, String> {
    let normalized_bbox = normalize_wgs84_bbox(bbox)?;
    match spec.dataset {
        OpenEoDataset::NasaBlackMarble | OpenEoDataset::NasaEmit | OpenEoDataset::NasaEcostress => {
            let default_url =
                build_cmr_granules_search_url(spec, normalized_bbox, temporal, limit)?;
            let url = if cmr_granules_url == NASA_CMR_GRANULES_JSON_URL {
                default_url
            } else {
                default_url.replacen(NASA_CMR_GRANULES_JSON_URL, cmr_granules_url, 1)
            };
            let mut last_err: Option<String> = None;
            let mut ok_payload: Option<Value> = None;
            for (attempt, delay_ms) in [0_u64, 500, 1500].into_iter().enumerate() {
                if attempt > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                }
                match client
                    .get(&url)
                    .header(reqwest::header::ACCEPT, "application/json")
                    .send()
                {
                    Ok(resp) => {
                        let status = resp.status();
                        if matches!(status.as_u16(), 429 | 502 | 503 | 504) && attempt < 2 {
                            last_err = Some(format!(
                                "CMR_SEARCH_HTTP_ERROR ({}): HTTP {status}",
                                spec.code
                            ));
                            continue;
                        }
                        if !status.is_success() {
                            return Err(format!(
                                "CMR_SEARCH_HTTP_ERROR ({}): HTTP {status}",
                                spec.code
                            ));
                        }
                        ok_payload = Some(resp.json().map_err(|err| {
                            format!("CMR_JSON_PARSE_FAILED ({}): {err}", spec.code)
                        })?);
                        break;
                    }
                    Err(err) => {
                        last_err = Some(format!("CMR_SEARCH_FAILED ({}): {err:?}", spec.code));
                        if attempt == 2 {
                            break;
                        }
                    }
                }
            }
            let payload = ok_payload.ok_or_else(|| {
                last_err
                    .unwrap_or_else(|| format!("CMR_SEARCH_FAILED ({}): unknown error", spec.code))
            })?;
            Ok(parse_cmr_granules_response(spec, &payload, normalized_bbox))
        }
        OpenEoDataset::DlrEnmap => {
            let body = build_dlr_enmap_stac_search_payload(normalized_bbox, temporal, limit)?;
            let mut last_err: Option<String> = None;
            let mut ok_payload: Option<Value> = None;
            for (attempt, delay_ms) in [0_u64, 500, 1500].into_iter().enumerate() {
                if attempt > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                }
                match client
                    .post(dlr_stac_search_url)
                    .header(
                        reqwest::header::ACCEPT,
                        "application/geo+json, application/json",
                    )
                    .json(&body)
                    .send()
                {
                    Ok(resp) => {
                        let status = resp.status();
                        if matches!(status.as_u16(), 429 | 502 | 503 | 504) && attempt < 2 {
                            last_err = Some(format!("DLR_ENMAP_STAC_HTTP_ERROR: HTTP {status}"));
                            continue;
                        }
                        if !status.is_success() {
                            return Err(format!("DLR_ENMAP_STAC_HTTP_ERROR: HTTP {status}"));
                        }
                        ok_payload = Some(
                            resp.json()
                                .map_err(|err| format!("DLR_ENMAP_STAC_JSON_FAILED: {err}"))?,
                        );
                        break;
                    }
                    Err(err) => {
                        last_err = Some(format!("DLR_ENMAP_STAC_SEARCH_FAILED: {err:?}"));
                        if attempt == 2 {
                            break;
                        }
                    }
                }
            }
            let payload = ok_payload.ok_or_else(|| {
                last_err.unwrap_or_else(|| "DLR_ENMAP_STAC_SEARCH_FAILED: unknown error".to_owned())
            })?;
            Ok(parse_dlr_enmap_stac_response(&payload, normalized_bbox))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_proxy::loopback::serve_once;
    use std::time::Duration;

    #[test]
    fn subproduct_resolution_covers_all_four_open_eo_datasets_and_aliases() {
        assert_eq!(
            resolve_open_eo_subproduct(OpenEoDataset::NasaBlackMarble, None).code,
            "VNP46A3"
        );
        assert_eq!(
            resolve_open_eo_subproduct(OpenEoDataset::NasaBlackMarble, Some("VNP46A4")).code,
            "VNP46A4"
        );
        assert_eq!(
            resolve_open_eo_subproduct(OpenEoDataset::NasaBlackMarble, Some("daily")).code,
            "VNP46A2"
        );
        assert_eq!(
            resolve_open_eo_subproduct(OpenEoDataset::NasaEmit, None).code,
            "EMITL2ARFL"
        );
        assert_eq!(
            resolve_open_eo_subproduct(OpenEoDataset::NasaEmit, Some("EMITL2BMIN")).code,
            "EMITL2BMIN"
        );
        assert_eq!(
            resolve_open_eo_subproduct(OpenEoDataset::NasaEcostress, None).code,
            "ECO_L2T_LSTE"
        );
        assert_eq!(
            resolve_open_eo_subproduct(OpenEoDataset::NasaEcostress, Some("ECO_L2T_LSTE_EMIS"))
                .code,
            "ECO_L2T_LSTE_EMIS"
        );
        assert_eq!(
            resolve_open_eo_subproduct(OpenEoDataset::NasaEcostress, Some("ECO_L3T_JET")).code,
            "ECO_L3T_JET"
        );
        let enmap = resolve_open_eo_subproduct(OpenEoDataset::DlrEnmap, None);
        assert_eq!(enmap.code, "ENMAP_HSI_L2A");
        assert!(!enmap.download_supported);
        assert!(!OpenEoDataset::DlrEnmap.download_supported());
    }

    #[test]
    fn sanitize_granule_stem_strips_extensions_and_windows_forbidden_colons() {
        assert_eq!(
            sanitize_granule_stem("VNP46A3.A2025001.h29v05.002.2025041142011.h5"),
            "VNP46A3.A2025001.h29v05.002.2025041142011"
        );
        assert_eq!(
            sanitize_granule_stem("LAADS:10129677384"),
            "LAADS_10129677384"
        );
    }

    #[test]
    fn multi_asset_cmr_selector_isolates_exact_science_layers_and_public_png_quicklook() {
        let eco_links = vec![
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/data#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/ECO_L2T_LSTE.002/ECOv002_L2T_LSTE_36789_005_50SNA_20250102T051210_0713_01/ECOv002_L2T_LSTE_36789_005_50SNA_20250102T051210_0713_01_water.tif"}),
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/data#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/ECO_L2T_LSTE.002/ECOv002_L2T_LSTE_36789_005_50SNA_20250102T051210_0713_01/ECOv002_L2T_LSTE_36789_005_50SNA_20250102T051210_0713_01_LST.tif"}),
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/data#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/ECO_L2T_LSTE.002/ECOv002_L2T_LSTE_36789_005_50SNA_20250102T051210_0713_01/ECOv002_L2T_LSTE_36789_005_50SNA_20250102T051210_0713_01_EmisWB.tif"}),
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/browse#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-public/ECO_L2T_LSTE.002/ECOv002_L2T_LSTE_36789_005_50SNA_20250102T051210_0713_01/ECOv002_L2T_LSTE_36789_005_50SNA_20250102T051210_0713_01_LST.png"}),
        ];
        let lst_spec =
            resolve_open_eo_subproduct(OpenEoDataset::NasaEcostress, Some("ECO_L2T_LSTE"));
        let emis_spec =
            resolve_open_eo_subproduct(OpenEoDataset::NasaEcostress, Some("ECO_L2T_LSTE_EMIS"));

        let (lst_href, lst_browse) = select_cmr_granule_links(lst_spec, &eco_links);
        assert!(lst_href.unwrap().ends_with("_LST.tif"));
        assert!(lst_browse.unwrap().ends_with("_LST.png"));

        let (emis_href, _) = select_cmr_granule_links(emis_spec, &eco_links);
        assert!(emis_href.unwrap().ends_with("_EmisWB.tif"));

        let jet_spec =
            resolve_open_eo_subproduct(OpenEoDataset::NasaEcostress, Some("ECO_L3T_JET"));
        let jet_without_daily_links = vec![
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/data#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/ECO_L3T_JET.002/G1/G1_PTJPLSMinst.tif"}),
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/data#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/ECO_L3T_JET.002/G1/G1_ETinstUncertainty.tif"}),
        ];
        assert_eq!(
            select_cmr_granule_links(jet_spec, &jet_without_daily_links).0,
            None,
            "ECO_L3T_JET must reject non-_ETdaily.tif layers instead of falling back to _PTJPLSMinst.tif"
        );
        let skipped_cmr = json!({
            "feed": {
                "entry": [
                    {
                        "producer_granule_id": "ECOv002_L3T_JET_MISSING_DAILY",
                        "time_start": "2025-01-02T00:00:00Z",
                        "links": jet_without_daily_links
                    },
                    {
                        "producer_granule_id": "ECOv002_L3T_JET_WITH_DAILY",
                        "time_start": "2025-01-03T00:00:00Z",
                        "links": [
                            {"rel": "http://esipfed.org/ns/fedsearch/1.1/data#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/ECO_L3T_JET.002/G2/ECOv002_L3T_JET_G2_ETdaily.tif"}
                        ]
                    }
                ]
            }
        });
        let jet_items =
            parse_cmr_granules_response(jet_spec, &skipped_cmr, [116.0, 39.0, 117.0, 40.0]);
        assert_eq!(jet_items.len(), 1);
        assert!(jet_items[0]["href"]
            .as_str()
            .unwrap()
            .ends_with("_ETdaily.tif"));

        let emit_links = vec![
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/data#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/EMITL2ARFL.001/EMIT_L2A_RFL_001_20240510T041200_2413103_004/EMIT_L2A_RFLUNCERT_001_20240510T041200_2413103_004.nc"}),
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/data#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/EMITL2ARFL.001/EMIT_L2A_RFL_001_20240510T041200_2413103_004/EMIT_L2A_MASK_001_20240510T041200_2413103_004.nc"}),
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/data#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/EMITL2ARFL.001/EMIT_L2A_RFL_001_20240510T041200_2413103_004/EMIT_L2A_RFL_001_20240510T041200_2413103_004.nc"}),
            json!({"rel": "http://esipfed.org/ns/fedsearch/1.1/browse#", "href": "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-public/EMITL2ARFL.001/EMIT_L2A_RFL_001_20240510T041200_2413103_004/EMIT_L2A_RFL_001_20240510T041200_2413103_004.png"}),
        ];
        let emit_spec = resolve_open_eo_subproduct(OpenEoDataset::NasaEmit, Some("EMITL2ARFL"));
        let (emit_href, emit_browse) = select_cmr_granule_links(emit_spec, &emit_links);
        assert!(emit_href
            .unwrap()
            .ends_with("EMIT_L2A_RFL_001_20240510T041200_2413103_004.nc"));
        assert!(emit_browse.unwrap().contains("/lp-prod-public/"));
    }

    #[test]
    fn enmap_stac_parser_marks_items_search_only_and_clears_direct_download_href() {
        let sample = json!({
            "type": "FeatureCollection",
            "features": [{
                "id": "ENMAP01-____L2A-DT0000089412_20240915T033211Z_001_V010402_20240916T120000Z",
                "bbox": [116.2, 39.7, 116.6, 40.1],
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[[116.2, 39.7], [116.6, 39.7], [116.6, 40.1], [116.2, 40.1], [116.2, 39.7]]]
                },
                "properties": {
                    "datetime": "2024-09-15T03:32:11Z",
                    "eo:cloud_cover": 4.2
                },
                "assets": {
                    "image": {
                        "href": "https://download.geoservice.dlr.de/ENMAP/files/L2A/ENMAP01-SPECTRAL_IMAGE.TIF"
                    }
                }
            }]
        });
        let items = parse_dlr_enmap_stac_response(&sample, [116.0, 39.5, 117.0, 40.5]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["search_only"], json!(true));
        assert_eq!(items[0]["href"], json!(""));
        assert_eq!(items[0]["auth_portal_url"], json!(DLR_EOWEB_PORTAL_URL));
        assert!(items[0]["portal_href"]
            .as_str()
            .unwrap()
            .starts_with("https://download.geoservice.dlr.de/"));
    }

    #[test]
    fn science_magic_validator_rejects_html_sso_pages_and_accepts_hdf5_geotiff_netcdf() {
        let html_err = validate_science_binary_magic(
            b"<!DOCTYPE html><html><title>Earthdata Login</title>",
            "VNP46A3.test",
            EARTHDATA_BLACK_MARBLE_AUTH_ERROR,
        )
        .unwrap_err();
        assert_eq!(html_err, EARTHDATA_BLACK_MARBLE_AUTH_ERROR);

        let dlr_err = validate_science_binary_magic(
            b"<html><head><meta content='sso.eoc.dlr.de'></head></html>",
            "ENMAP.test",
            DLR_ENMAP_SSO_DOWNLOAD_ERROR,
        )
        .unwrap_err();
        assert_eq!(dlr_err, DLR_ENMAP_SSO_DOWNLOAD_ERROR);

        assert!(validate_science_binary_magic(
            b"\x89HDF\r\n\x1a\npayload",
            "VNP46A3.test",
            EARTHDATA_BLACK_MARBLE_AUTH_ERROR
        )
        .is_ok());
        assert!(validate_science_binary_magic(
            b"II*\0payload",
            "ECO.test",
            EARTHDATA_ECOSTRESS_AUTH_ERROR
        )
        .is_ok());
        assert!(validate_science_binary_magic(
            b"CDF\x02payload",
            "EMIT.test",
            EARTHDATA_EMIT_AUTH_ERROR
        )
        .is_ok());
    }

    #[test]
    fn loopback_http_transport_maps_cmr_and_enmap_responses_deterministically() {
        const CMR_FIXTURE: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"feed\":{\"entry\":[{\"title\":\"LAADS:991\",\"producer_granule_id\":\"VNP46A3.A2025001.h29v05.002.h5\",\"time_start\":\"2025-01-01T00:00:00Z\",\"boxes\":[\"30 110 40 120\"],\"granule_size\":\"18.5\",\"links\":[{\"rel\":\"http://esipfed.org/ns/fedsearch/1.1/data#\",\"href\":\"https://data.laadsdaac.earthdatacloud.nasa.gov/prod/VNP46A3/VNP46A3.A2025001.h29v05.002.h5\"}]}]}}";
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let (cmr_addr, cmr_handle) = serve_once(CMR_FIXTURE);
        let bm_spec = resolve_open_eo_subproduct(OpenEoDataset::NasaBlackMarble, Some("VNP46A3"));
        let items = fetch_open_eo_catalog_items_with_endpoints(
            &client,
            bm_spec,
            [116.0, 39.5, 117.0, 40.5],
            Some(("2025-01-01", "2025-01-31")),
            5,
            &format!("http://{cmr_addr}/granules.json"),
            "http://127.0.0.1:9/unused",
        )
        .unwrap();
        let req_line = cmr_handle.join().unwrap().unwrap();
        assert!(req_line.contains("collection_concept_id=C3860061042-LAADS"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], json!("VNP46A3.A2025001.h29v05.002"));
        assert_eq!(items[0]["bbox"], json!([110.0, 30.0, 120.0, 40.0]));
    }

    #[test]
    #[ignore = "live public NASA CMR + DLR EnMAP STAC smoke test; run with --ignored"]
    fn live_public_catalogs_return_nonempty_scientific_items_across_all_four_datasets() {
        let builder = crate::network_proxy::blocking_builder_with_proxy(
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(25))
                .user_agent("EOStage-OpenEO-Test/1.0"),
            insar_core::ProxyPolicy::System,
        );
        let client = builder.build().expect("reqwest client");

        // 1. NASA Black Marble VNP46A3 (Beijing/North China)
        let bm_spec = resolve_open_eo_subproduct(OpenEoDataset::NasaBlackMarble, Some("VNP46A3"));
        let bm_items = fetch_open_eo_catalog_items(
            &client,
            bm_spec,
            [116.1, 39.7, 116.7, 40.2],
            Some(("2024-01-01", "2024-12-31")),
            5,
        )
        .expect("live Black Marble CMR search");
        assert!(
            !bm_items.is_empty(),
            "Black Marble VNP46A3 should return granules"
        );
        assert!(!bm_items[0]["id"].as_str().unwrap().contains(':'));
        assert!(bm_items[0]["href"].as_str().unwrap().ends_with(".h5"));

        // 2. NASA JPL EMIT L2A RFL (Dunhuang / NW China arid mineralogy belt)
        let emit_spec = resolve_open_eo_subproduct(OpenEoDataset::NasaEmit, Some("EMITL2ARFL"));
        let emit_items = fetch_open_eo_catalog_items(
            &client,
            emit_spec,
            [94.0, 39.5, 95.5, 40.8],
            Some(("2023-01-01", "2025-12-31")),
            5,
        )
        .expect("live EMIT CMR search");
        assert!(
            !emit_items.is_empty(),
            "EMIT L2A RFL should return granules"
        );
        let emit_href = emit_items[0]["href"].as_str().unwrap();
        assert!(emit_href.ends_with(".nc") && emit_href.to_ascii_lowercase().contains("_rfl_"));
        assert!(emit_items[0]["preview_url"]
            .as_str()
            .unwrap_or("")
            .contains("/lp-prod-public/"));

        // 3. NASA JPL ECOSTRESS L2T LSTE (Beijing/North China)
        let eco_spec =
            resolve_open_eo_subproduct(OpenEoDataset::NasaEcostress, Some("ECO_L2T_LSTE"));
        let eco_items = fetch_open_eo_catalog_items(
            &client,
            eco_spec,
            [116.1, 39.7, 116.7, 40.2],
            Some(("2024-06-01", "2025-06-01")),
            5,
        )
        .expect("live ECOSTRESS CMR search");
        assert!(
            !eco_items.is_empty(),
            "ECOSTRESS L2T LSTE should return granules"
        );
        assert!(eco_items[0]["href"].as_str().unwrap().ends_with("_LST.tif"));

        // 4. German DLR EnMAP L2A (Beijing/North China)
        let enmap_spec = resolve_open_eo_subproduct(OpenEoDataset::DlrEnmap, None);
        let enmap_items = fetch_open_eo_catalog_items(
            &client,
            enmap_spec,
            [116.0, 39.6, 116.8, 40.3],
            Some(("2023-01-01", "2025-12-31")),
            5,
        )
        .expect("live DLR EnMAP STAC search");
        assert!(
            !enmap_items.is_empty(),
            "DLR EnMAP STAC should return features"
        );
        assert_eq!(enmap_items[0]["search_only"], json!(true));
        assert_eq!(enmap_items[0]["href"], json!(""));
    }
}
