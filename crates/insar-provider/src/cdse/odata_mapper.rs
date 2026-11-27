use std::collections::{BTreeMap, HashMap};

use insar_core::{
    provider_product_key, stable_asset_key, AssetRole, CatalogCollectionDescriptor,
    CatalogSearchResponse, CatalogSearchStats, ContractError, DataSourceKind, ProductKind,
    ProviderAsset, ProviderProduct, ProviderResult,
};
use serde_json::{json, Map, Value};
use url::Url;
use uuid::Uuid;

use super::archive_filename::safe_cdse_odata_name;
use super::odata_types::{ODataAsset, ODataAttribute, ODataPage, ODataProduct};
use super::query::{normalize_utc, parse_utc_text, Sentinel2SearchFacts};

const PROVIDER_ID: &str = "copernicus.dataspace.odata";
pub const CDSE_ODATA_ASSET_VALUE_BASE: &str =
    "https://catalogue.dataspace.copernicus.eu/odata/v1/Assets";
const MAX_QUICKLOOK_BYTES: u64 = 16 * 1024 * 1024;

fn error(code: &str, message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_string(), Value::String(reason.to_string()))]),
    )
    .expect("bounded CDSE mapper context is safe")
}

pub fn sentinel2_collection(facts: &Sentinel2SearchFacts) -> CatalogCollectionDescriptor {
    facts.collection_descriptor()
}

fn authoritative_text(value: Option<&String>, field: &str) -> ProviderResult<String> {
    let value = value.map(String::as_str).ok_or_else(|| {
        error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 OData 记录缺少必需字段",
            field,
        )
    })?;
    if value.is_empty() || value != value.trim() || value.chars().any(char::is_control) {
        return Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 OData 权威文本无效",
            field,
        ));
    }
    Ok(value.to_string())
}

fn canonical_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .map(|parsed| parsed.hyphenated().to_string().eq_ignore_ascii_case(value))
        .unwrap_or(false)
}

fn canonical_lower_uuid(value: &str) -> Option<String> {
    let parsed = Uuid::parse_str(value).ok()?;
    let canonical = parsed.hyphenated().to_string();
    (canonical == value).then_some(canonical)
}

fn safe_quicklook_name(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.is_ascii()
        && value.encode_utf16().count() <= 255
        && !value.starts_with('.')
        && !value.ends_with(['.', ' '])
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn quicklook_media_rank(value: &str) -> Option<u8> {
    match value {
        "image/jpeg" => Some(0),
        "image/png" => Some(1),
        "image/webp" => Some(2),
        _ => None,
    }
}

fn quicklook_name_rank(value: &str) -> Option<u8> {
    let value = value.to_ascii_lowercase();
    if value.contains("quicklook") {
        Some(0)
    } else if value.contains("thumbnail") {
        Some(1)
    } else if value.contains("preview") {
        Some(2)
    } else if value.contains("browse") {
        Some(3)
    } else {
        None
    }
}

fn looks_like_quicklook(asset: &ODataAsset) -> bool {
    asset.asset_type.as_deref() == Some("QUICKLOOK")
        || asset
            .name
            .as_deref()
            .is_some_and(|value| quicklook_name_rank(value).is_some())
        || asset
            .content_type
            .as_deref()
            .is_some_and(|value| value.starts_with("image/"))
}

fn quicklook_extension_matches_media_type(name: &str, media_type: &str) -> bool {
    let name = name.to_ascii_lowercase();
    match media_type {
        "image/jpeg" => name.ends_with(".jpg") || name.ends_with(".jpeg"),
        "image/png" => name.ends_with(".png"),
        "image/webp" => name.ends_with(".webp"),
        _ => false,
    }
}

fn expected_quicklook_url(asset_id: &str) -> String {
    format!("{CDSE_ODATA_ASSET_VALUE_BASE}({asset_id})/$value")
}

fn validate_quicklook_locator(locator: Option<&String>, asset_id: &str) -> ProviderResult<()> {
    let Some(locator) = locator else {
        return Ok(());
    };
    let locator = authoritative_text(Some(locator), "quicklook.download_link")?;
    let parsed = Url::parse(&locator).map_err(|_| {
        error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 快视图地址无效",
            "quicklook.download_link",
        )
    })?;
    let expected = expected_quicklook_url(asset_id);
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("catalogue.dataspace.copernicus.eu")
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.as_str() != expected
    {
        return Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 快视图地址不受信任",
            "quicklook.download_link",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuicklookCandidate {
    asset_id: String,
    name: String,
    media_type: String,
    size_bytes: Option<u64>,
    name_rank: u8,
    media_rank: u8,
}

/// Closed, runtime-only evidence for one optional CDSE static quicklook.
///
/// Search/freeze deliberately does not carry this value into a SearchBatch.
/// It is derived only by the dedicated lazy asset-discovery operation from a
/// previously frozen product UUID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CdseQuicklookDescriptor {
    pub asset_id: String,
    pub media_type: String,
    pub expected_length: Option<u64>,
}

fn map_quicklook_candidate(asset: &ODataAsset) -> ProviderResult<Option<QuicklookCandidate>> {
    let declared_type = asset
        .asset_type
        .as_ref()
        .map(|value| authoritative_text(Some(value), "quicklook.type"))
        .transpose()?;
    if declared_type
        .as_deref()
        .is_some_and(|value| value != "QUICKLOOK")
    {
        return Ok(None);
    }
    let declared_quicklook = declared_type.as_deref() == Some("QUICKLOOK");
    if !looks_like_quicklook(asset) {
        return Ok(None);
    }
    let (media_type, media_rank) = match asset.content_type.as_ref() {
        Some(value) => {
            let media_type = authoritative_text(Some(value), "quicklook.content_type")?;
            let media_rank = quicklook_media_rank(&media_type).ok_or_else(|| {
                error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 快视图媒体类型不受支持",
                    "quicklook.content_type",
                )
            })?;
            (media_type, media_rank)
        }
        None if declared_quicklook => ("image/*".to_owned(), 3),
        None => {
            return Err(error(
                "CDSE_ODATA_ITEM_INVALID",
                "Sentinel-2 快视图媒体类型缺失",
                "quicklook.content_type",
            ));
        }
    };
    let asset_id = asset
        .id
        .as_deref()
        .and_then(canonical_lower_uuid)
        .ok_or_else(|| {
            error(
                "CDSE_ODATA_ITEM_INVALID",
                "Sentinel-2 快视图身份无效",
                "quicklook.uuid",
            )
        })?;
    let (name, name_rank) = match asset.name.as_ref() {
        Some(value) => {
            let name = authoritative_text(Some(value), "quicklook.name")?;
            if !safe_quicklook_name(&name) {
                return Err(error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 快视图名称无效",
                    "quicklook.name",
                ));
            }
            let name_rank = quicklook_name_rank(&name).ok_or_else(|| {
                error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 快视图名称不受支持",
                    "quicklook.name",
                )
            })?;
            if media_type != "image/*"
                && !quicklook_extension_matches_media_type(&name, &media_type)
            {
                return Err(error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 快视图文件扩展名无效",
                    "quicklook.name",
                ));
            }
            (name, name_rank)
        }
        None if declared_quicklook => ("QUICKLOOK".to_owned(), 4),
        None => {
            return Err(error(
                "CDSE_ODATA_ITEM_INVALID",
                "Sentinel-2 快视图名称缺失",
                "quicklook.name",
            ));
        }
    };
    let size_bytes = asset
        .content_length
        .map(|value| {
            if value == 0 || value > MAX_QUICKLOOK_BYTES {
                return Err(error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 快视图大小无效",
                    "quicklook.content_length",
                ));
            }
            Ok(value)
        })
        .transpose()?;
    validate_quicklook_locator(asset.download_link.as_ref(), &asset_id)?;
    Ok(Some(QuicklookCandidate {
        asset_id,
        name,
        media_type,
        size_bytes,
        name_rank,
        media_rank,
    }))
}

/// Deterministically select zero or one reviewed static quicklook from the
/// exact-product `Assets` response. Any malformed or conflicting quicklook is
/// a preview-only failure; bulk product search never calls this function.
pub fn select_quicklook_descriptor(
    source_assets: &[ODataAsset],
) -> ProviderResult<Option<CdseQuicklookDescriptor>> {
    let mut candidates = BTreeMap::<String, QuicklookCandidate>::new();
    for source_asset in source_assets {
        let Some(candidate) = map_quicklook_candidate(source_asset)? else {
            continue;
        };
        if let Some(existing) = candidates.get(&candidate.asset_id) {
            if existing != &candidate {
                return Err(error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 快视图证据冲突",
                    "quicklook.duplicate",
                ));
            }
            continue;
        }
        candidates.insert(candidate.asset_id.clone(), candidate);
    }
    let Some(candidate) = candidates.into_values().min_by(|left, right| {
        (left.name_rank, left.media_rank, &left.asset_id).cmp(&(
            right.name_rank,
            right.media_rank,
            &right.asset_id,
        ))
    }) else {
        return Ok(None);
    };
    Ok(Some(CdseQuicklookDescriptor {
        asset_id: candidate.asset_id,
        media_type: candidate.media_type,
        expected_length: candidate.size_bytes,
    }))
}

fn attributes_named<'a>(attributes: &'a [ODataAttribute], name: &str) -> Vec<&'a Value> {
    attributes
        .iter()
        .filter_map(|attribute| {
            (attribute.name.as_deref() == Some(name)).then_some(&attribute.value)
        })
        .collect()
}

fn require_product_type(
    attributes: &[ODataAttribute],
    facts: &Sentinel2SearchFacts,
) -> ProviderResult<String> {
    match attributes_named(attributes, "productType").as_slice() {
        [Value::String(value)] if facts.product_types.iter().any(|expected| expected == value) => {
            Ok(value.clone())
        }
        _ => Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 产品类型无效",
            "product_type",
        )),
    }
}

fn cloud_cover(
    attributes: &[ODataAttribute],
    facts: &Sentinel2SearchFacts,
) -> ProviderResult<Option<f64>> {
    let values = attributes_named(attributes, "cloudCover");
    let cloud_cover = match values.as_slice() {
        [] => None,
        [value] => Some(
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| {
                    error(
                        "CDSE_ODATA_ITEM_INVALID",
                        "Sentinel-2 云量字段无效",
                        "cloud_cover",
                    )
                })?,
        ),
        _ => {
            return Err(error(
                "CDSE_ODATA_ITEM_INVALID",
                "Sentinel-2 云量字段冲突",
                "cloud_cover",
            ))
        }
    };
    if cloud_cover.is_some_and(|value| !(0.0..=100.0).contains(&value)) {
        return Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 云量字段无效",
            "cloud_cover",
        ));
    }
    if let Some(threshold) = facts.cloud_cover_lte {
        if cloud_cover.is_none_or(|value| value > threshold) {
            return Err(error(
                "CDSE_ODATA_ITEM_INVALID",
                "Sentinel-2 云量不符合查询范围",
                "cloud_cover_scope",
            ));
        }
    }
    Ok(cloud_cover)
}

fn content_dates(
    record: &ODataProduct,
    facts: &Sentinel2SearchFacts,
) -> ProviderResult<(String, Option<String>)> {
    let content_date = record.content_date.as_ref().ok_or_else(|| {
        error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 获取时间缺失",
            "content_date",
        )
    })?;
    let start_text = authoritative_text(content_date.start.as_ref(), "content_date.start")?;
    let start = parse_utc_text(&start_text, "content_date.start")?;
    if let (Some(scope_start), Some(scope_end)) = (facts.start, facts.end) {
        if start < scope_start || start >= scope_end {
            return Err(error(
                "CDSE_ODATA_ITEM_INVALID",
                "Sentinel-2 获取时间不符合查询范围",
                "content_date_scope",
            ));
        }
    }
    let end = match content_date.end.as_ref() {
        None => None,
        Some(value) => {
            let value = authoritative_text(Some(value), "content_date.end")?;
            let end = parse_utc_text(&value, "content_date.end")?;
            if end < start {
                return Err(error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 获取结束时间无效",
                    "content_date.end",
                ));
            }
            Some(normalize_utc(end))
        }
    };
    Ok((normalize_utc(start), end))
}

fn polygon_from_wkt(value: &str) -> ProviderResult<Map<String, Value>> {
    let value = value.trim();
    let body = value
        .strip_prefix("POLYGON((")
        .and_then(|value| value.strip_suffix("))"))
        .ok_or_else(|| error("CDSE_ODATA_ITEM_INVALID", "Sentinel-2 WKT 足迹无效", "wkt"))?;
    let mut positions = Vec::new();
    for position in body.split(',') {
        let values = position.split_whitespace().collect::<Vec<_>>();
        if values.len() != 2 {
            return Err(error(
                "CDSE_ODATA_ITEM_INVALID",
                "Sentinel-2 WKT 足迹无效",
                "wkt_position",
            ));
        }
        let longitude = values[0]
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && (-180.0..=180.0).contains(value))
            .ok_or_else(|| {
                error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 WKT 足迹无效",
                    "wkt_longitude",
                )
            })?;
        let latitude = values[1]
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && (-90.0..=90.0).contains(value))
            .ok_or_else(|| {
                error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 WKT 足迹无效",
                    "wkt_latitude",
                )
            })?;
        positions.push(json!([longitude, latitude]));
    }
    if positions.len() < 4 || positions.first() != positions.last() {
        return Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 WKT 足迹无效",
            "wkt_ring",
        ));
    }
    Ok(Map::from_iter([
        ("type".to_string(), Value::String("Polygon".to_string())),
        (
            "coordinates".to_string(),
            Value::Array(vec![Value::Array(positions)]),
        ),
    ]))
}

fn footprint(value: Option<&Value>) -> ProviderResult<Option<Map<String, Value>>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(value)) => Ok(Some(value.clone())),
        Some(Value::String(value)) => polygon_from_wkt(value).map(Some),
        _ => Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 足迹无效",
            "geo_footprint",
        )),
    }
}

fn approved_properties(
    record: &ODataProduct,
    matched_product_type: &str,
    cloud_cover: Option<f64>,
    content_date_end: Option<String>,
) -> ProviderResult<Map<String, Value>> {
    let mut properties = Map::from_iter([
        (
            "product_type".to_string(),
            Value::String(matched_product_type.to_string()),
        ),
        (
            "online".to_string(),
            Value::Bool(record.online.ok_or_else(|| {
                error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 在线状态缺失",
                    "online",
                )
            })?),
        ),
    ]);
    if let Some(end) = content_date_end {
        properties.insert("content_date_end".to_string(), Value::String(end));
    }
    if let Some(path) = record
        .s3_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        properties.insert("s3_path".to_string(), Value::String(path.to_string()));
    }
    if let Some(cloud_cover) = cloud_cover {
        properties.insert("cloud_cover".to_string(), json!(cloud_cover));
    }
    if !record.checksums.is_empty() {
        let mut checksums = BTreeMap::new();
        for checksum in &record.checksums {
            let Some((algorithm, expected_length)) =
                checksum
                    .algorithm
                    .as_deref()
                    .and_then(|algorithm| match algorithm {
                        "MD5" => Some(("MD5", 32)),
                        "SHA256" => Some(("SHA256", 64)),
                        // CDSE may publish supplementary algorithms such as BLAKE3.
                        // They are not frozen until the resolver supports them.
                        _ => None,
                    })
            else {
                continue;
            };
            let value = authoritative_text(checksum.value.as_ref(), "checksum.value")?;
            if value.len() != expected_length || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 校验和值无效",
                    "checksum.value",
                ));
            }
            match checksums.insert(algorithm.to_string(), value.clone()) {
                Some(previous) if previous != value => {
                    return Err(error(
                        "CDSE_ODATA_ITEM_INVALID",
                        "Sentinel-2 校验和声明冲突",
                        "checksum_conflict",
                    ))
                }
                _ => {}
            }
        }
        if !checksums.is_empty() {
            properties.insert(
                "checksums".to_string(),
                Value::Array(
                    checksums
                        .into_iter()
                        .map(|(algorithm, value)| json!({"algorithm": algorithm, "value": value}))
                        .collect(),
                ),
            );
        }
    }
    Ok(properties)
}

pub(super) fn map_record(
    record: &ODataProduct,
    facts: &Sentinel2SearchFacts,
) -> ProviderResult<ProviderProduct> {
    let id = authoritative_text(record.id.as_ref(), "id")?;
    if !canonical_uuid(&id) {
        return Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 OData 身份无效",
            "uuid",
        ));
    }
    let name = authoritative_text(record.name.as_ref(), "name")?;
    if !safe_cdse_odata_name(&name) {
        return Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 OData 产品名称无效",
            "name",
        ));
    }
    let descriptor = facts.collection_descriptor();
    let expected_collection = descriptor.native_collection_id.as_str();
    let collection_name = record
        .collection
        .as_ref()
        .and_then(|c| c.name.as_deref())
        .unwrap_or(expected_collection);
    let collection_echo_is_valid = match record.collection.as_ref() {
        Some(collection) => collection.name.as_deref() == Some(expected_collection),
        None => facts.permits_missing_collection_echo(),
    };
    if !collection_echo_is_valid {
        return Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel 集合无效",
            "collection_echo",
        ));
    }
    if !facts.spacecraft.is_empty() && !facts.spacecraft.iter().any(|p| name.starts_with(p.prefix))
    {
        return Err(error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel 返回的平台与筛选条件不一致",
            "spacecraft_scope",
        ));
    }
    let matched_product_type = require_product_type(&record.attributes, facts)?;
    let cloud_cover = cloud_cover(&record.attributes, facts)?;
    let (acquisition_datetime, content_date_end) = content_dates(record, facts)?;
    let product_key = provider_product_key(PROVIDER_ID, collection_name, &id).map_err(|_| {
        error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel 产品身份无效",
            "product_key",
        )
    })?;
    let asset_key = "archive".to_string();
    let stable_asset_key = stable_asset_key(&product_key, &asset_key).map_err(|_| {
        error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 资产身份无效",
            "asset_key",
        )
    })?;
    let size_bytes = record
        .content_length
        .map(|value| {
            i64::try_from(value).map_err(|_| {
                error(
                    "CDSE_ODATA_ITEM_INVALID",
                    "Sentinel-2 文件大小无效",
                    "content_length",
                )
            })
        })
        .transpose()?;
    let assets = vec![ProviderAsset {
        asset_key: Some(asset_key),
        stable_asset_key: Some(stable_asset_key),
        access_descriptor: None,
        role: AssetRole::Primary,
        native_roles: vec!["archive".to_string()],
        url: None,
        file_name: None,
        media_type: None,
        size_bytes,
    }];
    let product = ProviderProduct {
        provider_id: PROVIDER_ID.to_string(),
        product_id: id,
        display_name: name,
        source_kind: DataSourceKind::Sentinel2,
        product_kind: ProductKind::OpticalScene,
        acquisition_datetime: Some(acquisition_datetime),
        footprint_geojson: footprint(record.geo_footprint.as_ref())?,
        footprint_bbox: None,
        assets,
        properties: approved_properties(
            record,
            &matched_product_type,
            cloud_cover,
            content_date_end,
        )?,
        payload: Map::new(),
    };
    product.validate().map_err(|_| {
        error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 产品记录无效",
            "normalized_product",
        )
    })?;
    Ok(product)
}

pub(crate) fn map_odata_record_value(
    record: &Value,
    facts: &Sentinel2SearchFacts,
) -> ProviderResult<ProviderProduct> {
    let parsed = serde_json::from_value::<ODataProduct>(record.clone()).map_err(|_| {
        error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 OData 单条记录格式无效",
            "record_shape",
        )
    })?;
    map_record(&parsed, facts)
}

pub(crate) fn sentinel2_collection_descriptor(
    facts: &Sentinel2SearchFacts,
) -> insar_core::CatalogCollectionDescriptor {
    sentinel2_collection(facts)
}

pub fn map_odata_records(
    records: &[Value],
    facts: &Sentinel2SearchFacts,
    matched_count: Option<u64>,
    next_cursor: Option<String>,
    request_count: u64,
    remote_exhausted: bool,
) -> ProviderResult<CatalogSearchResponse> {
    let fetched_record_count = records.len() as u64;
    let mut groups: Vec<Vec<ProviderProduct>> = Vec::new();
    let mut indices = HashMap::<String, usize>::new();
    let mut item_errors = Vec::new();
    for record in records {
        let record = serde_json::from_value::<ODataProduct>(record.clone()).map_err(|_| {
            error(
                "CDSE_ODATA_ITEM_INVALID",
                "Sentinel-2 OData 单条记录格式无效",
                "record_shape",
            )
        });
        match record.and_then(|record| map_record(&record, facts)) {
            Ok(product) => {
                let index = match indices.get(&product.product_id) {
                    Some(index) => *index,
                    None => {
                        let index = groups.len();
                        indices.insert(product.product_id.clone(), index);
                        groups.push(Vec::new());
                        index
                    }
                };
                groups[index].push(product);
            }
            Err(error) => item_errors.push(error),
        }
    }
    let mut products = Vec::new();
    let mut mapped_record_count = 0_u64;
    let mut exact_duplicate_record_count = 0_u64;
    for group in groups {
        if group.iter().skip(1).all(|candidate| candidate == &group[0]) {
            mapped_record_count += group.len() as u64;
            exact_duplicate_record_count += group.len().saturating_sub(1) as u64;
            products.push(group.into_iter().next().expect("non-empty group"));
        } else {
            item_errors.extend((0..group.len()).map(|_| {
                error(
                    "CDSE_ODATA_UUID_CONFLICT",
                    "Sentinel-2 OData UUID 存在冲突记录",
                    "conflicting_authoritative_fields",
                )
            }));
        }
    }
    let exact_matched_count = products.len() as u64;
    let working_set_satisfied = facts
        .result_limit
        .map(|target| exact_matched_count >= target || remote_exhausted)
        .unwrap_or(remote_exhausted);
    if !working_set_satisfied {
        return Err(error(
            "CDSE_ODATA_WORKING_SET_INCOMPLETE",
            "Sentinel-2 OData 未完成请求结果集",
            "working_set",
        ));
    }
    if let Some(target) = facts.result_limit {
        products.truncate(target as usize);
    }
    let returned_count = products.len() as u64;
    let response = CatalogSearchResponse {
        collection: sentinel2_collection(facts),
        stats: CatalogSearchStats {
            requested_limit: facts.result_limit,
            matched_count,
            request_count,
            transport_part_count: 1,
            fetched_record_count,
            mapped_record_count,
            profile_filtered_record_count: 0,
            exact_duplicate_record_count,
            unique_provider_product_count: exact_matched_count,
            exact_matched_count,
            returned_count,
            remote_exhausted,
            working_set_satisfied,
            elapsed_ms: 0,
            truncated: next_cursor.is_some(),
        },
        products,
        next_cursor,
        item_errors,
    };
    response.validate().map_err(|_| {
        error(
            "CDSE_ODATA_PAGE_INVALID",
            "Sentinel-2 OData 页面统计无效",
            "response",
        )
    })?;
    Ok(response)
}

pub fn map_odata_page(
    page: ODataPage,
    facts: &Sentinel2SearchFacts,
    next_cursor: Option<String>,
) -> ProviderResult<CatalogSearchResponse> {
    let remote_exhausted = next_cursor.is_none();
    map_odata_records(
        &page.value,
        facts,
        page.matched_count,
        next_cursor,
        1,
        remote_exhausted,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::RouteUse;
    use crate::cdse::query::decode_sentinel2_search_facts_for_use;
    use insar_core::CatalogSearchQuery;

    fn aoi_polygon() -> Map<String, Value> {
        json!({
            "type": "Polygon",
            "coordinates": [[[116.2, 39.8], [116.6, 39.8], [116.6, 40.1], [116.2, 40.1], [116.2, 39.8]]]
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn maps_sentinel2_l1c_and_multi_type_sentinel3_records_without_collection_echo() {
        let s2_l1c_query = CatalogSearchQuery {
            logical_dataset: "optical.sentinel-2.l1c".to_string(),
            provider_id: "copernicus.dataspace.odata".to_string(),
            native_collection_id: "SENTINEL-2".to_string(),
            aoi_geojson: Some(aoi_polygon()),
            result_limit: Some(10),
            cursor: None,
            filters: json!({
                "product_type": "S2MSI1C",
                "datetime": {
                    "start": "2024-06-01T00:00:00Z",
                    "end": "2024-06-05T00:00:00Z"
                }
            })
            .as_object()
            .unwrap()
            .clone(),
        };
        let s2_facts =
            decode_sentinel2_search_facts_for_use(&s2_l1c_query, RouteUse::Production).unwrap();
        let s2_record = json!({
            "Id": "e3183efa-2174-4df2-9757-5b7e04916753",
            "Name": "S2B_MSIL1C_20240602T025529_N0510_R032_T50TMK_20240602T044702.SAFE",
            "Online": true,
            "ContentLength": 521297168_i64,
            "ContentDate": {
                "Start": "2024-06-02T02:55:29.024000Z",
                "End": "2024-06-02T02:55:29.024000Z"
            },
            "Checksum": [
                {"Algorithm": "MD5", "Value": "75f1cc48edef845e87c4dc86d415c396"},
                {"Algorithm": "BLAKE3", "Value": "609e38fe2e0e72613cd86e54a7e882c03444e96e3320d6a15ffd25bd6b630f84"}
            ],
            "Attributes": [
                {"Name": "productType", "Value": "S2MSI1C"},
                {"Name": "cloudCover", "Value": 12.5}
            ]
        });
        let s2_resp = map_odata_records(&[s2_record], &s2_facts, Some(1), None, 1, true)
            .expect("S2 L1C maps");
        assert_eq!(s2_resp.products.len(), 1);
        assert_eq!(
            s2_resp.products[0].properties.get("product_type"),
            Some(&Value::String("S2MSI1C".to_string()))
        );

        let s3_query = CatalogSearchQuery {
            logical_dataset: "optical.sentinel-3.standard".to_string(),
            provider_id: "copernicus.dataspace.odata".to_string(),
            native_collection_id: "SENTINEL-3".to_string(),
            aoi_geojson: Some(aoi_polygon()),
            result_limit: Some(10),
            cursor: None,
            filters: json!({
                "product_type": ["OL_2_LFR___", "SL_2_LST___", "SR_2_LAN___"],
                "datetime": {
                    "start": "2024-06-01T00:00:00Z",
                    "end": "2024-06-05T00:00:00Z"
                }
            })
            .as_object()
            .unwrap()
            .clone(),
        };
        let s3_facts =
            decode_sentinel2_search_facts_for_use(&s3_query, RouteUse::Production).unwrap();
        let s3_records = vec![
            json!({
                "Id": "e1ca9111-7d47-4c92-a490-c883db33fe12",
                "Name": "S3B_OL_2_LFR____20240602T020447_20240602T020747_20250705T231500_0179_093_331_2340_ESA_R_NT_003.SEN3",
                "Online": true,
                "ContentLength": 122160176_i64,
                "ContentDate": {
                    "Start": "2024-06-02T02:04:47.000000Z",
                    "End": "2024-06-02T02:07:47.000000Z"
                },
                "Checksum": [
                    {"Algorithm": "MD5", "Value": "994acb41c0c33233c6ea90e0851b52b5"}
                ],
                "Attributes": [
                    {"Name": "productType", "Value": "OL_2_LFR___"},
                    {"Name": "cloudCover", "Value": 64.0}
                ]
            }),
            json!({
                "Id": "babded02-b1fb-4df0-8fbb-021a5dd63cdf",
                "Name": "S3A_SL_2_LST____20240602T140034_20240602T140334_20240604T002708_0179_113_096_0540_PS1_O_NT_004.SEN3",
                "Online": true,
                "ContentLength": 70862361_i64,
                "ContentDate": {
                    "Start": "2024-06-02T14:00:34.000000Z",
                    "End": "2024-06-02T14:03:34.000000Z"
                },
                "Checksum": [
                    {"Algorithm": "MD5", "Value": "761a43047bb38656c322e12030d16c4b"}
                ],
                "Attributes": [
                    {"Name": "productType", "Value": "SL_2_LST___"},
                    {"Name": "cloudCover", "Value": 47.5}
                ]
            }),
        ];
        let s3_resp = map_odata_records(&s3_records, &s3_facts, Some(2), None, 1, true)
            .expect("Sentinel-3 multi-productType maps");
        assert_eq!(s3_resp.products.len(), 2);
        assert_eq!(
            s3_resp.products[0].properties.get("product_type"),
            Some(&Value::String("OL_2_LFR___".to_string()))
        );
        assert_eq!(
            s3_resp.products[1].properties.get("product_type"),
            Some(&Value::String("SL_2_LST___".to_string()))
        );
        assert!(s3_resp.products[0].assets[0]
            .stable_asset_key
            .as_deref()
            .unwrap()
            .contains(":SENTINEL-3:"));
    }
}
