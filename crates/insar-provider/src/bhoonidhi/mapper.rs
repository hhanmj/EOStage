use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters, AssetRole,
    CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchResponse, CatalogSearchStats,
    ContractError, DataSourceKind, ProductKind, ProviderAsset, ProviderProduct, ProviderResult,
};
use serde_json::{Map, Value};
use url::Url;

use super::query::derive_bbox_from_geojson;
use super::{
    BHOONIDHI_COLLECTION_ID, BHOONIDHI_FILENAME_RULE_ID, BHOONIDHI_IDENTITY_RULE_ID,
    BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID,
};

const PUBLIC_ASSET_HOST: &str = "bhoonidhi.nrsc.gov.in";
const PUBLIC_ASSET_RESOLVER_ID: &str = "isro.nrsc.bhoonidhi.public-asset.v1";

fn error(code: &str, message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded Bhoonidhi mapper error is safe")
}

fn item_error(reason: &str) -> ContractError {
    error(
        "BHOONIDHI_ITEM_INVALID",
        "Bhoonidhi S-RSLC 目录项无效",
        reason,
    )
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "access_token",
        "refresh_token",
        "authorization",
        "cookie",
        "secret",
        "credential",
        "signed_url",
        "download_url",
    ]
    .iter()
    .any(|item| key == *item)
        || (key != "token" && key.ends_with("token"))
        || key.ends_with("_password")
        || key.contains("jwt")
        || (key.contains("signed") && key.contains("url"))
}

fn unsafe_locator(value: &str) -> bool {
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return false;
    }
    Url::parse(value)
        .map(|url| {
            !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
        })
        .unwrap_or(true)
}

fn validate_safe_metadata(value: &Value) -> ProviderResult<()> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if is_sensitive_key(key) {
                    return Err(item_error("sensitive_property"));
                }
                validate_safe_metadata(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_safe_metadata(value)?;
            }
        }
        Value::String(value) if value.chars().any(char::is_control) => {
            return Err(item_error("control_character"));
        }
        Value::String(value) if unsafe_locator(value) => {
            return Err(item_error("unsafe_property_locator"));
        }
        _ => {}
    }
    Ok(())
}

fn text_alias(
    properties: &Map<String, Value>,
    aliases: &[&str],
    field: &str,
    required: bool,
) -> ProviderResult<Option<String>> {
    let mut selected = None;
    for alias in aliases {
        let Some(value) = properties.get(*alias) else {
            continue;
        };
        let value = value.as_str().filter(|value| {
            !value.is_empty() && *value == value.trim() && !value.chars().any(char::is_control)
        });
        let Some(value) = value else {
            return Err(item_error(field));
        };
        if let Some(existing) = &selected {
            if existing != value {
                return Err(item_error("property_conflict"));
            }
        } else {
            selected = Some(value.to_owned());
        }
    }
    if required && selected.is_none() {
        return Err(item_error(field));
    }
    Ok(selected)
}

fn normalize_datetime(value: Option<String>) -> ProviderResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(&value)
        .map_err(|_| item_error("acquisition_datetime"))?
        .with_timezone(&Utc);
    Ok(Some(parsed.to_rfc3339_opts(SecondsFormat::AutoSi, true)))
}

fn normalize_direction(value: &str, field: &str) -> ProviderResult<String> {
    match value.trim().to_ascii_uppercase().as_str() {
        "ASCENDING" | "ASC" if field == "flight_direction" => Ok("ASCENDING".to_owned()),
        "DESCENDING" | "DESC" if field == "flight_direction" => Ok("DESCENDING".to_owned()),
        "LEFT" | "L" if field == "look_direction" => Ok("LEFT".to_owned()),
        "RIGHT" | "R" if field == "look_direction" => Ok("RIGHT".to_owned()),
        _ => Err(item_error(field)),
    }
}

fn safe_asset_url(value: &str) -> ProviderResult<Url> {
    let parsed = Url::parse(value).map_err(|_| item_error("asset_href"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(PUBLIC_ASSET_HOST)
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path().is_empty()
    {
        return Err(item_error("asset_origin"));
    }
    Ok(parsed)
}

fn asset_filename(url: &Url) -> Option<String> {
    let candidate = url.path_segments()?.next_back()?;
    (!candidate.is_empty() && !candidate.contains(['/', '\\']) && !candidate.contains('\0'))
        .then(|| candidate.to_owned())
}

fn map_public_asset(
    key: &str,
    value: &Value,
    item_id: &str,
) -> ProviderResult<Option<ProviderAsset>> {
    let role = match key {
        "metadata" => AssetRole::Metadata,
        "thumbnail" => AssetRole::Thumbnail,
        _ => return Ok(None),
    };
    let object = value.as_object().ok_or_else(|| item_error("asset_shape"))?;
    let href = object
        .get("href")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && *value == value.trim())
        .ok_or_else(|| item_error("asset_href"))?;
    let url = safe_asset_url(href)?;
    let media_type = object
        .get("type")
        .or_else(|| object.get("media_type"))
        .map(|value| value.as_str().ok_or_else(|| item_error("asset_media_type")))
        .transpose()?
        .map(str::to_owned);
    let stable_key = format!("{BHOONIDHI_PROVIDER_ID}:{BHOONIDHI_COLLECTION_ID}:{item_id}:{key}");
    Ok(Some(ProviderAsset {
        asset_key: Some(key.to_owned()),
        stable_asset_key: Some(stable_key),
        access_descriptor: Some(AssetAccessDescriptor {
            resolver_id: PUBLIC_ASSET_RESOLVER_ID.to_owned(),
            scheme: AssetAccessScheme::PublicHttps,
            stable_href: Some(url.to_string()),
            requires_credentials: false,
            credential_profile: None,
            resolver_parameters: AssetResolverParameters {},
        }),
        role,
        native_roles: vec![key.to_owned()],
        url: Some(url.to_string()),
        file_name: asset_filename(&url),
        media_type,
        size_bytes: None,
    }))
}

pub fn bhoonidhi_collection() -> CatalogCollectionDescriptor {
    CatalogCollectionDescriptor {
        logical_dataset: BHOONIDHI_LOGICAL_DATASET.to_owned(),
        provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
        native_collection_id: BHOONIDHI_COLLECTION_ID.to_owned(),
        collection_profile_id: BHOONIDHI_PROFILE_ID.to_owned(),
        identity_rule_id: BHOONIDHI_IDENTITY_RULE_ID.to_owned(),
        filename_rule_id: BHOONIDHI_FILENAME_RULE_ID.to_owned(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        production_enabled: true,
    }
}

/// Map one protected Bhoonidhi STAC Feature to the generic catalogue product.
///
/// The mapper requires exact item/collection/online identity and a polygonal
/// geometry. It retains unknown provider properties, including the observed
/// `AcquistionMode` spelling, but does not reinterpret opaque `CP` or `W` values.
pub fn map_bhoonidhi_feature(value: &Value) -> ProviderResult<ProviderProduct> {
    let object = value
        .as_object()
        .ok_or_else(|| item_error("feature_shape"))?;
    if object.get("type").and_then(Value::as_str) != Some("Feature") {
        return Err(item_error("feature_type"));
    }
    let item_id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && *value == value.trim()
                && value.len() <= 512
                && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| item_error("item_id"))?;
    let collection = object
        .get("collection")
        .and_then(Value::as_str)
        .ok_or_else(|| item_error("collection"))?;
    if collection != BHOONIDHI_COLLECTION_ID {
        return Err(item_error("collection_identity"));
    }
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| item_error("properties"))?;
    validate_safe_metadata(&Value::Object(properties.clone()))?;
    let property_id = text_alias(properties, &["ID", "id"], "property_id", true)?
        .expect("required property identity");
    if property_id != item_id {
        return Err(item_error("item_id_conflict"));
    }
    if text_alias(properties, &["Online"], "online", true)?.as_deref() != Some("Y") {
        return Err(item_error("online_must_be_y"));
    }

    let geometry = object
        .get("geometry")
        .filter(|value| !value.is_null())
        .and_then(Value::as_object)
        .ok_or_else(|| item_error("geometry"))?;
    let geometry_value = Value::Object(geometry.clone());
    let footprint_bbox =
        derive_bbox_from_geojson(&geometry_value).map_err(|_| item_error("geometry"))?;

    let datetime = text_alias(
        properties,
        &[
            "datetime",
            "AcquisitionDateTime",
            "AcquisitionDatetime",
            "AcquisitionTime",
        ],
        "acquisition_datetime",
        false,
    )?;
    let acquisition_datetime = normalize_datetime(datetime)?;
    let display_name = text_alias(
        properties,
        &["Scene_Identifier", "scene_identifier"],
        "scene_identifier",
        false,
    )?
    .unwrap_or_else(|| item_id.to_owned());

    let mut mapped_properties = properties.clone();
    for (aliases, normalized) in [
        (&["ProductCode", "product_code"][..], "product_code"),
        (
            &["SoftwareVersion", "software_version"][..],
            "software_version",
        ),
        (&["Producer", "producer"][..], "producer"),
        (&["Platform", "platform"][..], "platform"),
        (&["Instrument", "instrument"][..], "instrument"),
        (&["Mode", "mode"][..], "mode"),
        (&["Polarization", "polarization"][..], "polarization"),
        (
            &["Scene_Identifier", "scene_identifier"][..],
            "scene_identifier",
        ),
        (
            &["AcquistionMode", "AcquisitionMode", "acquisition_mode"][..],
            "acquisition_mode",
        ),
    ] {
        if let Some(value) = text_alias(properties, aliases, normalized, false)? {
            mapped_properties.insert(normalized.to_owned(), Value::String(value));
        }
    }
    if let Some(value) = text_alias(properties, &["Node", "node"], "flight_direction", false)? {
        mapped_properties.insert(
            "flight_direction".to_owned(),
            Value::String(normalize_direction(&value, "flight_direction")?),
        );
    }
    if let Some(value) = text_alias(
        properties,
        &["Look_dir", "LookDirection", "look_direction"],
        "look_direction",
        false,
    )? {
        mapped_properties.insert(
            "look_direction".to_owned(),
            Value::String(normalize_direction(&value, "look_direction")?),
        );
    }
    mapped_properties.insert(
        "provider_collection".to_owned(),
        Value::String(BHOONIDHI_COLLECTION_ID.to_owned()),
    );
    if let Some(raw_bbox) = object.get("bbox") {
        // Keep the provider's raw array as evidence only. `footprint_bbox` is
        // derived above and is the sole canonical bbox exposed to callers.
        mapped_properties.insert("provider_bbox".to_owned(), raw_bbox.clone());
    }

    let mut assets = Vec::new();
    if let Some(raw_assets) = object.get("assets") {
        let raw_assets = raw_assets
            .as_object()
            .ok_or_else(|| item_error("assets_shape"))?;
        for (key, value) in raw_assets {
            if let Some(asset) = map_public_asset(key, value, item_id)? {
                assets.push(asset);
            }
        }
    }
    assets.sort_by(|left, right| left.asset_key.cmp(&right.asset_key));

    let product = ProviderProduct {
        provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
        product_id: item_id.to_owned(),
        display_name,
        source_kind: DataSourceKind::Sar,
        product_kind: ProductKind::SarSlc,
        acquisition_datetime,
        footprint_geojson: Some(geometry.clone()),
        footprint_bbox: Some(footprint_bbox),
        assets,
        properties: mapped_properties,
        payload: Map::new(),
    };
    product
        .validate()
        .map_err(|_| item_error("product_contract"))?;
    Ok(product)
}

pub(crate) fn dedupe_products(
    products: Vec<ProviderProduct>,
) -> ProviderResult<(Vec<ProviderProduct>, u64)> {
    let mut unique = BTreeMap::new();
    let mut exact_duplicate_record_count = 0_u64;
    for product in products {
        match unique.get(&product.product_id) {
            Some(existing) if existing == &product => exact_duplicate_record_count += 1,
            Some(_) => {
                return Err(error(
                    "BHOONIDHI_PRODUCT_IDENTITY_CONFLICT",
                    "Bhoonidhi 返回了冲突的产品身份",
                    "product_identity",
                ));
            }
            None => {
                unique.insert(product.product_id.clone(), product);
            }
        }
    }
    Ok((unique.into_values().collect(), exact_duplicate_record_count))
}

// Keep independently observed pagination/count facts explicit at the response boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) fn response_from_products(
    products: Vec<ProviderProduct>,
    requested_limit: Option<u64>,
    matched_count: Option<u64>,
    request_count: u64,
    transport_part_count: u64,
    fetched_record_count: u64,
    exact_duplicate_record_count: u64,
    next_cursor: Option<String>,
    remote_exhausted: bool,
    elapsed_ms: u64,
) -> ProviderResult<CatalogSearchResponse> {
    let mut unique = BTreeMap::new();
    for product in products {
        unique.insert(product.product_id.clone(), product);
    }
    let exact_matched_count = unique.len() as u64;
    // The catalogue contract counts all distinct provider products before
    // the exact AOI spatial filter, not only products admitted into the
    // returned working set. Duplicates are accounted for independently.
    let unique_provider_product_count = fetched_record_count
        .checked_sub(exact_duplicate_record_count)
        .ok_or_else(|| {
            error(
                "BHOONIDHI_RESPONSE_INVALID",
                "Bhoonidhi 目录计数无效",
                "count_overflow",
            )
        })?;
    let mut products = unique.into_values().collect::<Vec<_>>();
    if let Some(limit) = requested_limit {
        products.truncate(limit as usize);
    }
    let response = CatalogSearchResponse {
        collection: bhoonidhi_collection(),
        stats: CatalogSearchStats {
            requested_limit,
            matched_count,
            request_count,
            transport_part_count,
            fetched_record_count,
            mapped_record_count: fetched_record_count,
            profile_filtered_record_count: 0,
            exact_duplicate_record_count,
            unique_provider_product_count,
            exact_matched_count,
            returned_count: products.len() as u64,
            remote_exhausted,
            working_set_satisfied: true,
            elapsed_ms,
            truncated: next_cursor.is_some(),
        },
        products,
        next_cursor,
        item_errors: Vec::new(),
    };
    response.validate().map_err(|_| {
        error(
            "BHOONIDHI_RESPONSE_INVALID",
            "Bhoonidhi 目录响应无效",
            "catalogue_contract",
        )
    })?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ITEM_ID: &str = "NISAR_S1_PR_RSLC_029_063_D_079_2500_CRNA_A_20260829T135915_20260829T135945_P00500_N_P_I_001";

    fn feature() -> Value {
        json!({
            "type": "Feature",
            "id": ITEM_ID,
            "collection": BHOONIDHI_COLLECTION_ID,
            "bbox": [67.5, 20.1, 70.2, 17.7],
            "geometry": {"type":"Polygon","coordinates":[[[68.0,18.0],[70.0,18.0],[70.0,20.0],[68.0,20.0],[68.0,18.0]]]},
            "properties": {
                "ID": ITEM_ID,
                "Online": "Y",
                "ProductCode": "ST0000KED",
                "SoftwareVersion": "1.4.00",
                "Producer": "ISRO",
                "Platform": "NISAR",
                "Instrument": "SSAR",
                "Mode": "W",
                "Polarization": "CP",
                "AcquistionMode": "W",
                "Scene_Identifier": "2026241135900_63_79",
                "datetime": "2026-08-29T13:59:15Z"
            },
            "assets": {
                "metadata": {"href":"https://bhoonidhi.nrsc.gov.in/NISAR/example.met", "type":"application/json"},
                "thumbnail": {"href":"https://bhoonidhi.nrsc.gov.in/NISAR/example.jpg", "type":"image/jpeg"}
            },
            "links": []
        })
    }

    #[test]
    fn maps_exact_identity_and_derives_bbox_from_geometry() {
        let product = map_bhoonidhi_feature(&feature()).expect("mapped product");
        assert_eq!(product.provider_id, BHOONIDHI_PROVIDER_ID);
        assert_eq!(product.product_id, ITEM_ID);
        assert_eq!(product.footprint_bbox.as_ref().unwrap().west, 68.0);
        assert_eq!(product.footprint_bbox.as_ref().unwrap().east, 70.0);
        assert_eq!(product.footprint_bbox.as_ref().unwrap().south, 18.0);
        assert_eq!(product.footprint_bbox.as_ref().unwrap().north, 20.0);
        assert_eq!(product.properties["AcquistionMode"], "W");
        assert_eq!(product.properties["acquisition_mode"], "W");
        assert_eq!(product.properties["mode"], "W");
        assert_eq!(product.properties["polarization"], "CP");
        assert_eq!(product.assets.len(), 2);
        assert!(product
            .assets
            .iter()
            .all(|asset| asset.role != AssetRole::Primary));
    }

    #[test]
    fn maps_redacted_offline_catalogue_fixture() {
        let feature: Value = serde_json::from_str(super::super::fixtures::CATALOGUE_FEATURE_JSON)
            .expect("redacted catalogue fixture");
        let product = map_bhoonidhi_feature(&feature).expect("fixture product");
        assert_eq!(product.product_id, feature["id"]);
        assert_eq!(product.properties["polarization"], "CP");
        assert_eq!(product.properties["mode"], "W");
        assert_eq!(product.properties["acquisition_mode"], "J");
        assert_eq!(product.properties["flight_direction"], "DESCENDING");
        assert_eq!(product.properties["look_direction"], "LEFT");
        assert_eq!(product.footprint_bbox.as_ref().unwrap().west, 74.84861701);
        assert_eq!(product.footprint_bbox.as_ref().unwrap().east, 77.18389716);
        assert_eq!(product.footprint_bbox.as_ref().unwrap().south, 5.147219959);
        assert_eq!(product.footprint_bbox.as_ref().unwrap().north, 6.223155245);
    }

    #[test]
    fn all_bbox_candidates_spatially_excluded_keep_provider_accounting() {
        // A paged response may contain no true-AOI hits and still have a
        // next cursor. Remote unique count is not the same as admitted hits.
        let response = response_from_products(
            Vec::new(),
            Some(2),
            Some(3),
            1,
            1,
            1,
            0,
            Some("opaque-next-page".to_owned()),
            false,
            0,
        )
        .unwrap();
        assert!(response.products.is_empty());
        assert_eq!(response.stats.unique_provider_product_count, 1);
        assert_eq!(response.stats.exact_matched_count, 0);
        assert_eq!(response.stats.matched_count, Some(3));
        assert_eq!(response.stats.fetched_record_count, 1);
        assert!(response.next_cursor.is_some());
    }

    #[test]
    fn online_and_identity_mismatches_fail_closed() {
        let mut offline = feature();
        offline["properties"]["Online"] = json!("N");
        assert_eq!(
            map_bhoonidhi_feature(&offline).unwrap_err().debug_context["reason"],
            "online_must_be_y"
        );

        let mut mismatch = feature();
        mismatch["properties"]["ID"] = json!("OTHER");
        assert_eq!(
            map_bhoonidhi_feature(&mismatch).unwrap_err().debug_context["reason"],
            "item_id_conflict"
        );
    }

    #[test]
    fn raw_bbox_is_retained_as_evidence_only() {
        let product = map_bhoonidhi_feature(&feature()).expect("mapped product");
        assert_eq!(
            product.properties["provider_bbox"],
            json!([67.5, 20.1, 70.2, 17.7])
        );
        assert_eq!(product.footprint_bbox.as_ref().unwrap().south, 18.0);
    }

    #[test]
    fn signed_or_query_bearing_property_locators_are_rejected() {
        let mut signed = feature();
        signed["properties"]["download_url"] =
            json!("https://bhoonidhi.nrsc.gov.in/download?id=secret");
        assert_eq!(
            map_bhoonidhi_feature(&signed).unwrap_err().debug_context["reason"],
            "sensitive_property"
        );

        let mut locator = feature();
        locator["properties"]["reference"] =
            json!("https://bhoonidhi.nrsc.gov.in/item?token=opaque");
        assert_eq!(
            map_bhoonidhi_feature(&locator).unwrap_err().debug_context["reason"],
            "unsafe_property_locator"
        );
    }
}
