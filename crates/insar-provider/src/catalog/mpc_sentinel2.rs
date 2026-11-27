use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::planetary_computer_asset::{
    sentinel2_l2a_reflectance_band_resolution, validate_planetary_computer_reviewed_asset_identity,
    PlanetaryComputerReviewedAssetClass,
};
use insar_core::{
    CatalogDeliveryModel, CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats,
    ContractError, DataSourceKind, ProductKind, ProviderNetworkPolicy, ProviderProduct,
    ProviderResult,
};
use reqwest::Url;
use serde_json::{Map, Value};

use super::sentinel2_filters::{decode_sentinel2_spacecraft, Sentinel2Platform};
use super::spatial::{
    authoritative_catalogue_geometry, bounded_catalogue_request_tiles,
    order_request_tiles_by_exact_overlap, PreparedCatalogueAoi,
};
use super::stac::{StacSearchBudget, StacTransport};
use super::{
    StacCollectionBinding, StacQueryOperator, StacQueryValueKind, StacQueryableBinding,
    StacRouteProfile, StacTransportLimits, MPC_SENTINEL2_L2A_COLLECTION_ID,
    MPC_SENTINEL2_L2A_FILENAME_RULE_ID, MPC_SENTINEL2_L2A_IDENTITY_RULE_ID,
    MPC_SENTINEL2_L2A_PROFILE_ID, MPC_SENTINEL2_L2A_PROVIDER_ID,
};

pub const MPC_SENTINEL2_STAC_ENDPOINT: &str =
    "https://planetarycomputer.microsoft.com/api/stac/v1/";

const MPC_SENTINEL2_SPATIAL_MAX_PARTS: usize = 8;
const MPC_SENTINEL2_SPATIAL_LONGITUDE_SPAN: f64 = 15.0;
const MPC_SENTINEL2_SPATIAL_LATITUDE_SPAN: f64 = 8.0;
const MPC_SENTINEL2_SPATIAL_RECORD_BUDGET: u64 = insar_core::SEARCH_SAFETY_CAP;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MpcSentinel2BandAsset {
    pub asset_key: String,
    pub stable_href: String,
    pub file_name: String,
    pub resolution_m: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MpcSentinel2BandManifest {
    pub item_id: String,
    pub safe_name: String,
    pub assets: Vec<MpcSentinel2BandAsset>,
}

/// Anonymous, fixed-origin Microsoft Sentinel-2 catalogue client.
///
/// Search does not grant asset execution. A separate exact map lookup verifies
/// the full SAFE name against the remote Item before granting a rendering source.
pub struct MpcSentinel2CatalogClient {
    transport: StacTransport,
}

impl std::fmt::Debug for MpcSentinel2CatalogClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MpcSentinel2CatalogClient")
            .field("provider", &MPC_SENTINEL2_L2A_PROVIDER_ID)
            .field("scope", &"fixed anonymous Sentinel-2 search only")
            .finish()
    }
}

impl MpcSentinel2CatalogClient {
    pub fn new(network_policy: ProviderNetworkPolicy) -> ProviderResult<Self> {
        let route = StacRouteProfile {
            provider_id: MPC_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
            endpoint: MPC_SENTINEL2_STAC_ENDPOINT.to_owned(),
            search_path: "search".to_owned(),
            page_cap: 250,
            accepted_stac_versions: vec!["1.0.0".to_owned(), "1.1.0".to_owned()],
        };
        let binding = StacCollectionBinding {
            logical_dataset: "optical.sentinel-2.l2a".to_owned(),
            provider_id: MPC_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
            native_collection_id: MPC_SENTINEL2_L2A_COLLECTION_ID.to_owned(),
            collection_profile_id: MPC_SENTINEL2_L2A_PROFILE_ID.to_owned(),
            identity_rule_id: MPC_SENTINEL2_L2A_IDENTITY_RULE_ID.to_owned(),
            filename_rule_id: MPC_SENTINEL2_L2A_FILENAME_RULE_ID.to_owned(),
            delivery_model: CatalogDeliveryModel::ArchiveFirst,
            source_kind: DataSourceKind::Sentinel2,
            product_kind: ProductKind::OpticalScene,
            production_enabled: true,
            product_id_property: None,
            display_name_property: Some("s2:product_uri".to_owned()),
            queryables: vec![
                StacQueryableBinding {
                    sceneport_filter_key: "cloud_cover_lte".to_owned(),
                    stac_property: "eo:cloud_cover".to_owned(),
                    operator: StacQueryOperator::Lte,
                    value_kind: StacQueryValueKind::Number,
                },
                StacQueryableBinding {
                    sceneport_filter_key: "spacecraft".to_owned(),
                    stac_property: "platform".to_owned(),
                    operator: StacQueryOperator::In,
                    value_kind: StacQueryValueKind::StringList,
                },
            ],
            asset_role_bindings: Vec::new(),
        };
        let socket = Url::parse(MPC_SENTINEL2_STAC_ENDPOINT)
            .expect("fixed Planetary Computer Sentinel-2 URL is valid");
        let limits = StacTransportLimits {
            request_timeout: Duration::from_secs(20),
            overall_timeout: Duration::from_secs(45),
            max_pages: 128,
            max_records: insar_core::SEARCH_SAFETY_CAP as usize,
            max_page_bytes: 8 * 1024 * 1024,
            max_total_bytes: 48 * 1024 * 1024,
        };
        let mut transport =
            StacTransport::new_with_network_policy(route, binding, socket, limits, network_policy)?;
        transport.excluded_item_fields = vec![
            "assets".to_owned(),
            "links".to_owned(),
            "stac_extensions".to_owned(),
        ];
        Ok(Self { transport })
    }

    pub fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        let normalized = normalize_query(query)?;
        let (request_query, exact_aoi) = super::stac::compact_complex_request_aoi(&normalized)?;
        let spacecraft = decode_sentinel2_spacecraft(query.filters.get("spacecraft"))
            .map_err(|reason| error("STAC_QUERY_INVALID", reason))?;
        let cloud_limit = normalized
            .filters
            .get("cloud_cover_lte")
            .and_then(Value::as_f64);
        let direct_spacecraft = spacecraft.clone();
        let mut budget = self.transport.search_budget();
        match self.transport.search_with_mapper_exact_in_budget(
            &request_query,
            cancelled,
            &mut budget,
            exact_aoi.as_ref(),
            move |feature, product| {
                map_search_feature(feature, product, &direct_spacecraft, cloud_limit)
            },
        ) {
            Ok(response) => Ok(response),
            Err(limit)
                if limit.code == "STAC_LIMIT_EXCEEDED"
                    && limit.debug_context.get("reason").and_then(Value::as_str)
                        == Some("request_body") =>
            {
                let Some(aoi) = normalized
                    .aoi_geojson
                    .as_ref()
                    .and_then(authoritative_catalogue_geometry)
                else {
                    return Err(limit);
                };
                self.search_spatial_fallback(
                    &normalized,
                    &aoi,
                    &spacecraft,
                    cloud_limit,
                    cancelled,
                    &mut budget,
                )
            }
            Err(error) => Err(error),
        }
    }

    /// Progressive page-by-page STAC search for Microsoft Planetary Computer
    /// Sentinel-2 L2A. Single-part AOIs delegate to `StacTransport`; multi-part
    /// spatial-fallback AOIs fail closed with `streaming_cap_order` so
    /// `dispatcher` retries on whole-response `search_catalog`.
    pub fn search_catalog_streaming<S>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut S,
    ) -> ProviderResult<(insar_core::CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
    {
        let query = &insar_core::clamp_catalogue_search_query(query);
        let normalized = normalize_query(query)?;
        let (request_query, exact_aoi) = super::stac::compact_complex_request_aoi(&normalized)?;
        let spacecraft = decode_sentinel2_spacecraft(query.filters.get("spacecraft"))
            .map_err(|reason| error("STAC_QUERY_INVALID", reason))?;
        let cloud_limit = normalized
            .filters
            .get("cloud_cover_lte")
            .and_then(Value::as_f64);
        let mut budget = self.transport.search_budget();
        match self.transport.search_streaming_exact(
            &request_query,
            cancelled,
            &mut budget,
            exact_aoi.as_ref(),
            move |feature, product| map_search_feature(feature, product, &spacecraft, cloud_limit),
            sink,
        ) {
            Ok(result) => Ok(result),
            Err(limit)
                if limit.code == "STAC_LIMIT_EXCEEDED"
                    && limit.debug_context.get("reason").and_then(Value::as_str)
                        == Some("request_body") =>
            {
                Err(error("STAC_RESPONSE_INVALID", "streaming_cap_order"))
            }
            Err(error) => Err(error),
        }
    }

    fn search_spatial_fallback(
        &self,
        query: &CatalogSearchQuery,
        canonical_aoi: &geo::MultiPolygon<f64>,
        spacecraft: &[Sentinel2Platform],
        cloud_limit: Option<f64>,
        cancelled: &(dyn Fn() -> bool + Sync),
        budget: &mut StacSearchBudget,
    ) -> ProviderResult<CatalogSearchResponse> {
        let target = query
            .result_limit
            .filter(|target| *target > 0 && *target <= MPC_SENTINEL2_SPATIAL_RECORD_BUDGET)
            .ok_or_else(|| limit_error("spatial_target"))?;
        let tiles = bounded_catalogue_request_tiles(
            canonical_aoi,
            MPC_SENTINEL2_SPATIAL_LONGITUDE_SPAN,
            MPC_SENTINEL2_SPATIAL_LATITUDE_SPAN,
            MPC_SENTINEL2_SPATIAL_MAX_PARTS,
        )
        .ok_or_else(|| limit_error("spatial_plan"))?;
        let prepared_aoi = PreparedCatalogueAoi::new(canonical_aoi);
        let tiles = order_request_tiles_by_exact_overlap(prepared_aoi.as_ref(), tiles);
        if tiles.is_empty() {
            return Err(limit_error("spatial_plan"));
        }
        let per_part_limit = target;
        let started = Instant::now();
        let mut collection = None;
        let mut unique = BTreeMap::<String, ProviderProduct>::new();
        let mut exact = BTreeMap::<String, ProviderProduct>::new();
        let mut fetched = 0_u64;
        let mut mapped = 0_u64;
        let mut request_count = 0_u64;
        let mut transport_part_count = 0_u64;
        let mut all_attempted_parts_exhausted = true;
        let mut attempted_parts = 0_usize;

        for tile in &tiles {
            budget.check_stop(cancelled)?;
            let mut part = query.clone();
            part.aoi_geojson = Some(tile.clone());
            part.result_limit = Some(per_part_limit);
            let response = self.transport.search_with_mapper_in_budget(
                &part,
                cancelled,
                budget,
                |feature, product| {
                    let product = map_search_feature(feature, product, spacecraft, cloud_limit)?;
                    // Observe every mapped record before the transport truncates
                    // its per-part result. Discarded tail rows are not duplicates.
                    let cloud_ok = !cloud_limit.is_some_and(|limit| {
                        product
                            .properties
                            .get("cloud_cover")
                            .and_then(Value::as_f64)
                            .is_some_and(|cloud| cloud > limit)
                    });
                    admit_spatial_product_with_cloud_ok(
                        &mut unique,
                        &mut exact,
                        &product,
                        prepared_aoi.as_ref(),
                        cloud_ok,
                    )?;
                    Ok(product)
                },
            )?;
            attempted_parts += 1;
            request_count = request_count.saturating_add(response.stats.request_count);
            transport_part_count =
                transport_part_count.saturating_add(response.stats.transport_part_count);
            fetched = fetched.saturating_add(response.stats.fetched_record_count);
            mapped = mapped.saturating_add(response.stats.mapped_record_count);
            if fetched > MPC_SENTINEL2_SPATIAL_RECORD_BUDGET
                || mapped > MPC_SENTINEL2_SPATIAL_RECORD_BUDGET
            {
                return Err(limit_error("spatial_record_budget"));
            }
            all_attempted_parts_exhausted &= response.stats.remote_exhausted;
            if let Some(existing) = &collection {
                if existing != &response.collection {
                    return Err(error("STAC_RESPONSE_INVALID", "spatial_collection"));
                }
            } else {
                collection = Some(response.collection.clone());
            }
            if exact.len() as u64 >= target {
                break;
            }
        }

        budget.check_stop(cancelled)?;
        let exact_matched_count = exact.len() as u64;
        let mut products = exact.into_values().collect::<Vec<_>>();
        products.truncate(target as usize);
        let returned_count = products.len() as u64;
        let attempted_all_tiles = attempted_parts == tiles.len();
        let remote_exhausted = attempted_all_tiles && all_attempted_parts_exhausted;
        if returned_count < target && !remote_exhausted {
            return Err(limit_error("spatial_record_budget"));
        }
        let unique_provider_product_count = unique.len() as u64;
        let exact_duplicate_record_count = mapped
            .checked_sub(unique_provider_product_count)
            .ok_or_else(|| error("STAC_RESPONSE_INVALID", "spatial_counts"))?;
        let response = CatalogSearchResponse {
            collection: collection
                .ok_or_else(|| error("STAC_RESPONSE_INVALID", "spatial_empty"))?,
            products,
            stats: CatalogSearchStats {
                requested_limit: query.result_limit,
                matched_count: None,
                request_count,
                transport_part_count,
                fetched_record_count: fetched,
                mapped_record_count: mapped,
                profile_filtered_record_count: 0,
                exact_duplicate_record_count,
                unique_provider_product_count,
                exact_matched_count,
                returned_count,
                remote_exhausted,
                working_set_satisfied: returned_count == target || remote_exhausted,
                elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                truncated: false,
            },
            next_cursor: None,
            item_errors: Vec::new(),
        };
        response
            .validate()
            .map_err(|_| error("STAC_RESPONSE_INVALID", "spatial_catalogue_contract"))?;
        Ok(response)
    }

    /// A matching name recipe is a lookup candidate, not proof of equivalence.
    /// The remote product URI, including its processing baseline, must match.
    pub fn resolve_map_item_id(
        &self,
        safe_name: &str,
        acquisition_datetime: Option<&str>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<String> {
        let item_id = map_lookup_candidate(safe_name, acquisition_datetime)?;
        let (feature, product) = self.transport.read_exact_item(&item_id, cancelled)?;
        let product = map_feature(&feature, product, None)?;
        validate_map_item_evidence(&feature, &item_id, safe_name)?;
        validate_mpc_sentinel2_source_id(&product.product_id, safe_name, acquisition_datetime)?;
        Ok(item_id)
    }

    pub fn resolve_analysis_band_manifest(
        &self,
        safe_name: &str,
        acquisition_datetime: Option<&str>,
        asset_keys: &[String],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<MpcSentinel2BandManifest> {
        validate_analysis_band_selection(asset_keys)?;
        let item_id = map_lookup_candidate(safe_name, acquisition_datetime)?;
        let (feature, product) = self.transport.read_exact_item(&item_id, cancelled)?;
        let product = map_feature(&feature, product, None)?;
        validate_map_item_evidence(&feature, &item_id, safe_name)?;
        validate_mpc_sentinel2_source_id(&product.product_id, safe_name, acquisition_datetime)?;
        let assets = project_analysis_band_assets(&feature, &item_id, asset_keys)?;
        Ok(MpcSentinel2BandManifest {
            item_id,
            safe_name: safe_name.to_owned(),
            assets,
        })
    }
}

fn validate_analysis_band_selection(asset_keys: &[String]) -> ProviderResult<()> {
    if asset_keys.is_empty() || asset_keys.len() > 12 {
        return Err(error("STAC_QUERY_INVALID", "analysis_band_count"));
    }
    let mut seen = BTreeSet::new();
    for key in asset_keys {
        if sentinel2_l2a_reflectance_band_resolution(key).is_none() || !seen.insert(key.as_str()) {
            return Err(error("STAC_QUERY_INVALID", "analysis_band_selection"));
        }
    }
    Ok(())
}

fn project_analysis_band_assets(
    feature: &Value,
    item_id: &str,
    asset_keys: &[String],
) -> ProviderResult<Vec<MpcSentinel2BandAsset>> {
    let assets = feature
        .get("assets")
        .and_then(Value::as_object)
        .ok_or_else(|| error("STAC_RESPONSE_INVALID", "analysis_assets"))?;
    let mut projected = Vec::with_capacity(asset_keys.len());
    for key in asset_keys {
        let resolution = sentinel2_l2a_reflectance_band_resolution(key)
            .ok_or_else(|| error("STAC_QUERY_INVALID", "analysis_band_key"))?;
        let asset = assets
            .get(key)
            .and_then(Value::as_object)
            .ok_or_else(|| error("STAC_ITEM_NOT_FOUND", "analysis_band_missing"))?;
        let href = asset
            .get("href")
            .and_then(Value::as_str)
            .ok_or_else(|| error("STAC_RESPONSE_INVALID", "analysis_band_href"))?;
        let media_type = asset
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| error("STAC_RESPONSE_INVALID", "analysis_band_type"))?;
        let roles = asset
            .get("roles")
            .and_then(Value::as_array)
            .ok_or_else(|| error("STAC_RESPONSE_INVALID", "analysis_band_roles"))?;
        let gsd = asset
            .get("gsd")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .ok_or_else(|| error("STAC_RESPONSE_INVALID", "analysis_band_gsd"))?;
        if !media_type.starts_with("image/tiff")
            || !roles.iter().any(|role| role.as_str() == Some("data"))
            || (gsd - f64::from(resolution)).abs() > f64::EPSILON
        {
            return Err(error("STAC_RESPONSE_INVALID", "analysis_band_semantics"));
        }
        let file_name = validate_planetary_computer_reviewed_asset_identity(
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
            item_id,
            key,
            href,
        )
        .map_err(|_| error("STAC_IDENTITY_MISMATCH", "analysis_band_identity"))?;
        projected.push(MpcSentinel2BandAsset {
            asset_key: key.clone(),
            stable_href: href.to_owned(),
            file_name,
            resolution_m: resolution,
        });
    }
    Ok(projected)
}

fn map_lookup_candidate(safe_name: &str, acquired: Option<&str>) -> ProviderResult<String> {
    let parts = safe_name
        .strip_suffix(".SAFE")
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "safe_suffix"))?
        .split('_')
        .collect::<Vec<_>>();
    if parts.len() != 7 {
        return Err(error("STAC_IDENTITY_MISMATCH", "safe_name"));
    }
    let item_id = [parts[0], parts[1], parts[2], parts[4], parts[5], parts[6]].join("_");
    validate_mpc_sentinel2_source_id(&item_id, safe_name, acquired)?;
    Ok(item_id)
}

fn validate_map_item_evidence(
    feature: &Value,
    item_id: &str,
    safe_name: &str,
) -> ProviderResult<()> {
    if feature.get("id").and_then(Value::as_str) != Some(item_id)
        || feature.get("collection").and_then(Value::as_str)
            != Some(MPC_SENTINEL2_L2A_COLLECTION_ID)
        || feature
            .pointer("/properties/s2:product_uri")
            .and_then(Value::as_str)
            != Some(safe_name)
    {
        return Err(error("STAC_IDENTITY_MISMATCH", "map_safe_identity"));
    }
    let visual = feature
        .pointer("/assets/visual")
        .and_then(Value::as_object)
        .ok_or_else(|| error("STAC_ITEM_NOT_FOUND", "map_visual_missing"))?;
    if !visual
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|value| value.starts_with("image/tiff"))
        || !visual
            .get("roles")
            .and_then(Value::as_array)
            .is_some_and(|roles| roles.iter().any(|role| role.as_str() == Some("data")))
        || !visual
            .get("eo:bands")
            .and_then(Value::as_array)
            .is_some_and(|bands| {
                bands.len() == 3
                    && bands
                        .iter()
                        .zip(["red", "green", "blue"])
                        .all(|(band, name)| {
                            band.get("common_name").and_then(Value::as_str) == Some(name)
                        })
            })
    {
        return Err(error("STAC_RESPONSE_INVALID", "map_visual_semantics"));
    }
    Ok(())
}

fn normalize_query(query: &CatalogSearchQuery) -> ProviderResult<CatalogSearchQuery> {
    if query.logical_dataset != "optical.sentinel-2.l2a"
        || query.provider_id != MPC_SENTINEL2_L2A_PROVIDER_ID
        || query.native_collection_id != MPC_SENTINEL2_L2A_COLLECTION_ID
        || query.cursor.is_some()
    {
        return Err(error("STAC_IDENTITY_MISMATCH", "query_source"));
    }
    let mut normalized = query.clone();
    let product_type = normalized.filters.remove("product_type");
    if product_type.as_ref().and_then(Value::as_str) != Some("S2MSI2A") {
        return Err(error("STAC_QUERY_INVALID", "product_type"));
    }
    for key in normalized.filters.keys() {
        if !matches!(key.as_str(), "cloud_cover_lte" | "datetime" | "spacecraft") {
            return Err(error("STAC_QUERY_INVALID", "unsupported_filter"));
        }
    }
    if normalized
        .filters
        .get("cloud_cover_lte")
        .is_some_and(|value| {
            value
                .as_f64()
                .is_none_or(|cloud| !cloud.is_finite() || !(0.0..=100.0).contains(&cloud))
        })
    {
        return Err(error("STAC_QUERY_INVALID", "cloud_cover_lte"));
    }
    let spacecraft = decode_sentinel2_spacecraft(normalized.filters.get("spacecraft"))
        .map_err(|reason| error("STAC_QUERY_INVALID", reason))?;
    if !spacecraft.is_empty() {
        normalized.filters.insert(
            "spacecraft".to_owned(),
            Value::Array(
                spacecraft
                    .iter()
                    .map(|p| Value::String(p.stac_name.to_owned()))
                    .collect(),
            ),
        );
    }
    Ok(normalized)
}

fn map_search_feature(
    feature: &Value,
    product: ProviderProduct,
    spacecraft: &[Sentinel2Platform],
    cloud_limit: Option<f64>,
) -> ProviderResult<ProviderProduct> {
    if !spacecraft.is_empty()
        && !spacecraft
            .iter()
            .any(|platform| product.product_id.starts_with(platform.prefix))
    {
        return Err(error("STAC_IDENTITY_MISMATCH", "spacecraft_scope"));
    }
    map_feature(feature, product, cloud_limit)
}

fn map_feature(
    feature: &Value,
    mut product: ProviderProduct,
    _cloud_limit: Option<f64>,
) -> ProviderResult<ProviderProduct> {
    let properties = feature
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| error("STAC_RESPONSE_INVALID", "properties"))?;
    let item_id = feature
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "item_id"))?;
    let product_uri = properties
        .get("s2:product_uri")
        .and_then(Value::as_str)
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "product_uri"))?;
    let mgrs_tile = properties
        .get("s2:mgrs_tile")
        .and_then(Value::as_str)
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "mgrs_tile"))?;
    let platform = properties
        .get("platform")
        .and_then(Value::as_str)
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "platform"))?;
    let cloud_cover = properties
        .get("eo:cloud_cover")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or_else(|| error("STAC_RESPONSE_INVALID", "cloud_cover"))?;
    validate_identity(
        item_id,
        product_uri,
        mgrs_tile,
        platform,
        product.acquisition_datetime.as_deref(),
    )?;
    if product.product_id != item_id
        || product.display_name != product_uri
        || !product.assets.is_empty()
    {
        return Err(error("STAC_IDENTITY_MISMATCH", "normalized_product"));
    }
    product.properties = Map::from_iter([
        (
            "product_type".to_owned(),
            Value::String("S2MSI2A".to_owned()),
        ),
        (
            "cloud_cover".to_owned(),
            serde_json::Number::from_f64(cloud_cover)
                .map(Value::Number)
                .ok_or_else(|| error("STAC_RESPONSE_INVALID", "cloud_cover"))?,
        ),
    ]);
    product.payload = Map::new();
    product
        .validate()
        .map_err(|_| error("STAC_RESPONSE_INVALID", "normalized_product"))?;
    Ok(product)
}

fn validate_identity(
    item_id: &str,
    product_uri: &str,
    mgrs_tile: &str,
    platform: &str,
    acquisition_datetime: Option<&str>,
) -> ProviderResult<()> {
    let item = item_id.split('_').collect::<Vec<_>>();
    let safe = product_uri
        .strip_suffix(".SAFE")
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "safe_suffix"))?
        .split('_')
        .collect::<Vec<_>>();
    if item.len() != 6
        || safe.len() != 7
        || item[1] != "MSIL2A"
        || safe[1] != "MSIL2A"
        || item[0] != safe[0]
        || item[2] != safe[2]
        || item[3] != safe[4]
        || item[4] != safe[5]
        || item[5] != safe[6]
        || !valid_datetime_token(item[2])
        || !valid_datetime_token(item[5])
        || !safe[3].strip_prefix('N').is_some_and(four_digits)
        || !item[3].strip_prefix('R').is_some_and(three_digits)
        || !item[4].strip_prefix('T').is_some_and(valid_mgrs_tile)
        || item[4].strip_prefix('T') != Some(mgrs_tile)
    {
        return Err(error("STAC_IDENTITY_MISMATCH", "sentinel2_identity"));
    }
    let expected_platform = match item[0] {
        "S2A" => "Sentinel-2A",
        "S2B" => "Sentinel-2B",
        "S2C" => "Sentinel-2C",
        _ => return Err(error("STAC_IDENTITY_MISMATCH", "platform_code")),
    };
    if platform != expected_platform {
        return Err(error("STAC_IDENTITY_MISMATCH", "platform"));
    }
    let expected_second = format!(
        "{}-{}-{}T{}:{}:{}",
        &item[2][0..4],
        &item[2][4..6],
        &item[2][6..8],
        &item[2][9..11],
        &item[2][11..13],
        &item[2][13..15]
    );
    if !acquisition_datetime.is_some_and(|value| {
        value.starts_with(&expected_second)
            && value
                .as_bytes()
                .get(expected_second.len())
                .is_some_and(|next| matches!(*next, b'.' | b'Z' | b'+'))
    }) {
        return Err(error("STAC_IDENTITY_MISMATCH", "acquisition_datetime"));
    }
    Ok(())
}

/// Revalidate the self-contained source identity that crosses the Provider →
/// Application boundary. Provider-only STAC properties (platform/MGRS) are not
/// trusted after mapping; the item/S.A.F.E. names must prove the same values.
pub fn validate_mpc_sentinel2_source_id(
    item_id: &str,
    product_uri: &str,
    acquisition_datetime: Option<&str>,
) -> ProviderResult<()> {
    let item = item_id.split('_').collect::<Vec<_>>();
    let mgrs_tile = item
        .get(4)
        .and_then(|value| value.strip_prefix('T'))
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "mgrs_tile"))?;
    let platform = match item.first().copied() {
        Some("S2A") => "Sentinel-2A",
        Some("S2B") => "Sentinel-2B",
        Some("S2C") => "Sentinel-2C",
        _ => return Err(error("STAC_IDENTITY_MISMATCH", "platform_code")),
    };
    validate_identity(
        item_id,
        product_uri,
        mgrs_tile,
        platform,
        acquisition_datetime,
    )
}

fn valid_datetime_token(value: &str) -> bool {
    value.len() == 15
        && value.as_bytes().get(8) == Some(&b'T')
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 8 || byte.is_ascii_digit())
}

fn four_digits(value: &str) -> bool {
    value.len() == 4 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn three_digits(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_mgrs_tile(value: &str) -> bool {
    value.len() == 5
        && value.as_bytes()[0..2]
            .iter()
            .all(|byte| byte.is_ascii_digit())
        && value.as_bytes()[2..5]
            .iter()
            .all(|byte| byte.is_ascii_uppercase())
}

fn limit_error(reason: &str) -> ContractError {
    ContractError::new(
        "STAC_LIMIT_EXCEEDED",
        "目录检索超过安全限制",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded Sentinel-2 limit error is safe")
}

#[cfg(test)]
fn admit_spatial_product(
    unique: &mut BTreeMap<String, ProviderProduct>,
    exact: &mut BTreeMap<String, ProviderProduct>,
    product: &ProviderProduct,
    prepared_aoi: Option<&PreparedCatalogueAoi>,
) -> ProviderResult<()> {
    admit_spatial_product_with_cloud_ok(unique, exact, product, prepared_aoi, true)
}

fn admit_spatial_product_with_cloud_ok(
    unique: &mut BTreeMap<String, ProviderProduct>,
    exact: &mut BTreeMap<String, ProviderProduct>,
    product: &ProviderProduct,
    prepared_aoi: Option<&PreparedCatalogueAoi>,
    cloud_ok: bool,
) -> ProviderResult<()> {
    if let Some(existing) = unique.get(&product.product_id) {
        if existing != product {
            return Err(error("STAC_IDENTITY_MISMATCH", "spatial_product_conflict"));
        }
        return Ok(());
    }
    unique.insert(product.product_id.clone(), product.clone());
    if cloud_ok
        && product.footprint_geojson.as_ref().is_some_and(|footprint| {
            prepared_aoi.is_some_and(|prepared| prepared.intersects_footprint(footprint))
        })
    {
        exact.insert(product.product_id.clone(), product.clone());
    }
    Ok(())
}

fn error(code: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        "Microsoft Sentinel-2 STAC 证据无效",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded Sentinel-2 STAC error is safe")
}

#[cfg(test)]
mod tests {
    use super::super::stac_test_server::JsonPageServer;
    use super::*;
    use serde_json::json;

    fn dense_inner_mongolia_rectangle() -> Map<String, Value> {
        let west = 97.0;
        let east = 126.0;
        let south = 37.0;
        let north = 53.0;
        let segments = 720_u32;
        let mut ring = Vec::with_capacity((segments as usize) * 4 + 1);
        for index in 0..segments {
            let ratio = f64::from(index) / f64::from(segments);
            ring.push(json!([west + (east - west) * ratio, south]));
        }
        for index in 0..segments {
            let ratio = f64::from(index) / f64::from(segments);
            ring.push(json!([east, south + (north - south) * ratio]));
        }
        for index in 0..segments {
            let ratio = f64::from(index) / f64::from(segments);
            ring.push(json!([east - (east - west) * ratio, north]));
        }
        for index in 0..segments {
            let ratio = f64::from(index) / f64::from(segments);
            ring.push(json!([west, north - (north - south) * ratio]));
        }
        ring.push(ring[0].clone());
        json!({"type":"Polygon","coordinates":[ring]})
            .as_object()
            .unwrap()
            .clone()
    }

    fn dense_inner_mongolia_query() -> CatalogSearchQuery {
        serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": MPC_SENTINEL2_L2A_PROVIDER_ID,
            "native_collection_id": MPC_SENTINEL2_L2A_COLLECTION_ID,
            "aoi_geojson": dense_inner_mongolia_rectangle(),
            "cursor": null,
            "result_limit": 100,
            "filters": {"product_type":"S2MSI2A"}
        }))
        .unwrap()
    }

    /// Generated Sentinel-2 item whose identity tokens validate; `index`
    /// varies only the processing time so every item ID is distinct.
    fn s2_feature(index: usize, inside_aoi: bool) -> Value {
        let processed = format!(
            "20250114T{:02}{:02}{:02}",
            index / 3600,
            (index / 60) % 60,
            index % 60
        );
        let (west, south) = if inside_aoi {
            (100.0, 40.0)
        } else {
            (0.0, 0.0)
        };
        json!({
            "type": "Feature", "stac_version": "1.1.0",
            "id": format!("S2A_MSIL2A_20250114T023041_R046_T51RUH_{processed}"),
            "collection": MPC_SENTINEL2_L2A_COLLECTION_ID,
            "geometry": {"type": "Polygon", "coordinates": [[
                [west, south], [west + 1.0, south], [west + 1.0, south + 1.0],
                [west, south + 1.0], [west, south]
            ]]},
            "properties": {
                "datetime": "2025-01-14T02:30:41Z",
                "s2:product_uri": format!("S2A_MSIL2A_20250114T023041_N0511_R046_T51RUH_{processed}.SAFE"),
                "s2:mgrs_tile": "51RUH", "platform": "Sentinel-2A", "eo:cloud_cover": 10.0
            },
            "assets": {}, "links": []
        })
    }

    fn s2_page(features: Vec<Value>, next: bool) -> Value {
        let links = if next {
            vec![json!({"rel": "next", "href": "/api/stac/v1/search?token=next"})]
        } else {
            Vec::new()
        };
        json!({"type": "FeatureCollection", "numberReturned": features.len(),
            "features": features, "links": links})
    }

    fn loopback_client(server: &JsonPageServer, max_records: usize) -> MpcSentinel2CatalogClient {
        let production = MpcSentinel2CatalogClient::new(ProviderNetworkPolicy::direct()).unwrap();
        // The production limits, with a scaled-down shared record budget.
        let limits = StacTransportLimits {
            request_timeout: Duration::from_secs(10),
            overall_timeout: Duration::from_secs(30),
            max_pages: 128,
            max_records,
            max_page_bytes: 2 * 1024 * 1024,
            max_total_bytes: 8 * 1024 * 1024,
        };
        MpcSentinel2CatalogClient {
            transport: production
                .transport
                .loopback_for_test(&server.origin, limits)
                .unwrap(),
        }
    }

    fn spatial_fallback(
        client: &MpcSentinel2CatalogClient,
        target: u64,
    ) -> ProviderResult<CatalogSearchResponse> {
        let mut query = normalize_query(&dense_inner_mongolia_query()).unwrap();
        query.result_limit = Some(target);
        let aoi = authoritative_catalogue_geometry(query.aoi_geojson.as_ref().unwrap()).unwrap();
        let mut budget = client.transport.search_budget();
        client.search_spatial_fallback(&query, &aoi, &[], None, &|| false, &mut budget)
    }

    /// Slice 3(a): the spatial shards share one record budget, and the first
    /// shard may consume all of it. That is only acceptable with a truthful
    /// result: N kept with remote exhaustion unproven (the frozen batch is
    /// then possibly incomplete), or a typed budget failure — never a
    /// silently short or falsely complete batch.
    #[test]
    fn one_spatial_shard_consuming_the_shared_budget_is_reported_truthfully() {
        const TARGET: usize = 4;
        // The first shard fills the target and still has a continuation.
        let server = JsonPageServer::new(vec![s2_page(
            (0..TARGET).map(|index| s2_feature(index, true)).collect(),
            true,
        )]);
        let response = spatial_fallback(&loopback_client(&server, TARGET), TARGET as u64).unwrap();
        assert_eq!(server.count(), 1, "later shards are not queried");
        assert_eq!(response.products.len(), TARGET);
        assert_eq!(response.stats.requested_limit, Some(TARGET as u64));
        assert!(!response.stats.remote_exhausted);
        assert!(response.stats.transport_part_count < 2);

        // The first shard consumes the shared budget but only half of it is
        // inside the AOI: the next shard cannot run, so the search fails
        // with a typed budget error instead of freezing a short batch.
        let server = JsonPageServer::new(vec![
            s2_page(
                (0..TARGET)
                    .map(|index| s2_feature(index, index % 2 == 0))
                    .collect(),
                true,
            ),
            s2_page(vec![s2_feature(TARGET, true)], false),
        ]);
        let error = spatial_fallback(&loopback_client(&server, TARGET), TARGET as u64).unwrap_err();
        assert_eq!(error.code, "STAC_LIMIT_EXCEEDED");
        assert_eq!(error.debug_context["reason"], "records");

        // Exactly N exist across every shard and each shard is exhausted:
        // complete, so no incompleteness may be reported.
        let tiles = bounded_catalogue_request_tiles(
            &authoritative_catalogue_geometry(&dense_inner_mongolia_rectangle()).unwrap(),
            MPC_SENTINEL2_SPATIAL_LONGITUDE_SPAN,
            MPC_SENTINEL2_SPATIAL_LATITUDE_SPAN,
            MPC_SENTINEL2_SPATIAL_MAX_PARTS,
        )
        .unwrap();
        let mut pages = vec![s2_page(
            (0..TARGET - 1)
                .map(|index| s2_feature(index, true))
                .collect(),
            false,
        )];
        pages.push(s2_page(vec![s2_feature(TARGET - 1, true)], false));
        pages.extend((2..tiles.len()).map(|_| s2_page(Vec::new(), false)));
        let server = JsonPageServer::new(pages);
        let response = spatial_fallback(&loopback_client(&server, TARGET), TARGET as u64).unwrap();
        assert_eq!(
            server.count(),
            2,
            "the target is met after the second shard"
        );
        assert_eq!(response.products.len(), TARGET);
        // Stopped by the target before any remaining shard: not proven.
        assert_eq!(response.stats.remote_exhausted, tiles.len() == 2);
        let server = JsonPageServer::new(pages_all_exhausted(tiles.len(), TARGET));
        let response =
            spatial_fallback(&loopback_client(&server, TARGET + 1), TARGET as u64 + 1).unwrap();
        assert_eq!(server.count(), tiles.len());
        assert_eq!(response.products.len(), TARGET);
        assert!(response.stats.remote_exhausted);
    }

    fn pages_all_exhausted(tiles: usize, total: usize) -> Vec<Value> {
        let mut pages = vec![s2_page(
            (0..total).map(|index| s2_feature(index, true)).collect(),
            false,
        )];
        pages.extend((1..tiles).map(|_| s2_page(Vec::new(), false)));
        pages
    }

    #[test]
    fn spatial_search_budget_allows_the_full_safety_cap() {
        assert!(
            std::hint::black_box(MPC_SENTINEL2_SPATIAL_RECORD_BUDGET)
                >= insar_core::SEARCH_SAFETY_CAP
        );
    }

    #[test]
    fn high_detail_inner_mongolia_aoi_has_small_bounded_fallback_tiles() {
        let query = normalize_query(&dense_inner_mongolia_query()).unwrap();
        let aoi = query.aoi_geojson.as_ref().unwrap();
        assert!(
            serde_json::to_vec(aoi).unwrap().len() > 32 * 1024,
            "fixture must reproduce the STAC request-body safety pressure"
        );
        let canonical = authoritative_catalogue_geometry(aoi).unwrap();
        let tiles = bounded_catalogue_request_tiles(
            &canonical,
            MPC_SENTINEL2_SPATIAL_LONGITUDE_SPAN,
            MPC_SENTINEL2_SPATIAL_LATITUDE_SPAN,
            MPC_SENTINEL2_SPATIAL_MAX_PARTS,
        )
        .unwrap();
        assert!((2..=MPC_SENTINEL2_SPATIAL_MAX_PARTS).contains(&tiles.len()));
        assert!(tiles
            .iter()
            .all(|tile| serde_json::to_vec(tile).unwrap().len() < 1024));
    }

    #[test]
    fn spatial_duplicates_require_identical_scientific_evidence() {
        let geometry = json!({"type":"Polygon","coordinates":[[[100.0,40.0],[101.0,40.0],[101.0,41.0],[100.0,41.0],[100.0,40.0]]]});
        let aoi = authoritative_catalogue_geometry(geometry.as_object().unwrap()).unwrap();
        let prepared_aoi = PreparedCatalogueAoi::new(&aoi);
        let product = ProviderProduct {
            provider_id: MPC_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
            product_id: "S2A_MSIL2A_20250114T023041_R046_T51RUH_20250114T061830".to_owned(),
            display_name: "S2A_MSIL2A_20250114T023041_N0511_R046_T51RUH_20250114T061830.SAFE"
                .to_owned(),
            source_kind: DataSourceKind::Sentinel2,
            product_kind: ProductKind::OpticalScene,
            acquisition_datetime: Some("2025-01-14T02:30:41Z".to_owned()),
            footprint_geojson: Some(geometry.as_object().unwrap().clone()),
            footprint_bbox: None,
            assets: Vec::new(),
            properties: Map::from_iter([("cloud_cover".to_owned(), json!(10.0))]),
            payload: Map::new(),
        };
        let mut unique = BTreeMap::new();
        let mut exact = BTreeMap::new();
        admit_spatial_product(&mut unique, &mut exact, &product, prepared_aoi.as_ref()).unwrap();
        admit_spatial_product(&mut unique, &mut exact, &product, prepared_aoi.as_ref()).unwrap();
        assert_eq!((unique.len(), exact.len()), (1, 1));
        for conflict in 0..3 {
            let mut changed = product.clone();
            match conflict {
                0 => changed
                    .properties
                    .insert("cloud_cover".to_owned(), json!(20.0)),
                1 => {
                    changed.display_name = changed.display_name.replace("N0511", "N0500");
                    None
                }
                _ => {
                    changed.footprint_geojson = None;
                    None
                }
            };
            let error =
                admit_spatial_product(&mut unique, &mut exact, &changed, prepared_aoi.as_ref())
                    .unwrap_err();
            assert_eq!(error.code, "STAC_IDENTITY_MISMATCH");
            assert_eq!(error.debug_context["reason"], "spatial_product_conflict");
            assert_eq!(unique.get(&product.product_id), Some(&product));
            assert_eq!(exact.get(&product.product_id), Some(&product));
        }
    }

    #[test]
    #[ignore = "requires EOSTAGE_OA01_S2_LIVE=1; anonymous public MPC search only"]
    fn public_high_detail_inner_mongolia_search_uses_bounded_fallback() {
        assert_eq!(std::env::var("EOSTAGE_OA01_S2_LIVE").as_deref(), Ok("1"));
        let client = MpcSentinel2CatalogClient::new(ProviderNetworkPolicy::direct()).unwrap();
        let response = client
            .search_catalog(&dense_inner_mongolia_query(), &|| false)
            .unwrap();
        assert_eq!(response.products.len(), 100);
        assert_eq!(response.stats.requested_limit, Some(100));
        assert!(response.stats.request_count >= 1);
        assert!(response.stats.fetched_record_count <= MPC_SENTINEL2_SPATIAL_RECORD_BUDGET);
        assert!(response.stats.working_set_satisfied);
        assert!(!response.stats.truncated);
    }

    #[test]
    fn map_preview_requires_remote_safe_identity_not_just_a_matching_item_recipe() {
        let safe_name = "S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE";
        let id = map_lookup_candidate(safe_name, Some("2026-07-06T03:25:11Z")).unwrap();
        let mut feature = json!({"id":id,"collection":"sentinel-2-l2a",
            "properties":{"s2:product_uri":safe_name},
            "assets":{"visual":{"type":"image/tiff; application=geotiff","roles":["data"],
                "eo:bands":[{"common_name":"red"},{"common_name":"green"},{"common_name":"blue"}]}}});
        validate_map_item_evidence(&feature, &id, safe_name).unwrap();
        feature["properties"]["s2:product_uri"] = json!(safe_name.replace("N0512", "N0500"));
        assert!(validate_map_item_evidence(&feature, &id, safe_name).is_err());
        feature["properties"]["s2:product_uri"] = json!(safe_name);
        feature["assets"]["visual"]["eo:bands"][0]["common_name"] = json!("blue");
        assert!(validate_map_item_evidence(&feature, &id, safe_name).is_err());
        feature["assets"] = json!({});
        assert!(validate_map_item_evidence(&feature, &id, safe_name).is_err());
        assert!(
            map_lookup_candidate("../../untrusted.SAFE", Some("2026-07-06T03:25:11Z")).is_err()
        );
        assert!(map_lookup_candidate(safe_name, Some("2026-07-07T03:25:11Z")).is_err());
    }

    #[test]
    fn analysis_band_projection_is_closed_to_reviewed_reflectance_assets() {
        let item_id = "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916";
        let b02 = "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R10m/T48SYC_20260706T032511_B02_10m.tif";
        let b11 = "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R20m/T48SYC_20260706T032511_B11_20m.tif";
        let feature = json!({
            "assets": {
                "B02": {"href":b02,"type":"image/tiff; application=geotiff; profile=cloud-optimized","roles":["data"],"gsd":10},
                "B11": {"href":b11,"type":"image/tiff; application=geotiff; profile=cloud-optimized","roles":["data"],"gsd":20},
                "SCL": {"href":"https://untrusted.invalid/scl.tif","type":"image/tiff","roles":["data"],"gsd":20}
            }
        });
        let keys = vec!["B02".to_owned(), "B11".to_owned()];
        let assets = project_analysis_band_assets(&feature, item_id, &keys).unwrap();
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].resolution_m, 10);
        assert_eq!(assets[1].resolution_m, 20);
        assert!(validate_analysis_band_selection(&keys).is_ok());
        assert!(validate_analysis_band_selection(&["B02".to_owned(), "B02".to_owned()]).is_err());
        assert!(validate_analysis_band_selection(&["SCL".to_owned()]).is_err());

        let mut forged = feature.clone();
        forged["assets"]["B02"]["gsd"] = json!(20);
        assert!(project_analysis_band_assets(&forged, item_id, &["B02".to_owned()]).is_err());
        let mut signed = feature;
        signed["assets"]["B02"]["href"] = json!(format!("{b02}?sig=forged"));
        assert!(project_analysis_band_assets(&signed, item_id, &["B02".to_owned()]).is_err());
    }

    #[test]
    #[ignore = "requires EOSTAGE_P10_S2_MAP_LIVE=1; public exact Item lookup only"]
    fn cdse_safe_resolves_exact_public_map_item() {
        assert_eq!(std::env::var("EOSTAGE_P10_S2_MAP_LIVE").as_deref(), Ok("1"));
        let client = MpcSentinel2CatalogClient::new(ProviderNetworkPolicy::direct()).unwrap();
        assert_eq!(
            client
                .resolve_map_item_id(
                    "S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE",
                    Some("2026-07-06T03:25:11Z"),
                    &|| false
                )
                .unwrap(),
            "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916"
        );
    }

    #[test]
    fn identity_keeps_native_item_and_safe_product_distinct() {
        validate_identity(
            "S2A_MSIL2A_20250114T023041_R046_T51RUH_20250114T061830",
            "S2A_MSIL2A_20250114T023041_N0511_R046_T51RUH_20250114T061830.SAFE",
            "51RUH",
            "Sentinel-2A",
            Some("2025-01-14T02:30:41.024Z"),
        )
        .unwrap();
        assert!(validate_identity(
            "S2A_MSIL2A_20250114T023041_R046_T51RUH_20250114T061830",
            "S2A_MSIL2A_20250114T023041_N0511_R046_T51RUK_20250114T061830.SAFE",
            "51RUH",
            "Sentinel-2A",
            Some("2025-01-14T02:30:41.024Z"),
        )
        .is_err());
        validate_mpc_sentinel2_source_id(
            "S2A_MSIL2A_20250114T023041_R046_T51RUH_20250114T061830",
            "S2A_MSIL2A_20250114T023041_N0511_R046_T51RUH_20250114T061830.SAFE",
            Some("2025-01-14T02:30:41.024Z"),
        )
        .unwrap();
    }

    #[test]
    fn query_scope_is_search_only_and_closed() {
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": MPC_SENTINEL2_L2A_PROVIDER_ID,
            "native_collection_id": MPC_SENTINEL2_L2A_COLLECTION_ID,
            "aoi_geojson":{"type":"Polygon","coordinates":[[[111.0,31.5],[111.2,31.5],[111.2,31.7],[111.0,31.5]]]},
            "cursor":null,
            "result_limit":10,
            "filters":{"product_type":"S2MSI2A","cloud_cover_lte":35.5}
        }))
        .unwrap();
        let normalized = normalize_query(&query).unwrap();
        assert!(!normalized.filters.contains_key("product_type"));
        assert_eq!(normalized.filters["cloud_cover_lte"], 35.5);
        let mut selected = query.clone();
        selected.filters.insert(
            "spacecraft".to_owned(),
            json!(["sentinel_2c", "sentinel_2a"]),
        );
        assert_eq!(
            normalize_query(&selected).unwrap().filters["spacecraft"],
            json!(["Sentinel-2A", "Sentinel-2C"])
        );
        selected
            .filters
            .insert("spacecraft".to_owned(), json!(["landsat_9"]));
        assert!(normalize_query(&selected).is_err());
        let mut foreign = query.clone();
        foreign.provider_id = "copernicus.dataspace.odata".to_owned();
        assert!(normalize_query(&foreign).is_err());
        let mut unknown = query;
        unknown.filters.insert("bands".to_owned(), json!(["B04"]));
        assert!(normalize_query(&unknown).is_err());
    }

    #[test]
    fn disjoint_china_like_spatial_fallback_prunes_empty_gap_tiles() {
        let disjoint_aoi = super::super::spatial::dense_disjoint_china_like_aoi_for_test();
        let canonical = authoritative_catalogue_geometry(&disjoint_aoi).unwrap();
        let raw_tiles = bounded_catalogue_request_tiles(
            &canonical,
            MPC_SENTINEL2_SPATIAL_LONGITUDE_SPAN,
            MPC_SENTINEL2_SPATIAL_LATITUDE_SPAN,
            MPC_SENTINEL2_SPATIAL_MAX_PARTS,
        )
        .unwrap();
        let prepared = PreparedCatalogueAoi::new(&canonical).unwrap();
        let ordered = order_request_tiles_by_exact_overlap(Some(&prepared), raw_tiles.clone());
        assert!(
            ordered.len() < raw_tiles.len() && ordered.len() == 2,
            "disjoint components overlap only 2 of {} grid tiles",
            raw_tiles.len()
        );

        let mut query = dense_inner_mongolia_query();
        query.aoi_geojson = Some(disjoint_aoi);
        query.result_limit = Some(4);
        let normalized = normalize_query(&query).unwrap();

        // Only `ordered.len()` (2) pages are provided; querying any pruned gap tile hits HTTP 500.
        let server =
            JsonPageServer::new(vec![s2_page(Vec::new(), false), s2_page(Vec::new(), false)]);
        let client = loopback_client(&server, 16);
        let mut budget = client.transport.search_budget();
        let response = client
            .search_spatial_fallback(&normalized, &canonical, &[], None, &|| false, &mut budget)
            .unwrap();
        assert_eq!(server.count(), ordered.len());
        assert_eq!(response.stats.transport_part_count, ordered.len() as u64);
        assert!(response.stats.remote_exhausted);
    }

    #[test]
    fn administrative_aoi_compacts_remote_intersects_excludes_heavy_fields_and_filters_locally() {
        // Build a 24-vertex administrative-like concave polygon (< 2 KiB, well
        // below the 24 KiB byte threshold) covering [100..102, 40..42] except
        // for the northeast corner [101.2..102.0, 41.2..42.0].
        let mut ring = Vec::new();
        for step in 0..8 {
            let lon = 100.0 + f64::from(step) * 0.15;
            ring.push(json!([lon, 40.0]));
        }
        ring.push(json!([102.0, 40.0]));
        for step in 1..=6 {
            let lat = 40.0 + f64::from(step) * 0.2;
            ring.push(json!([102.0, lat]));
        }
        // Concave step excluding [101.2..102.0, 41.2..42.0]
        ring.push(json!([101.1, 41.2]));
        for step in 1..=6 {
            let lat = 41.2 + f64::from(step) * (0.8 / 6.0);
            ring.push(json!([101.1, lat]));
        }
        ring.push(json!([100.0, 42.0]));
        ring.push(ring[0].clone());
        assert!(ring.len() > super::super::stac::COMPLEX_REQUEST_AOI_MAX_POSITIONS);

        let aoi = json!({"type": "Polygon", "coordinates": [ring]})
            .as_object()
            .unwrap()
            .clone();
        assert!(serde_json::to_vec(&aoi).unwrap().len() < 2048);

        // Inside feature at [100.0..101.0, 40.0..41.0], with `assets` and `links`
        // omitted from the server response just as STAC `fields.exclude` returns.
        let mut inside = s2_feature(0, true);
        inside.as_object_mut().unwrap().remove("assets");
        inside.as_object_mut().unwrap().remove("links");

        // Outside feature in the northeast bbox corner [101.3..101.9, 41.3..41.9]
        // (inside the [100..102, 40..42] bbox, but outside the concave polygon).
        let mut corner_outside = s2_feature(1, true);
        corner_outside["geometry"] = json!({
            "type": "Polygon",
            "coordinates": [[[101.3, 41.3], [101.9, 41.3], [101.9, 41.9], [101.3, 41.9], [101.3, 41.3]]]
        });
        corner_outside.as_object_mut().unwrap().remove("assets");
        corner_outside.as_object_mut().unwrap().remove("links");

        let server = JsonPageServer::new(vec![s2_page(vec![inside, corner_outside], false)]);
        let client = loopback_client(&server, 250);
        let mut query = dense_inner_mongolia_query();
        query.aoi_geojson = Some(aoi);
        query.result_limit = Some(10);

        let (compact_query, exact) =
            super::super::stac::compact_complex_request_aoi(&query).unwrap();
        assert!(
            exact.is_some(),
            "administrative AOI must carry exact local filter"
        );
        assert_eq!(
            compact_query.aoi_geojson.as_ref().unwrap()["coordinates"][0]
                .as_array()
                .unwrap()
                .len(),
            5,
            "remote STAC request must use 5-position compact bbox envelope"
        );
        assert_eq!(
            client.transport.excluded_item_fields,
            vec![
                "assets".to_owned(),
                "links".to_owned(),
                "stac_extensions".to_owned()
            ]
        );

        let response = client.search_catalog(&query, &|| false).unwrap();
        assert_eq!(server.count(), 1);
        assert_eq!(response.products.len(), 1);
        assert!(response.products[0].assets.is_empty());
        assert!(response.products[0].payload.is_empty());
        assert_eq!(response.stats.fetched_record_count, 2);
        assert_eq!(response.stats.exact_matched_count, 1);
        assert_eq!(response.stats.returned_count, 1);
    }

    #[test]
    fn stac_transport_filters_out_pgstac_fractional_cloud_cover_leak() {
        let mut leaky = s2_feature(0, true);
        leaky["properties"]["eo:cloud_cover"] = json!(20.36);
        let mut valid = s2_feature(1, true);
        valid["properties"]["eo:cloud_cover"] = json!(19.4);

        let server = JsonPageServer::new(vec![s2_page(vec![leaky, valid], false)]);
        let client = loopback_client(&server, 250);
        let aoi = json!({
            "type": "Polygon",
            "coordinates": [[[100.0, 40.0], [101.0, 40.0], [101.0, 41.0], [100.0, 41.0], [100.0, 40.0]]]
        })
        .as_object()
        .unwrap()
        .clone();
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": MPC_SENTINEL2_L2A_PROVIDER_ID,
            "native_collection_id": MPC_SENTINEL2_L2A_COLLECTION_ID,
            "aoi_geojson": aoi,
            "cursor": null,
            "result_limit": 10,
            "filters": {
                "product_type": "S2MSI2A",
                "cloud_cover_lte": 20.0
            }
        }))
        .unwrap();

        let response = client.search_catalog(&query, &|| false).unwrap();
        assert_eq!(response.products.len(), 1);
        assert_eq!(
            response.products[0]
                .properties
                .get("cloud_cover")
                .and_then(Value::as_f64),
            Some(19.4)
        );
        assert_eq!(response.stats.fetched_record_count, 2);
        assert_eq!(response.stats.exact_matched_count, 1);
        assert_eq!(response.stats.matched_count, None);
    }
}
