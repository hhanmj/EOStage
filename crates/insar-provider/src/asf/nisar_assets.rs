use std::collections::{BTreeMap, BTreeSet};

use insar_core::{
    stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters, AssetRole,
    ContractError, ProviderAsset, ProviderResult,
};
use serde_json::{json, Map, Value};
use url::Url;

pub const NISAR_MAX_RAW_RESOURCE_ENTRIES: usize = 128;
pub const NISAR_MAX_DEDUPLICATED_ASSETS: usize = 32;
pub const NISAR_MAX_NATIVE_ROLES: usize = 8;
pub const NISAR_MAX_NATIVE_ROLE_BYTES: usize = 128;
pub const NISAR_MAX_FILE_NAME_BYTES: usize = 512;
pub const NISAR_MAX_MEDIA_TYPE_BYTES: usize = 128;
pub const NISAR_MAX_LOCATOR_BYTES: usize = 4_096;
pub const NISAR_MAX_ASSET_GRAPH_JSON_BYTES: usize = 40_960;

#[derive(Clone, Debug, PartialEq)]
pub struct NisarAssetGraph {
    pub assets: Vec<ProviderAsset>,
    pub integrity: Map<String, Value>,
    pub routes: Map<String, Value>,
    pub metadata_assets: Vec<Value>,
    pub serialized_metadata_bytes: usize,
    pub primary_asset_key: String,
}

#[derive(Clone, Debug)]
struct ParsedAsset {
    asset_id: String,
    file_name: String,
    media_type: String,
    role: AssetRole,
    native_roles: BTreeSet<String>,
    archive_name: Option<String>,
    product_match: bool,
    url_types: BTreeSet<String>,
    url_subtypes: BTreeSet<String>,
    formats: BTreeSet<String>,
    size_bytes: Option<u64>,
    checksum: Option<(String, String)>,
    routes: BTreeMap<(String, String), (String, String)>,
}

pub fn classify_nisar_assets(
    value: &Value,
    provider_product_key: &str,
) -> ProviderResult<NisarAssetGraph> {
    let graph = value
        .as_object()
        .ok_or_else(|| classification_error("asset_graph"))?;
    let raw_resource_count = graph
        .get("raw_resource_count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| classification_error("raw_resource_count"))?;
    if raw_resource_count > NISAR_MAX_RAW_RESOURCE_ENTRIES {
        return Err(limit_error("raw_resource_count"));
    }
    let records = graph
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| classification_error("asset_records"))?;
    if records.len() > NISAR_MAX_RAW_RESOURCE_ENTRIES {
        return Err(limit_error("asset_record_count"));
    }
    if records.is_empty() || records.len() > raw_resource_count {
        return Err(classification_error("raw_resource_count"));
    }

    let mut deduplicated = BTreeMap::<String, ParsedAsset>::new();
    let mut asset_identities = BTreeMap::<String, String>::new();
    let mut represented_raw_resources = 0usize;
    for record in records {
        let raw_route_count = record
            .get("routes")
            .and_then(Value::as_array)
            .map(Vec::len)
            .ok_or_else(|| classification_error("routes"))?;
        let parsed = parse_asset(record)?;
        represented_raw_resources = represented_raw_resources
            .checked_add(usize::from(parsed.archive_name.is_some()))
            .and_then(|value| value.checked_add(raw_route_count))
            .ok_or_else(|| limit_error("represented_raw_resources"))?;
        if represented_raw_resources > NISAR_MAX_RAW_RESOURCE_ENTRIES {
            return Err(limit_error("represented_raw_resources"));
        }
        match asset_identities.entry(parsed.asset_id.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(parsed.file_name.clone());
            }
            std::collections::btree_map::Entry::Occupied(entry)
                if entry.get() != &parsed.file_name =>
            {
                return Err(classification_error("duplicate_asset_id"));
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
        match deduplicated.entry(parsed.file_name.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(parsed);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                merge_asset(entry.get_mut(), parsed)?;
            }
        }
    }
    if raw_resource_count < represented_raw_resources {
        return Err(classification_error("raw_resource_count"));
    }
    for asset in deduplicated.values() {
        validate_role_evidence(asset, true)?;
    }
    if deduplicated.len() > NISAR_MAX_DEDUPLICATED_ASSETS {
        return Err(limit_error("deduplicated_asset_count"));
    }

    let primary_keys = deduplicated
        .values()
        .filter(|asset| asset.role == AssetRole::Primary)
        .map(|asset| asset.asset_id.clone())
        .collect::<Vec<_>>();
    let primary_asset_key = match primary_keys.as_slice() {
        [] => return Err(primary_missing()),
        [primary] => primary.clone(),
        _ => return Err(primary_ambiguous()),
    };

    let mut built = deduplicated
        .into_values()
        .map(|asset| build_asset(provider_product_key, asset))
        .collect::<ProviderResult<Vec<_>>>()?;
    built.sort_by(|left, right| {
        role_rank(&left.asset.role)
            .cmp(&role_rank(&right.asset.role))
            .then_with(|| left.file_name.cmp(&right.file_name))
            .then_with(|| left.media_type.cmp(&right.media_type))
            .then_with(|| left.stable_asset_key.cmp(&right.stable_asset_key))
    });

    let assets = built
        .iter()
        .map(|asset| asset.asset.clone())
        .collect::<Vec<_>>();
    let integrity = Map::from_iter(built.iter().filter_map(|asset| {
        asset
            .integrity
            .clone()
            .map(|value| (asset.asset_id.clone(), value))
    }));
    let routes = Map::from_iter(
        built
            .iter()
            .map(|asset| (asset.asset_id.clone(), Value::Array(asset.routes.clone()))),
    );
    let metadata_assets = built
        .into_iter()
        .map(|asset| asset.metadata)
        .collect::<Vec<_>>();
    let serialized_metadata_bytes = serde_json::to_vec(&metadata_assets)
        .map_err(|_| classification_error("metadata_assets_serialization"))?
        .len();
    if serialized_metadata_bytes > NISAR_MAX_ASSET_GRAPH_JSON_BYTES {
        return Err(limit_error("metadata_assets_json_bytes"));
    }

    Ok(NisarAssetGraph {
        assets,
        integrity,
        routes,
        metadata_assets,
        serialized_metadata_bytes,
        primary_asset_key,
    })
}

fn parse_asset(value: &Value) -> ProviderResult<ParsedAsset> {
    let record = value
        .as_object()
        .ok_or_else(|| classification_error("asset_record"))?;
    let asset_id = bounded_text(record, "asset_id", NISAR_MAX_FILE_NAME_BYTES)?;
    let file_name = bounded_text(record, "file_name", NISAR_MAX_FILE_NAME_BYTES)?;
    if !safe_basename(&file_name) {
        return Err(classification_error("file_name"));
    }
    let media_type = bounded_text(record, "media_type", NISAR_MAX_MEDIA_TYPE_BYTES)?;
    if !valid_media_type(&media_type) {
        return Err(classification_error("media_type"));
    }
    let role = parse_role(record.get("normalized_role"))?;
    let native_roles = bounded_string_set(record, "native_roles")?;
    let evidence = record
        .get("native_evidence")
        .and_then(Value::as_object)
        .ok_or_else(|| classification_error("native_evidence"))?;
    let archive_name = match evidence.get("archive_name") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => {
            let value = bounded_value_text(value, "archive_name", NISAR_MAX_FILE_NAME_BYTES)?;
            if value != file_name {
                return Err(classification_error("archive_identity"));
            }
            Some(value)
        }
        Some(_) => return Err(classification_error("archive_name")),
    };
    let product_match = evidence
        .get("product_match")
        .and_then(Value::as_bool)
        .ok_or_else(|| classification_error("product_match"))?;
    let url_types = bounded_string_set(evidence, "url_types")?;
    let url_subtypes = bounded_string_set(evidence, "url_subtypes")?;
    let formats = bounded_string_set(evidence, "formats")?;
    let size_bytes = match record.get("size_bytes") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .filter(|value| *value > 0 && *value <= i64::MAX as u64)
                .ok_or_else(|| classification_error("size_bytes"))?,
        ),
    };
    let checksum = parse_checksum(record.get("checksum"))?;
    if size_bytes.is_some() != checksum.is_some() {
        return Err(classification_error("asset_integrity"));
    }
    let routes = parse_routes(record.get("routes"), &file_name)?;

    let parsed = ParsedAsset {
        asset_id,
        file_name,
        media_type,
        role,
        native_roles,
        archive_name,
        product_match,
        url_types,
        url_subtypes,
        formats,
        size_bytes,
        checksum,
        routes,
    };
    validate_role_evidence(&parsed, false)?;
    Ok(parsed)
}

fn validate_role_evidence(asset: &ParsedAsset, require_https: bool) -> ProviderResult<()> {
    let lower_name = asset.file_name.to_ascii_lowercase();
    let mut allowed_native_roles = asset.url_types.clone();
    allowed_native_roles.extend(asset.url_subtypes.iter().cloned());
    for format in &asset.formats {
        allowed_native_roles.insert(format!("ARCHIVE:{format}"));
    }
    let has_archive_role = asset.archive_name.is_some()
        && asset
            .formats
            .iter()
            .any(|format| asset.native_roles.contains(&format!("ARCHIVE:{format}")));
    let native_roles_match = !asset.native_roles.is_empty()
        && asset
            .native_roles
            .iter()
            .all(|role| allowed_native_roles.contains(role))
        && if asset.archive_name.is_some() {
            has_archive_role
        } else {
            asset
                .native_roles
                .iter()
                .all(|role| !role.starts_with("ARCHIVE:"))
        };
    let forbidden_scope_evidence = asset
        .native_roles
        .iter()
        .chain(asset.url_types.iter())
        .chain(asset.url_subtypes.iter())
        .any(|value| {
            let value = value.to_ascii_uppercase();
            value.contains("S-BAND")
                || value.contains("S_BAND")
                || value.contains("BHOONIDHI")
                || value.contains("PRIVATE")
        });
    if !native_roles_match || forbidden_scope_evidence {
        return Err(classification_error("native_role_evidence"));
    }
    let is_hdf5 = lower_name.ends_with(".h5")
        && asset.media_type == "application/x-hdf5"
        && has_only_format(asset, &["HDF5"]);
    let is_png = lower_name.ends_with(".png")
        && asset.media_type == "image/png"
        && has_only_format(asset, &["PNG"]);
    let is_jpeg = matches_extension(&lower_name, &[".jpg", ".jpeg"])
        && asset.media_type == "image/jpeg"
        && has_only_format(asset, &["JPEG", "JPG"]);
    let is_xml = lower_name.ends_with(".xml")
        && asset.media_type == "application/xml"
        && has_only_format(asset, &["XML"]);
    let is_yaml = matches_extension(&lower_name, &[".yaml", ".yml"])
        && asset.media_type == "application/yaml"
        && has_only_format(asset, &["YAML", "YML"]);
    let is_csv = lower_name.ends_with(".csv")
        && asset.media_type == "text/csv"
        && has_only_format(asset, &["CSV"]);
    let is_json = lower_name.ends_with(".json")
        && asset.media_type == "application/json"
        && has_only_format(asset, &["JSON"]);
    let is_text = lower_name.ends_with(".txt")
        && asset.media_type == "text/plain"
        && has_only_format(asset, &["TEXT"]);
    let is_zip = lower_name.ends_with(".zip")
        && asset.media_type == "application/zip"
        && has_only_format(asset, &["ZIP"]);
    let is_kml = lower_name.ends_with(".kml")
        && asset.media_type == "application/vnd.google-earth.kml+xml"
        && has_only_format(asset, &["KML"]);
    let is_pdf = lower_name.ends_with(".pdf")
        && asset.media_type == "application/pdf"
        && has_only_format(asset, &["PDF"]);
    let is_binary = lower_name.ends_with(".bin")
        && asset.media_type.starts_with("application/")
        && has_only_format(asset, &["BINARY"]);
    let is_other = asset.media_type == "application/octet-stream"
        && has_only_format(asset, &["OTHER"])
        && !matches_extension(&lower_name, &[".h5", ".hdf5", ".he5"]);
    let is_qa_hdf5 = lower_name.ends_with("_qa_stats.h5") || lower_name.ends_with("_qa.h5");
    let has_data_role = asset
        .url_types
        .iter()
        .any(|value| matches!(value.as_str(), "GET DATA" | "GET DATA VIA DIRECT ACCESS"));
    let only_data_roles = !asset.url_types.is_empty()
        && asset
            .url_types
            .iter()
            .all(|value| matches!(value.as_str(), "GET DATA" | "GET DATA VIA DIRECT ACCESS"));
    let has_visualization = asset
        .url_types
        .iter()
        .any(|value| value == "GET RELATED VISUALIZATION");
    let only_visualization = !asset.url_types.is_empty()
        && asset
            .url_types
            .iter()
            .all(|value| value == "GET RELATED VISUALIZATION");
    let only_extended_metadata = !asset.url_types.is_empty()
        && asset
            .url_types
            .iter()
            .all(|value| value == "EXTENDED METADATA");
    let only_related_information = !asset.url_types.is_empty()
        && asset
            .url_types
            .iter()
            .all(|value| value == "VIEW RELATED INFORMATION");
    let has_quality_role = asset.url_subtypes.iter().any(|value| {
        matches!(
            value.as_str(),
            "PRODUCT QUALITY ASSESSMENT" | "QA STATISTICS"
        )
    });
    let only_quality_roles = !asset.url_subtypes.is_empty()
        && asset.url_subtypes.iter().all(|value| {
            matches!(
                value.as_str(),
                "PRODUCT QUALITY ASSESSMENT" | "QA STATISTICS"
            )
        });
    let has_https = asset.routes.values().any(|(scheme, _)| scheme == "https");

    let valid = match asset.role {
        AssetRole::Primary => {
            is_hdf5
                && !is_qa_hdf5
                && asset.archive_name.is_some()
                && asset.product_match
                && has_data_role
                && only_data_roles
                && asset.url_subtypes.is_empty()
                && asset.size_bytes.is_some()
                && asset.checksum.is_some()
                && (!require_https || has_https)
        }
        AssetRole::Preview | AssetRole::Browse => {
            (is_png || is_jpeg)
                && !lower_name.contains("thumbnail")
                && !asset.product_match
                && has_visualization
                && only_visualization
                && asset.url_subtypes.is_empty()
                && (!require_https || has_https)
        }
        AssetRole::Thumbnail => {
            (is_png || is_jpeg)
                && lower_name.contains("thumbnail")
                && !asset.product_match
                && has_visualization
                && only_visualization
                && asset.url_subtypes.is_empty()
                && (!require_https || has_https)
        }
        AssetRole::Metadata => {
            asset.archive_name.is_some()
                && !asset.product_match
                && !has_data_role
                && (((is_xml || is_yaml || is_json || is_text)
                    && only_extended_metadata
                    && asset.url_subtypes.is_empty())
                    || (is_csv
                        && only_related_information
                        && has_quality_role
                        && only_quality_roles))
        }
        AssetRole::Auxiliary => {
            asset.archive_name.is_some()
                && !asset.product_match
                && !has_data_role
                && ((is_hdf5
                    && is_qa_hdf5
                    && only_related_information
                    && has_quality_role
                    && only_quality_roles)
                    || (is_pdf
                        && only_related_information
                        && has_quality_role
                        && only_quality_roles)
                    || (is_kml && only_extended_metadata && asset.url_subtypes.is_empty())
                    || (is_binary && only_related_information)
                    || (is_zip
                        && (only_related_information
                            || only_extended_metadata
                            || asset.url_types.is_empty()))
                    || (is_other && asset.archive_name.is_some()))
        }
        AssetRole::Derived => false,
    };
    if valid {
        Ok(())
    } else {
        Err(classification_error("role_evidence"))
    }
}

fn has_only_format(asset: &ParsedAsset, allowed: &[&str]) -> bool {
    asset.formats.len() == 1
        && asset
            .formats
            .iter()
            .all(|value| allowed.contains(&value.as_str()))
}

fn merge_asset(existing: &mut ParsedAsset, incoming: ParsedAsset) -> ProviderResult<()> {
    if existing.asset_id != incoming.asset_id
        || existing.file_name != incoming.file_name
        || existing.media_type != incoming.media_type
        || existing.role != incoming.role
        || existing.archive_name != incoming.archive_name
        || existing.product_match != incoming.product_match
        || existing.size_bytes != incoming.size_bytes
        || existing.checksum != incoming.checksum
    {
        return Err(classification_error("alternate_route_conflict"));
    }
    existing.native_roles.extend(incoming.native_roles);
    existing.url_types.extend(incoming.url_types);
    existing.url_subtypes.extend(incoming.url_subtypes);
    existing.formats.extend(incoming.formats);
    existing.routes.extend(incoming.routes);
    if existing.native_roles.len() > NISAR_MAX_NATIVE_ROLES
        || existing.url_types.len() > NISAR_MAX_NATIVE_ROLES
        || existing.url_subtypes.len() > NISAR_MAX_NATIVE_ROLES
        || existing.formats.len() > NISAR_MAX_NATIVE_ROLES
    {
        return Err(limit_error("merged_native_roles"));
    }
    validate_role_evidence(existing, false)
}

#[derive(Clone, Debug)]
struct BuiltAsset {
    asset_id: String,
    file_name: String,
    media_type: String,
    stable_asset_key: String,
    asset: ProviderAsset,
    integrity: Option<Value>,
    routes: Vec<Value>,
    metadata: Value,
}

fn build_asset(provider_product_key: &str, asset: ParsedAsset) -> ProviderResult<BuiltAsset> {
    let stable_key = stable_asset_key(provider_product_key, &asset.asset_id)
        .map_err(|_| classification_error("stable_asset_key"))?;
    let route_values = asset
        .routes
        .values()
        .map(|(scheme, locator)| json!({"scheme": scheme, "locator": locator}))
        .collect::<Vec<_>>();
    let https = asset
        .routes
        .values()
        .find(|(scheme, _)| scheme == "https")
        .map(|(_, locator)| locator.clone());
    let access_descriptor = if asset.role == AssetRole::Primary {
        let descriptor = AssetAccessDescriptor {
            resolver_id: "earthdata_https".to_owned(),
            scheme: AssetAccessScheme::EarthdataHttps,
            stable_href: https.clone(),
            requires_credentials: true,
            credential_profile: Some("earthdata-default".to_owned()),
            resolver_parameters: AssetResolverParameters {},
        };
        descriptor
            .validate()
            .map_err(|_| classification_error("access_descriptor"))?;
        Some(descriptor)
    } else {
        None
    };
    let public_url = if matches!(
        asset.role,
        AssetRole::Preview | AssetRole::Browse | AssetRole::Thumbnail
    ) {
        https
    } else {
        None
    };
    let size_bytes = asset
        .size_bytes
        .map(|value| i64::try_from(value).expect("size was bounded"));
    let provider_asset = ProviderAsset {
        asset_key: Some(asset.asset_id.clone()),
        stable_asset_key: Some(stable_key.clone()),
        access_descriptor,
        role: asset.role.clone(),
        native_roles: asset.native_roles.iter().cloned().collect(),
        url: public_url,
        file_name: Some(asset.file_name.clone()),
        media_type: Some(asset.media_type.clone()),
        size_bytes,
    };
    provider_asset
        .validate()
        .map_err(|_| classification_error("provider_asset"))?;
    let integrity =
        asset
            .checksum
            .as_ref()
            .zip(asset.size_bytes)
            .map(|((algorithm, value), size_bytes)| {
                json!({
                    "file_name": asset.file_name,
                    "size_bytes": size_bytes,
                    "algorithm": algorithm,
                    "value": value
                })
            });
    let metadata = json!({
        "asset_key": asset.asset_id,
        "stable_asset_key": stable_key,
        "role": role_name(&asset.role),
        "native_roles": asset.native_roles,
        "file_name": asset.file_name,
        "media_type": asset.media_type,
        "size_bytes": asset.size_bytes,
        "checksum": asset.checksum.as_ref().map(|(algorithm, value)| json!({
            "algorithm": algorithm,
            "value": value
        })),
        "routes": route_values,
        "native_evidence": {
            "archive_name": asset.archive_name,
            "product_match": asset.product_match,
            "url_types": asset.url_types,
            "url_subtypes": asset.url_subtypes,
            "formats": asset.formats
        }
    });
    Ok(BuiltAsset {
        asset_id: provider_asset
            .asset_key
            .clone()
            .expect("asset key was assigned"),
        file_name: provider_asset
            .file_name
            .clone()
            .expect("file name was assigned"),
        media_type: provider_asset
            .media_type
            .clone()
            .expect("media type was assigned"),
        stable_asset_key: provider_asset
            .stable_asset_key
            .clone()
            .expect("stable key was assigned"),
        asset: provider_asset,
        integrity,
        routes: metadata["routes"]
            .as_array()
            .expect("routes metadata is an array")
            .clone(),
        metadata,
    })
}

fn parse_role(value: Option<&Value>) -> ProviderResult<AssetRole> {
    match value.and_then(Value::as_str) {
        Some("primary") => Ok(AssetRole::Primary),
        Some("preview") => Ok(AssetRole::Preview),
        Some("browse") => Ok(AssetRole::Browse),
        Some("thumbnail") => Ok(AssetRole::Thumbnail),
        Some("metadata") => Ok(AssetRole::Metadata),
        Some("auxiliary") => Ok(AssetRole::Auxiliary),
        _ => Err(classification_error("normalized_role")),
    }
}

fn bounded_string_set(
    map: &Map<String, Value>,
    key: &'static str,
) -> ProviderResult<BTreeSet<String>> {
    let values = map
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| classification_error(key))?;
    if values.len() > NISAR_MAX_NATIVE_ROLES {
        return Err(limit_error(key));
    }
    let mut result = BTreeSet::new();
    for value in values {
        let value = value
            .as_str()
            .filter(|value| valid_text(value))
            .ok_or_else(|| classification_error(key))?;
        if value.len() > NISAR_MAX_NATIVE_ROLE_BYTES {
            return Err(limit_error(key));
        }
        result.insert(value.to_owned());
    }
    Ok(result)
}

fn bounded_text(
    map: &Map<String, Value>,
    key: &'static str,
    maximum: usize,
) -> ProviderResult<String> {
    let value = map
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| valid_text(value))
        .ok_or_else(|| classification_error(key))?;
    if value.len() > maximum {
        return Err(limit_error(key));
    }
    Ok(value.to_owned())
}

fn bounded_value_text(value: &str, reason: &'static str, maximum: usize) -> ProviderResult<String> {
    if !valid_text(value) {
        return Err(classification_error(reason));
    }
    if value.len() > maximum {
        return Err(limit_error(reason));
    }
    Ok(value.to_owned())
}

fn parse_checksum(value: Option<&Value>) -> ProviderResult<Option<(String, String)>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_object()
        .ok_or_else(|| classification_error("checksum"))?;
    let algorithm = bounded_text(value, "algorithm", 32)?;
    let checksum = bounded_text(value, "value", 128)?.to_ascii_lowercase();
    let expected = match algorithm.as_str() {
        "MD5" => 32,
        "SHA-256" | "SHA256" => 64,
        _ => return Err(classification_error("checksum_algorithm")),
    };
    if checksum.len() != expected || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(classification_error("checksum_value"));
    }
    Ok(Some((algorithm, checksum)))
}

fn parse_routes(
    value: Option<&Value>,
    file_name: &str,
) -> ProviderResult<BTreeMap<(String, String), (String, String)>> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| classification_error("routes"))?;
    if values.len() > NISAR_MAX_RAW_RESOURCE_ENTRIES {
        return Err(limit_error("routes"));
    }
    let mut routes = BTreeMap::new();
    for value in values {
        let value = value
            .as_object()
            .ok_or_else(|| classification_error("route"))?;
        let scheme = bounded_text(value, "scheme", 16)?;
        let locator = value
            .get("locator")
            .and_then(Value::as_str)
            .filter(|value| valid_text(value))
            .ok_or_else(|| classification_error("route_locator"))?;
        if locator.len() > NISAR_MAX_LOCATOR_BYTES {
            return Err(limit_error("route_locator"));
        }
        validate_route(&scheme, locator, file_name)?;
        routes.insert(
            (scheme.clone(), locator.to_owned()),
            (scheme, locator.to_owned()),
        );
    }
    Ok(routes)
}

fn validate_route(scheme: &str, locator: &str, file_name: &str) -> ProviderResult<()> {
    let parsed = Url::parse(locator).map_err(|_| classification_error("route_locator"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| classification_error("route_locator"))?;
    let fixture_host = matches!(
        (scheme, host),
        ("https", "example.invalid") | ("s3", "example-invalid")
    );
    let official_host = matches!(
        (scheme, host),
        ("https", "nisar.asf.earthdatacloud.nasa.gov")
            | ("s3", "sds-n-cumulus-prod-nisar-products")
            | ("s3", "sds-n-cumulus-prod-nisar-browse")
    );
    let common_safe = (official_host || fixture_host)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && !host.to_ascii_lowercase().contains("bhoonidhi")
        && !host.to_ascii_lowercase().contains("private")
        && !parsed.path().to_ascii_lowercase().contains("/private/")
        && parsed
            .path()
            .rsplit('/')
            .next()
            .and_then(decode_route_basename)
            .is_some_and(|basename| basename == file_name);
    let scheme_safe = match scheme {
        "https" => parsed.scheme() == "https" && parsed.port_or_known_default() == Some(443),
        "s3" => parsed.scheme() == "s3" && parsed.port().is_none(),
        _ => false,
    };
    if common_safe && scheme_safe {
        Ok(())
    } else {
        Err(classification_error("route_locator"))
    }
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && !value.chars().any(char::is_control)
        && !value.contains('\0')
}

fn safe_basename(value: &str) -> bool {
    valid_text(value) && !value.contains(['/', '\\']) && value != "." && value != ".."
}

fn decode_route_basename(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let high = hex_nibble(bytes[index + 1])?;
            let low = hex_nibble(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if !safe_basename(&decoded) {
        return None;
    }
    Some(decoded)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn valid_media_type(value: &str) -> bool {
    valid_text(value)
        && value.split_once('/').is_some_and(|(kind, subtype)| {
            !kind.is_empty()
                && !subtype.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && byte != b';')
        })
}

fn matches_extension(value: &str, extensions: &[&str]) -> bool {
    extensions
        .iter()
        .any(|extension| value.ends_with(extension))
}

fn role_rank(role: &AssetRole) -> u8 {
    match role {
        AssetRole::Primary => 0,
        AssetRole::Preview => 1,
        AssetRole::Browse => 2,
        AssetRole::Thumbnail => 3,
        AssetRole::Metadata => 4,
        AssetRole::Auxiliary => 5,
        AssetRole::Derived => 6,
    }
}

fn role_name(role: &AssetRole) -> &'static str {
    match role {
        AssetRole::Primary => "primary",
        AssetRole::Preview => "preview",
        AssetRole::Browse => "browse",
        AssetRole::Thumbnail => "thumbnail",
        AssetRole::Metadata => "metadata",
        AssetRole::Auxiliary => "auxiliary",
        AssetRole::Derived => "derived",
    }
}

fn classification_error(reason: &'static str) -> ContractError {
    ContractError::new(
        "ASF_ASSET_CLASSIFICATION_INVALID",
        "NISAR 资产证据无法安全分类",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded NISAR asset error is safe")
}

fn limit_error(reason: &'static str) -> ContractError {
    ContractError::new(
        "ASF_ASSET_GRAPH_LIMIT_EXCEEDED",
        "NISAR 资产图超过冻结边界",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded NISAR asset limit error is safe")
}

fn primary_missing() -> ContractError {
    ContractError::new(
        "ASF_PRIMARY_ASSET_MISSING",
        "NISAR Granule 缺少主科学 HDF5",
        Map::new(),
    )
    .expect("bounded missing-primary error is safe")
}

fn primary_ambiguous() -> ContractError {
    ContractError::new(
        "ASF_PRIMARY_ASSET_AMBIGUOUS",
        "NISAR Granule 包含多个主科学 HDF5",
        Map::new(),
    )
    .expect("bounded ambiguous-primary error is safe")
}
