use chrono::{DateTime, SecondsFormat, Utc};
use insar_core::{
    provider_product_key, CatalogSearchQuery, ContractError, DataSourceKind, ProductKind,
    ProviderProduct, ProviderResult,
};
use serde_json::{Map, Value};

use crate::catalog::{
    CollectionProfile, DatasetRoutePolicyRegistry, NisarRslcProductSpecificationProfileRegistry,
    NISAR_BAND, NISAR_COLLECTION_SHORT_NAME, NISAR_COLLECTION_VERSION, NISAR_FILENAME_RULE_ID,
    NISAR_FLIGHT_DIRECTIONS, NISAR_FRAME_COVERAGES, NISAR_FREQUENCY_A_POLARIZATIONS,
    NISAR_FREQUENCY_B_POLARIZATIONS, NISAR_IDENTITY_RULE_ID, NISAR_LOGICAL_DATASET,
    NISAR_LOOK_DIRECTIONS, NISAR_NATIVE_COLLECTION_ID, NISAR_PRODUCTION_CONFIGURATION,
    NISAR_PROFILE_ID, NISAR_PROVIDER_ID, NISAR_QUALITY_STATUS, NISAR_RANGE_BANDWIDTHS,
    NISAR_REQUIRED_PRODUCT_TYPE, NISAR_RESOLVER_POLICY_ID, NISAR_SCIENCE_LEVEL,
};

use super::query::{
    catalogue_filter_error, catalogue_query_error, catalogue_spatial_plan, catalogue_temporal,
    catalogue_transport_page_size, optional_catalogue_integer, optional_catalogue_string,
    push_cmr_attribute, AsfCmrQuery,
};

const MAX_CMR_INTEGER: u64 = i32::MAX as u64;

pub use super::nisar_assets::{
    classify_nisar_assets, NisarAssetGraph, NISAR_MAX_ASSET_GRAPH_JSON_BYTES,
    NISAR_MAX_DEDUPLICATED_ASSETS, NISAR_MAX_FILE_NAME_BYTES, NISAR_MAX_LOCATOR_BYTES,
    NISAR_MAX_MEDIA_TYPE_BYTES, NISAR_MAX_NATIVE_ROLES, NISAR_MAX_NATIVE_ROLE_BYTES,
    NISAR_MAX_RAW_RESOURCE_ENTRIES,
};
use super::umm::CmrGranuleEnvelope;

const NISAR_BETA_COLLECTION_ID: &str = "C2850225585-ASF";
const NISAR_BETA_COLLECTION_SHORT_NAME: &str = "NISAR_L1_RSLC_BETA_V1";

pub fn map_nisar_rslc_umm_granule(
    envelope: &CmrGranuleEnvelope,
    profile: &CollectionProfile,
) -> ProviderResult<ProviderProduct> {
    validate_nisar_profile(profile)?;
    if envelope
        .collection
        .get("concept_id")
        .and_then(Value::as_str)
        == Some(NISAR_BETA_COLLECTION_ID)
        || envelope
            .collection
            .get("short_name")
            .and_then(Value::as_str)
            == Some(NISAR_BETA_COLLECTION_SHORT_NAME)
        || envelope
            .granule
            .get("quality_status")
            .and_then(Value::as_str)
            == Some("Beta")
    {
        return Err(quality_error("beta"));
    }
    validate_nisar_collection(&envelope.collection)?;

    let concept_id = required_product_text(&envelope.granule, "concept_id")?;
    if !valid_cmr_granule_concept_id(&concept_id) {
        return Err(product_identity_error("granule_concept_id"));
    }
    let producer_granule_id = required_product_text(&envelope.granule, "producer_granule_id")?;
    let processing_level = required_scope_text(&envelope.granule, "processing_level")?;
    let science_level = required_scope_text(&envelope.granule, "science_level")?;
    let quality_status = required_scope_text(&envelope.granule, "quality_status")?;
    if quality_status != NISAR_QUALITY_STATUS {
        return Err(quality_error("quality_status"));
    }
    let production_configuration =
        required_scope_text(&envelope.granule, "production_configuration")?;
    let band = required_scope_text(&envelope.granule, "band")?;
    if processing_level != NISAR_REQUIRED_PRODUCT_TYPE
        || science_level != NISAR_SCIENCE_LEVEL
        || production_configuration != NISAR_PRODUCTION_CONFIGURATION
        || band != NISAR_BAND
    {
        return Err(scope_error("product_scope"));
    }
    let crid = required_scope_text(&envelope.granule, "crid")?;
    let product_specification_profile = NisarRslcProductSpecificationProfileRegistry
        .for_crid(&crid)
        .ok_or_else(|| scope_error("crid"))?;

    let frame_coverage = required_scope_text(&envelope.granule, "frame_coverage")?;
    if !NISAR_FRAME_COVERAGES.contains(&frame_coverage.as_str()) {
        return Err(scope_error("frame_coverage"));
    }
    let frequency_a = optional_scope_text(&envelope.granule, "frequency_a_polarization")?;
    let frequency_b = optional_scope_text(&envelope.granule, "frequency_b_polarization")?;
    if frequency_a.is_none() && frequency_b.is_none() {
        return Err(scope_error("frequency_presence"));
    }
    if frequency_a
        .as_deref()
        .is_some_and(|value| !NISAR_FREQUENCY_A_POLARIZATIONS.contains(&value))
        || frequency_b
            .as_deref()
            .is_some_and(|value| !NISAR_FREQUENCY_B_POLARIZATIONS.contains(&value))
    {
        return Err(scope_error("frequency_polarization"));
    }
    let range_bandwidth = required_scope_text(&envelope.granule, "range_bandwidth")?;
    if !NISAR_RANGE_BANDWIDTHS.contains(&range_bandwidth.as_str()) {
        return Err(scope_error("range_bandwidth"));
    }
    let joint_observation = envelope
        .granule
        .get("joint_observation")
        .and_then(Value::as_bool)
        .ok_or_else(|| scope_error("joint_observation"))?;
    let flight_direction = required_scope_text(&envelope.granule, "flight_direction")?;
    let look_direction = required_scope_text(&envelope.granule, "look_direction")?;
    if !NISAR_FLIGHT_DIRECTIONS.contains(&flight_direction.as_str())
        || !NISAR_LOOK_DIRECTIONS.contains(&look_direction.as_str())
    {
        return Err(scope_error("direction"));
    }
    let track = required_positive_integer(&envelope.granule, "track")?;
    let relative_orbit = required_positive_integer(&envelope.granule, "relative_orbit")?;
    let frame = required_positive_integer(&envelope.granule, "frame")?;
    if track != relative_orbit {
        return Err(scope_error("track_relative_orbit"));
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
        provider_product_key(NISAR_PROVIDER_ID, NISAR_NATIVE_COLLECTION_ID, &concept_id)
            .map_err(|_| product_identity_error("provider_product_key"))?;
    let graph = classify_nisar_assets(
        envelope.granule.get("assets").unwrap_or(&Value::Null),
        &product_key,
    )?;
    let primary = graph
        .assets
        .iter()
        .find(|asset| asset.asset_key.as_deref() == Some(graph.primary_asset_key.as_str()))
        .expect("classified primary key exists");
    let expected_primary_name = format!("{producer_granule_id}.h5");
    if primary.file_name.as_deref() != Some(expected_primary_name.as_str()) {
        return Err(classification_error("primary_product_identity"));
    }

    let mut properties = Map::from_iter([
        (
            "provider_product_key".to_owned(),
            Value::String(product_key),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(concept_id.clone()),
        ),
        (
            "cmr_granule_concept_id".to_owned(),
            Value::String(concept_id.clone()),
        ),
        (
            "producer_granule_id".to_owned(),
            Value::String(producer_granule_id.clone()),
        ),
        (
            "logical_dataset".to_owned(),
            Value::String(NISAR_LOGICAL_DATASET.to_owned()),
        ),
        (
            "collection_concept_id".to_owned(),
            Value::String(NISAR_NATIVE_COLLECTION_ID.to_owned()),
        ),
        (
            "collection_short_name".to_owned(),
            Value::String(NISAR_COLLECTION_SHORT_NAME.to_owned()),
        ),
        (
            "collection_version".to_owned(),
            Value::String(NISAR_COLLECTION_VERSION.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(NISAR_PROFILE_ID.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(NISAR_IDENTITY_RULE_ID.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(NISAR_FILENAME_RULE_ID.to_owned()),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(product_specification_profile.resolver_policy_id.to_owned()),
        ),
        (
            "processing_level".to_owned(),
            Value::String(processing_level),
        ),
        ("science_level".to_owned(), Value::String(science_level)),
        ("quality_status".to_owned(), Value::String(quality_status)),
        (
            "production_configuration".to_owned(),
            Value::String(production_configuration),
        ),
        ("band".to_owned(), Value::String(band)),
        ("frame_coverage".to_owned(), Value::String(frame_coverage)),
        ("frame".to_owned(), Value::from(frame)),
        ("track".to_owned(), Value::from(track)),
        ("relative_orbit".to_owned(), Value::from(relative_orbit)),
        (
            "flight_direction".to_owned(),
            Value::String(flight_direction),
        ),
        ("look_direction".to_owned(), Value::String(look_direction)),
        ("range_bandwidth".to_owned(), Value::String(range_bandwidth)),
        (
            "joint_observation".to_owned(),
            Value::Bool(joint_observation),
        ),
        (
            "acquisition_end_utc".to_owned(),
            Value::String(acquisition_end),
        ),
        (
            "primary_asset_key".to_owned(),
            Value::String(graph.primary_asset_key.clone()),
        ),
        ("asset_integrity".to_owned(), Value::Object(graph.integrity)),
        ("asset_routes".to_owned(), Value::Object(graph.routes)),
        ("assets".to_owned(), Value::Array(graph.metadata_assets)),
        (
            "preview_available".to_owned(),
            Value::Bool(graph.assets.iter().any(|asset| {
                matches!(
                    asset.role,
                    insar_core::AssetRole::Preview | insar_core::AssetRole::Browse
                )
            })),
        ),
        ("full_product".to_owned(), Value::Bool(true)),
        ("crid".to_owned(), Value::String(crid)),
        (
            "product_specification_version".to_owned(),
            Value::String(
                product_specification_profile
                    .product_specification_version
                    .to_owned(),
            ),
        ),
        (
            "product_specification_profile_registry_version".to_owned(),
            Value::from(product_specification_profile.registry_version),
        ),
    ]);
    for key in ["asf_product_id", "pge_version"] {
        if let Some(value) = optional_scope_text(&envelope.granule, key)? {
            properties.insert(key.to_owned(), Value::String(value));
        }
    }
    if let Some(value) = frequency_a {
        properties.insert("frequency_a_polarization".to_owned(), Value::String(value));
    }
    if let Some(value) = frequency_b {
        properties.insert("frequency_b_polarization".to_owned(), Value::String(value));
    }

    let product = ProviderProduct {
        provider_id: NISAR_PROVIDER_ID.to_owned(),
        product_id: concept_id,
        display_name: producer_granule_id,
        source_kind: DataSourceKind::Sar,
        product_kind: ProductKind::SarProduct,
        acquisition_datetime: Some(acquisition_start),
        footprint_geojson: Some(footprint),
        footprint_bbox: None,
        assets: graph.assets,
        properties,
        payload: Map::new(),
    };
    product
        .validate()
        .map_err(|_| classification_error("provider_product"))?;
    Ok(product)
}

/// Translate the closed NISAR Provisional RSLC query into typed CMR parameters.
pub fn build_nisar_rslc_query(
    query: &CatalogSearchQuery,
    profile: &CollectionProfile,
) -> ProviderResult<AsfCmrQuery> {
    let query = &insar_core::clamp_catalogue_search_query(query);
    query
        .validate()
        .map_err(|reason| catalogue_query_error("CATALOGUE_QUERY_INVALID", "query", reason))?;
    if query.cursor.is_some() {
        return Err(catalogue_query_error(
            "CATALOGUE_CURSOR_UNSUPPORTED",
            "cursor",
            "external_continuation",
        ));
    }
    if query.logical_dataset != NISAR_LOGICAL_DATASET
        || query.provider_id != NISAR_PROVIDER_ID
        || query.native_collection_id != NISAR_NATIVE_COLLECTION_ID
        || profile.profile_id != NISAR_PROFILE_ID
        || profile.logical_dataset != NISAR_LOGICAL_DATASET
        || profile.provider_id != NISAR_PROVIDER_ID
        || profile.native_collection_id != NISAR_NATIVE_COLLECTION_ID
        || profile.required_product_type != NISAR_REQUIRED_PRODUCT_TYPE
        || profile.identity_rule_id != NISAR_IDENTITY_RULE_ID
        || profile.filename_rule_id != NISAR_FILENAME_RULE_ID
        || profile.resolver_policy_id != NISAR_RESOLVER_POLICY_ID
    {
        return Err(catalogue_query_error(
            "ASF_COLLECTION_IDENTITY_MISMATCH",
            "collection",
            "profile_or_query",
        ));
    }
    let filter_keys = query.filters.keys().map(String::as_str).collect::<Vec<_>>();
    DatasetRoutePolicyRegistry
        .validate_filter_keys(NISAR_LOGICAL_DATASET, &filter_keys)
        .map_err(|_| {
            catalogue_query_error(
                "CATALOGUE_FILTER_UNSUPPORTED",
                "filters",
                "unsupported_filter",
            )
        })?;

    let spatial_plan = match query.aoi_geojson.as_ref() {
        Some(aoi_geojson) => catalogue_spatial_plan(aoi_geojson)?,
        None => super::query::CatalogueSpatialPlan::empty(),
    };
    let polygons = spatial_plan.polygons.clone();
    let is_multi_polygon = polygons.len() > 1;
    let provider_page_size = catalogue_transport_page_size(query.result_limit, is_multi_polygon);
    if query.aoi_geojson.is_some() && polygons.is_empty() {
        return Err(catalogue_query_error(
            "CATALOGUE_QUERY_INVALID",
            "aoi_geojson",
            "empty_polygon_plan",
        ));
    }

    let mut params = vec![(
        "collection_concept_id".to_owned(),
        NISAR_NATIVE_COLLECTION_ID.to_owned(),
    )];
    if let Some(value) = query.filters.get("datetime") {
        params.push(("temporal".to_owned(), catalogue_temporal(value)?));
    }
    push_cmr_attribute(&mut params, "PRODUCT_TYPE", NISAR_REQUIRED_PRODUCT_TYPE);
    push_cmr_attribute(
        &mut params,
        "PRODUCTION_PIPELINE",
        NISAR_PRODUCTION_CONFIGURATION,
    );
    params.push((
        "options[attribute][exclude_collection]".to_owned(),
        "true".to_owned(),
    ));
    params.extend([
        ("page_size".to_owned(), provider_page_size.to_string()),
        ("sort_key[]".to_owned(), "start_date".to_owned()),
        ("sort_key[]".to_owned(), "granule_ur".to_owned()),
    ]);

    if let Some(value) = canonical_string(query, "production_configuration")? {
        if value != NISAR_PRODUCTION_CONFIGURATION {
            return Err(catalogue_filter_error("production_configuration"));
        }
    }
    if let Some(value) = canonical_string(query, "frame_coverage")? {
        if !NISAR_FRAME_COVERAGES.contains(&value.as_str()) {
            return Err(catalogue_filter_error("frame_coverage"));
        }
        push_cmr_attribute(
            &mut params,
            "FULL_FRAME",
            if value == "FULL" { "TRUE" } else { "FALSE" },
        );
    }
    push_exact_string_filter(
        query,
        &mut params,
        "frequency_a_polarization",
        "FREQUENCY_A_POLARIZATION_CONCAT",
        NISAR_FREQUENCY_A_POLARIZATIONS,
    )?;
    push_exact_string_filter(
        query,
        &mut params,
        "frequency_b_polarization",
        "FREQUENCY_B_POLARIZATION_CONCAT",
        NISAR_FREQUENCY_B_POLARIZATIONS,
    )?;
    push_exact_string_filter(
        query,
        &mut params,
        "range_bandwidth",
        "RANGE_BANDWIDTH_CONCAT",
        NISAR_RANGE_BANDWIDTHS,
    )?;

    if let Some(value) = optional_bool(query, "joint_observation")? {
        push_cmr_attribute(
            &mut params,
            "JOINT_OBSERVATION",
            if value { "TRUE" } else { "FALSE" },
        );
    }
    push_exact_string_filter(
        query,
        &mut params,
        "flight_direction",
        "ASCENDING_DESCENDING",
        NISAR_FLIGHT_DIRECTIONS,
    )?;

    let mut complete_batch_local_filters = Vec::new();
    if let Some(value) = canonical_string(query, "look_direction")? {
        if !NISAR_LOOK_DIRECTIONS.contains(&value.as_str()) {
            return Err(catalogue_filter_error("look_direction"));
        }
        complete_batch_local_filters.push(("look_direction".to_owned(), value));
    }

    let track = bounded_integer(query, "track")?;
    let relative_orbit = bounded_integer(query, "relative_orbit")?;
    if track.is_some() && relative_orbit.is_some() && track != relative_orbit {
        return Err(catalogue_filter_error("track"));
    }
    if let Some(value) = track.or(relative_orbit) {
        push_cmr_integer_attribute(&mut params, "TRACK_NUMBER", value);
    }
    if let Some(value) = bounded_integer(query, "frame")? {
        push_cmr_integer_attribute(&mut params, "FRAME_NUMBER", value);
    }

    Ok(AsfCmrQuery {
        common_params: params,
        canonical_geometry: spatial_plan.canonical_geometry,
        polygons,
        result_limit: query.result_limit,
        provider_page_size,
        is_multi_polygon,
        complete_batch_local_filters,
        spatial_transport_mode: spatial_plan.mode,
        requires_exact_spatial_post_filter: spatial_plan.requires_exact_post_filter,
        canonical_polygon_count: spatial_plan.canonical_polygon_count,
        canonical_ring_count: spatial_plan.canonical_ring_count,
        canonical_vertex_count: spatial_plan.canonical_vertex_count,
        transport_part_count: spatial_plan.transport_part_count,
        transport_vertex_counts: spatial_plan.transport_vertex_counts,
    })
}

fn canonical_string(
    query: &CatalogSearchQuery,
    key: &'static str,
) -> ProviderResult<Option<String>> {
    optional_catalogue_string(&query.filters, key)
        .map(|value| value.map(|value| value.to_ascii_uppercase()))
}

fn push_exact_string_filter(
    query: &CatalogSearchQuery,
    params: &mut Vec<(String, String)>,
    key: &'static str,
    attribute: &str,
    allowed: &[&str],
) -> ProviderResult<()> {
    let Some(value) = canonical_string(query, key)? else {
        return Ok(());
    };
    if !allowed.contains(&value.as_str()) {
        return Err(catalogue_filter_error(key));
    }
    push_cmr_attribute(params, attribute, &value);
    Ok(())
}

fn optional_bool(query: &CatalogSearchQuery, key: &'static str) -> ProviderResult<Option<bool>> {
    let Some(value) = query.filters.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| catalogue_filter_error(key))
}

fn bounded_integer(query: &CatalogSearchQuery, key: &'static str) -> ProviderResult<Option<u64>> {
    let value = optional_catalogue_integer(&query.filters, key)?;
    if value.is_some_and(|value| value == 0 || value > MAX_CMR_INTEGER) {
        return Err(catalogue_filter_error(key));
    }
    Ok(value)
}

fn push_cmr_integer_attribute(params: &mut Vec<(String, String)>, name: &str, value: u64) {
    params.push(("attribute[]".to_owned(), format!("int,{name},{value}")));
}

fn validate_nisar_profile(profile: &CollectionProfile) -> ProviderResult<()> {
    if profile.profile_id != NISAR_PROFILE_ID
        || profile.logical_dataset != NISAR_LOGICAL_DATASET
        || profile.provider_id != NISAR_PROVIDER_ID
        || profile.native_collection_id != NISAR_NATIVE_COLLECTION_ID
        || profile.required_product_type != NISAR_REQUIRED_PRODUCT_TYPE
        || profile.identity_rule_id != NISAR_IDENTITY_RULE_ID
        || profile.filename_rule_id != NISAR_FILENAME_RULE_ID
        || profile.resolver_policy_id != NISAR_RESOLVER_POLICY_ID
    {
        return Err(collection_error("profile"));
    }
    Ok(())
}

fn validate_nisar_collection(collection: &Map<String, Value>) -> ProviderResult<()> {
    let matches = collection.get("concept_id").and_then(Value::as_str)
        == Some(NISAR_NATIVE_COLLECTION_ID)
        && collection.get("short_name").and_then(Value::as_str)
            == Some(NISAR_COLLECTION_SHORT_NAME)
        && collection.get("version").and_then(Value::as_str) == Some(NISAR_COLLECTION_VERSION)
        && collection.get("cmr_provider_id").and_then(Value::as_str) == Some("ASF");
    if matches {
        Ok(())
    } else {
        Err(collection_error("collection"))
    }
}

fn required_product_text(map: &Map<String, Value>, key: &'static str) -> ProviderResult<String> {
    map.get(key)
        .and_then(Value::as_str)
        .filter(|value| valid_mapper_text(value))
        .map(str::to_owned)
        .ok_or_else(|| product_identity_error(key))
}

fn required_scope_text(map: &Map<String, Value>, key: &'static str) -> ProviderResult<String> {
    optional_scope_text(map, key)?.ok_or_else(|| scope_error(key))
}

fn optional_scope_text(
    map: &Map<String, Value>,
    key: &'static str,
) -> ProviderResult<Option<String>> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|value| valid_mapper_text(value))
            .map(|value| Some(value.to_owned()))
            .ok_or_else(|| scope_error(key)),
    }
}

fn required_positive_integer(map: &Map<String, Value>, key: &'static str) -> ProviderResult<u64> {
    map.get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0 && *value <= MAX_CMR_INTEGER)
        .ok_or_else(|| scope_error(key))
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

fn valid_mapper_text(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.len() <= NISAR_MAX_FILE_NAME_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_cmr_granule_concept_id(value: &str) -> bool {
    value
        .strip_prefix('G')
        .and_then(|value| value.strip_suffix("-ASF"))
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn collection_error(reason: &'static str) -> ContractError {
    mapper_error(
        "ASF_COLLECTION_IDENTITY_MISMATCH",
        "NISAR Collection 身份不匹配",
        reason,
    )
}

fn product_identity_error(reason: &'static str) -> ContractError {
    mapper_error(
        "ASF_PRODUCT_IDENTITY_MISSING",
        "NISAR Granule 缺少有效身份",
        reason,
    )
}

fn quality_error(reason: &'static str) -> ContractError {
    mapper_error(
        "NISAR_QUALITY_STATUS_MISMATCH",
        "NISAR 产品质量状态不属于 Provisional 路由",
        reason,
    )
}

fn scope_error(reason: &'static str) -> ContractError {
    mapper_error(
        "ASF_PRODUCT_SCOPE_MISMATCH",
        "NISAR 产品不属于冻结的 L-band RSLC/PR 范围",
        reason,
    )
}

fn classification_error(reason: &'static str) -> ContractError {
    mapper_error(
        "ASF_ASSET_CLASSIFICATION_INVALID",
        "NISAR 资产身份与产品证据不一致",
        reason,
    )
}

fn mapper_error(code: &'static str, message: &'static str, reason: &'static str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded NISAR mapper error is safe")
}
