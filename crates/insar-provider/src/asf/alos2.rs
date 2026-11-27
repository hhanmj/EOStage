use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, SecondsFormat, Utc};
use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetResolverParameters, AssetRole, ContractError, DataSourceKind, ProductKind, ProviderAsset,
    ProviderProduct, ProviderResult,
};
use serde_json::{json, Map, Value};
use url::Url;

use crate::catalog::{
    alos2_expected_polarizations, CollectionProfile, ALOS2_COLLECTION_SHORT_NAME,
    ALOS2_COLLECTION_VERSION, ALOS2_FILENAME_RULE_ID, ALOS2_IDENTITY_RULE_ID,
    ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROCESSING_LEVELS, ALOS2_PROFILE_ID,
    ALOS2_PROVIDER_ID, ALOS2_RESOLVER_POLICY_ID,
};

use super::umm::CmrGranuleEnvelope;

const MAX_TEXT_BYTES: usize = 512;
const MAX_ALOS2_ASSET_RECORDS: usize = 16;
const MAX_ALOS2_ROUTES: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct Alos2AssetGraph {
    pub assets: Vec<ProviderAsset>,
    pub integrity: Map<String, Value>,
    pub routes: Map<String, Value>,
}

pub fn map_alos2_umm_granule(
    envelope: &CmrGranuleEnvelope,
    profile: &CollectionProfile,
) -> ProviderResult<ProviderProduct> {
    validate_profile(profile)?;
    validate_collection(&envelope.collection)?;

    let granule_id = required_text(
        &envelope.granule,
        "concept_id",
        "ASF_PRODUCT_IDENTITY_MISSING",
        "granule_concept_id",
    )?;
    if !valid_cmr_granule_concept_id(&granule_id) {
        return Err(error(
            "ASF_PRODUCT_IDENTITY_MISSING",
            "ALOS-2 Granule 缺少有效的 CMR 身份",
            "granule_concept_id",
        ));
    }
    let native_id = required_text(
        &envelope.granule,
        "native_id",
        "ASF_PRODUCT_IDENTITY_MISSING",
        "native_id",
    )?;
    let producer_granule_id = required_text(
        &envelope.granule,
        "producer_granule_id",
        "ASF_PRODUCT_IDENTITY_MISSING",
        "producer_granule_id",
    )?;
    let platform = required_scope_text(&envelope.granule, "platform")?;
    let instrument = required_scope_text(&envelope.granule, "instrument")?;
    let processing_level = required_scope_text(&envelope.granule, "processing_level")?;
    let beam_mode = required_scope_text(&envelope.granule, "beam_mode")?;
    if platform != "ALOS-2"
        || instrument != "PALSAR-2"
        || !ALOS2_PROCESSING_LEVELS.contains(&processing_level.as_str())
    {
        return Err(scope_error("mission_or_level"));
    }
    let polarizations = polarizations(&envelope.granule)?;
    if alos2_expected_polarizations(&beam_mode) != Some(polarizations.as_slice()) {
        return Err(scope_error("beam_polarization"));
    }

    let acquisition_start = acquisition_time(&envelope.granule, "acquisition_start_utc")?;
    let acquisition_end = acquisition_time(&envelope.granule, "acquisition_end_utc")?;
    if acquisition_end < acquisition_start {
        return Err(scope_error("acquisition_interval"));
    }
    let acquisition_start = acquisition_start.to_rfc3339_opts(SecondsFormat::AutoSi, true);
    let acquisition_end = acquisition_end.to_rfc3339_opts(SecondsFormat::AutoSi, true);
    let footprint = envelope
        .granule
        .get("footprint")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| scope_error("footprint"))?;

    let product_key =
        provider_product_key(ALOS2_PROVIDER_ID, ALOS2_NATIVE_COLLECTION_ID, &granule_id).map_err(
            |_| {
                error(
                    "ASF_PRODUCT_IDENTITY_MISSING",
                    "ALOS-2 Granule 身份无法规范化",
                    "provider_product_key",
                )
            },
        )?;
    let graph = classify_alos2_assets(
        envelope.granule.get("assets").unwrap_or(&Value::Null),
        &product_key,
    )?;
    let preview_available = match envelope.granule.get("browse") {
        None | Some(Value::Null) => false,
        Some(_) => {
            return Err(error(
                "ASF_ASSET_CLASSIFICATION_INVALID",
                "ALOS-2 Browse 证据尚未通过当前分类规则",
                "browse",
            ))
        }
    };

    let mut properties = Map::from_iter([
        (
            "provider_product_key".to_owned(),
            Value::String(product_key),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(granule_id.clone()),
        ),
        (
            "cmr_granule_concept_id".to_owned(),
            Value::String(granule_id.clone()),
        ),
        ("cmr_native_id".to_owned(), Value::String(native_id.clone())),
        (
            "producer_granule_id".to_owned(),
            Value::String(producer_granule_id),
        ),
        (
            "logical_dataset".to_owned(),
            Value::String(ALOS2_LOGICAL_DATASET.to_owned()),
        ),
        (
            "collection_concept_id".to_owned(),
            Value::String(ALOS2_NATIVE_COLLECTION_ID.to_owned()),
        ),
        (
            "collection_short_name".to_owned(),
            Value::String(ALOS2_COLLECTION_SHORT_NAME.to_owned()),
        ),
        (
            "collection_version".to_owned(),
            Value::String(ALOS2_COLLECTION_VERSION.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(ALOS2_PROFILE_ID.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(ALOS2_IDENTITY_RULE_ID.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(ALOS2_FILENAME_RULE_ID.to_owned()),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(ALOS2_RESOLVER_POLICY_ID.to_owned()),
        ),
        ("platform".to_owned(), Value::String(platform)),
        ("instrument".to_owned(), Value::String(instrument)),
        (
            "processing_level".to_owned(),
            Value::String(processing_level),
        ),
        ("beam_mode".to_owned(), Value::String(beam_mode)),
        ("polarizations".to_owned(), json!(polarizations)),
        (
            "acquisition_end_utc".to_owned(),
            Value::String(acquisition_end),
        ),
        ("asset_integrity".to_owned(), Value::Object(graph.integrity)),
        ("asset_routes".to_owned(), Value::Object(graph.routes)),
        (
            "preview_available".to_owned(),
            Value::Bool(preview_available),
        ),
    ]);
    for key in [
        "operational_mode",
        "flight_direction",
        "look_direction",
        "off_nadir_angle",
        "orbit",
        "path",
        "frame",
    ] {
        if let Some(value) = envelope.granule.get(key).filter(|value| !value.is_null()) {
            properties.insert(key.to_owned(), value.clone());
        }
    }

    let product = ProviderProduct {
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        product_id: granule_id,
        display_name: native_id,
        source_kind: DataSourceKind::Sar,
        product_kind: ProductKind::SarProduct,
        acquisition_datetime: Some(acquisition_start),
        footprint_geojson: Some(footprint),
        footprint_bbox: None,
        assets: graph.assets,
        properties,
        payload: Map::new(),
    };
    product.validate().map_err(|_| {
        error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "ALOS-2 产品合同验证失败",
            "provider_product",
        )
    })?;
    Ok(product)
}

pub fn classify_alos2_assets(
    value: &Value,
    provider_product_key: &str,
) -> ProviderResult<Alos2AssetGraph> {
    let records = value.as_array().ok_or_else(|| {
        error(
            "ASF_PRIMARY_ASSET_MISSING",
            "ALOS-2 Granule 缺少主 ZIP",
            "assets",
        )
    })?;
    if records.len() > MAX_ALOS2_ASSET_RECORDS {
        return Err(asset_error("asset_count"));
    }
    let mut primary = BTreeMap::<String, Vec<&Map<String, Value>>>::new();
    for record in records {
        let record = record
            .as_object()
            .ok_or_else(|| asset_error("asset_record"))?;
        match record.get("native_role").and_then(Value::as_str) {
            Some("archive") => {}
            _ => return Err(asset_error("native_role")),
        }
        let asset_id = required_asset_text(record, "asset_id")?;
        let file_name = required_asset_text(record, "file_name")?;
        if asset_id != file_name || !safe_zip_basename(&file_name) {
            return Err(asset_error("file_name"));
        }
        primary.entry(asset_id).or_default().push(record);
    }
    let (asset_id, records) = match primary.len() {
        0 => {
            return Err(error(
                "ASF_PRIMARY_ASSET_MISSING",
                "ALOS-2 Granule 缺少主 ZIP",
                "primary_asset",
            ))
        }
        1 => primary.into_iter().next().expect("one primary identity"),
        _ => {
            return Err(error(
                "ASF_PRIMARY_ASSET_AMBIGUOUS",
                "ALOS-2 Granule 包含多个主 ZIP 身份",
                "primary_asset",
            ))
        }
    };

    let first = records[0];
    let media_type = required_asset_text(first, "media_type")?;
    if media_type != "application/zip" {
        return Err(asset_error("media_type"));
    }
    let size_bytes = first
        .get("size_bytes")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0 && *value <= i64::MAX as u64)
        .ok_or_else(|| asset_error("size_bytes"))?;
    let (algorithm, checksum_value) = checksum(first.get("checksum"))?;
    let mut merged_routes = BTreeMap::new();
    for record in records {
        if required_asset_text(record, "asset_id")? != asset_id
            || required_asset_text(record, "file_name")? != asset_id
            || required_asset_text(record, "media_type")? != media_type
            || record.get("size_bytes").and_then(Value::as_u64) != Some(size_bytes)
            || checksum(record.get("checksum"))? != (algorithm.clone(), checksum_value.clone())
        {
            return Err(asset_error("alternate_route_conflict"));
        }
        for route in routes(record.get("routes"))? {
            merged_routes.insert(route.clone(), route);
        }
    }
    let routes = merged_routes.into_values().collect::<Vec<_>>();
    let file_name = asset_id.clone();
    let https = routes
        .iter()
        .find(|(scheme, _)| scheme == "https")
        .map(|(_, locator)| locator.clone())
        .ok_or_else(|| asset_error("https_route"))?;
    let stable_key = stable_asset_key(provider_product_key, &asset_id)
        .map_err(|_| asset_error("stable_asset_key"))?;
    let descriptor = AssetAccessDescriptor {
        resolver_id: "earthdata_https".to_owned(),
        scheme: AssetAccessScheme::EarthdataHttps,
        stable_href: Some(https),
        requires_credentials: true,
        credential_profile: Some("earthdata-default".to_owned()),
        resolver_parameters: AssetResolverParameters {},
    };
    descriptor
        .validate()
        .map_err(|_| asset_error("access_descriptor"))?;

    let integrity = Map::from_iter([(
        asset_id.clone(),
        json!({
            "file_name": file_name,
            "size_bytes": size_bytes,
            "algorithm": algorithm,
            "value": checksum_value,
        }),
    )]);
    let routes = Map::from_iter([(
        asset_id.clone(),
        Value::Array(
            routes
                .into_iter()
                .map(|(scheme, locator)| json!({"scheme": scheme, "locator": locator}))
                .collect(),
        ),
    )]);
    Ok(Alos2AssetGraph {
        assets: vec![ProviderAsset {
            asset_key: Some(asset_id),
            stable_asset_key: Some(stable_key),
            access_descriptor: Some(descriptor),
            role: AssetRole::Primary,
            native_roles: vec!["archive".to_owned()],
            url: None,
            file_name: Some(file_name),
            media_type: Some(media_type),
            size_bytes: Some(size_bytes as i64),
        }],
        integrity,
        routes,
    })
}

fn validate_profile(profile: &CollectionProfile) -> ProviderResult<()> {
    if profile.profile_id != ALOS2_PROFILE_ID
        || profile.logical_dataset != ALOS2_LOGICAL_DATASET
        || profile.provider_id != ALOS2_PROVIDER_ID
        || profile.native_collection_id != ALOS2_NATIVE_COLLECTION_ID
        || profile.required_product_type != ALOS2_COLLECTION_SHORT_NAME
        || profile.identity_rule_id != ALOS2_IDENTITY_RULE_ID
        || profile.filename_rule_id != ALOS2_FILENAME_RULE_ID
        || profile.resolver_policy_id != ALOS2_RESOLVER_POLICY_ID
    {
        return Err(collection_error("profile"));
    }
    Ok(())
}

fn validate_collection(collection: &Map<String, Value>) -> ProviderResult<()> {
    let matches = collection.get("concept_id").and_then(Value::as_str)
        == Some(ALOS2_NATIVE_COLLECTION_ID)
        && collection.get("short_name").and_then(Value::as_str)
            == Some(ALOS2_COLLECTION_SHORT_NAME)
        && collection.get("version").and_then(Value::as_str) == Some(ALOS2_COLLECTION_VERSION);
    if matches {
        Ok(())
    } else {
        Err(collection_error("collection"))
    }
}

fn required_scope_text(map: &Map<String, Value>, key: &'static str) -> ProviderResult<String> {
    required_text(map, key, "ASF_PRODUCT_SCOPE_MISMATCH", key)
}

fn required_text(
    map: &Map<String, Value>,
    key: &'static str,
    code: &'static str,
    reason: &'static str,
) -> ProviderResult<String> {
    map.get(key)
        .and_then(Value::as_str)
        .filter(|value| valid_text(value))
        .map(str::to_owned)
        .ok_or_else(|| error(code, "ALOS-2 权威元数据缺失或无效", reason))
}

fn required_asset_text(map: &Map<String, Value>, key: &'static str) -> ProviderResult<String> {
    map.get(key)
        .and_then(Value::as_str)
        .filter(|value| valid_text(value))
        .map(str::to_owned)
        .ok_or_else(|| asset_error(key))
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.len() <= MAX_TEXT_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_cmr_granule_concept_id(value: &str) -> bool {
    value
        .strip_prefix('G')
        .and_then(|value| value.strip_suffix("-ASF"))
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn polarizations(granule: &Map<String, Value>) -> ProviderResult<Vec<&str>> {
    let values = granule
        .get("polarizations")
        .and_then(Value::as_array)
        .ok_or_else(|| scope_error("polarizations"))?;
    let mut unique = BTreeSet::new();
    for value in values {
        let value = value
            .as_str()
            .filter(|value| valid_text(value))
            .ok_or_else(|| scope_error("polarizations"))?;
        if !unique.insert(value) {
            return Err(scope_error("polarizations"));
        }
    }
    Ok(unique.into_iter().collect())
}

fn acquisition_time(
    granule: &Map<String, Value>,
    key: &'static str,
) -> ProviderResult<DateTime<Utc>> {
    let value = required_scope_text(granule, key)?;
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| scope_error(key))
}

fn safe_zip_basename(value: &str) -> bool {
    valid_text(value)
        && !value.contains(['/', '\\', '\0'])
        && value.to_ascii_lowercase().ends_with(".zip")
        && value != "."
        && value != ".."
}

fn checksum(value: Option<&Value>) -> ProviderResult<(String, String)> {
    let value = value
        .and_then(Value::as_object)
        .ok_or_else(|| asset_error("checksum"))?;
    let algorithm = required_asset_text(value, "algorithm")?;
    let checksum = required_asset_text(value, "value")?.to_ascii_lowercase();
    let expected = match algorithm.as_str() {
        "MD5" => 32,
        "SHA256" => 64,
        _ => return Err(asset_error("checksum_algorithm")),
    };
    if checksum.len() != expected || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(asset_error("checksum_value"));
    }
    Ok((algorithm, checksum))
}

fn routes(value: Option<&Value>) -> ProviderResult<Vec<(String, String)>> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| asset_error("routes"))?;
    if values.is_empty() || values.len() > MAX_ALOS2_ROUTES {
        return Err(asset_error("routes"));
    }
    let mut routes = BTreeMap::new();
    for value in values {
        let value = value.as_object().ok_or_else(|| asset_error("route"))?;
        let scheme = required_asset_text(value, "scheme")?;
        let locator = required_asset_text(value, "locator")?;
        validate_route(&scheme, &locator)?;
        routes.insert((scheme.clone(), locator.clone()), (scheme, locator));
    }
    Ok(routes.into_values().collect())
}

fn validate_route(scheme: &str, locator: &str) -> ProviderResult<()> {
    let parsed = Url::parse(locator).map_err(|_| asset_error("route_locator"))?;
    let common_safe = parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none();
    let valid = match scheme {
        "https" => {
            common_safe && parsed.scheme() == "https" && parsed.port_or_known_default() == Some(443)
        }
        "s3" => common_safe && parsed.scheme() == "s3" && parsed.port().is_none(),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(asset_error("route_locator"))
    }
}

fn collection_error(reason: &'static str) -> ContractError {
    error(
        "ASF_COLLECTION_IDENTITY_MISMATCH",
        "ALOS-2 Collection 身份与冻结 Profile 不一致",
        reason,
    )
}

fn scope_error(reason: &'static str) -> ContractError {
    error(
        "ASF_PRODUCT_SCOPE_MISMATCH",
        "ALOS-2 产品不属于 ScanSAR Level 1.1 支持范围",
        reason,
    )
}

fn asset_error(reason: &'static str) -> ContractError {
    error(
        "ASF_ASSET_CLASSIFICATION_INVALID",
        "ALOS-2 ZIP 资产证据无效",
        reason,
    )
}

fn error(code: &'static str, message: &'static str, reason: &'static str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded ALOS-2 error is safe")
}
