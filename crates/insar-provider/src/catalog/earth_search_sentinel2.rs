use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::time::{Duration, Instant};

use crate::planetary_computer_asset::{
    sentinel2_l2a_reflectance_band_resolution, PlanetaryComputerReviewedAssetEvidence,
};
use insar_core::{
    provider_product_key, stable_asset_key, AssetRole, CatalogCollectionDescriptor,
    CatalogDeliveryModel, CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats,
    ContractError, DataSourceKind, ProductKind, ProviderAsset, ProviderNetworkPolicy,
    ProviderProduct, ProviderResult,
};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use reqwest::{StatusCode, Url};
use serde_json::{Map, Value};

use super::earth_search::EARTH_SEARCH_STAC_ENDPOINT;
use super::sentinel2_filters::{decode_sentinel2_spacecraft, Sentinel2Platform};
use super::spatial::{
    authoritative_catalogue_geometry, bounded_catalogue_request_tiles,
    order_request_tiles_by_exact_overlap, PreparedCatalogueAoi,
};
use super::stac::{StacSearchBudget, StacTransport};
use super::{
    StacCollectionBinding, StacQueryOperator, StacQueryValueKind, StacQueryableBinding,
    StacRouteProfile, StacTransportLimits, EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
    EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID, EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID,
    EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID, EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
};

pub const EARTH_SEARCH_SENTINEL2_COGS_HOST: &str = "sentinel-cogs.s3.us-west-2.amazonaws.com";
pub const EARTH_SEARCH_SENTINEL2_COGS_PATH_PREFIX: &str = "/sentinel-s2-l2a-cogs/";

const EARTH_SEARCH_SENTINEL2_SPATIAL_MAX_PARTS: usize = 8;
const EARTH_SEARCH_SENTINEL2_SPATIAL_LONGITUDE_SPAN: f64 = 15.0;
const EARTH_SEARCH_SENTINEL2_SPATIAL_LATITUDE_SPAN: f64 = 8.0;
const EARTH_SEARCH_SENTINEL2_SPATIAL_RECORD_BUDGET: u64 = insar_core::SEARCH_SAFETY_CAP;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarthSearchSentinel2BandAsset {
    pub asset_key: String,
    pub stac_asset_key: String,
    pub stable_href: String,
    pub file_name: String,
    pub resolution_m: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarthSearchSentinel2BandManifest {
    pub item_id: String,
    pub safe_name: String,
    pub assets: Vec<EarthSearchSentinel2BandAsset>,
}

pub fn earth_search_sentinel2_stac_asset_key(band_key: &str) -> Option<&'static str> {
    match band_key {
        "B01" => Some("coastal"),
        "B02" => Some("blue"),
        "B03" => Some("green"),
        "B04" => Some("red"),
        "B05" => Some("rededge1"),
        "B06" => Some("rededge2"),
        "B07" => Some("rededge3"),
        "B08" => Some("nir"),
        "B8A" => Some("nir08"),
        "B09" => Some("nir09"),
        "B11" => Some("swir16"),
        "B12" => Some("swir22"),
        _ => None,
    }
}

pub struct EarthSearchSentinel2CatalogClient {
    transport: StacTransport,
    probe_client: Client,
}

impl std::fmt::Debug for EarthSearchSentinel2CatalogClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EarthSearchSentinel2CatalogClient")
            .field("provider", &EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID)
            .field("collection", &EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID)
            .finish()
    }
}

impl EarthSearchSentinel2CatalogClient {
    pub fn new(network_policy: ProviderNetworkPolicy) -> ProviderResult<Self> {
        let route = StacRouteProfile {
            provider_id: EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
            endpoint: EARTH_SEARCH_STAC_ENDPOINT.to_owned(),
            search_path: "search".to_owned(),
            page_cap: 100,
            accepted_stac_versions: vec!["1.0.0".to_owned(), "1.1.0".to_owned()],
        };
        let binding = StacCollectionBinding {
            logical_dataset: "optical.sentinel-2.l2a".to_owned(),
            provider_id: EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
            native_collection_id: EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID.to_owned(),
            collection_profile_id: EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID.to_owned(),
            identity_rule_id: EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID.to_owned(),
            filename_rule_id: EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID.to_owned(),
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
        let socket = Url::parse(EARTH_SEARCH_STAC_ENDPOINT)
            .expect("fixed Element 84 Earth Search STAC URL is valid");
        let limits = StacTransportLimits {
            request_timeout: Duration::from_secs(20),
            overall_timeout: Duration::from_secs(45),
            max_pages: 128,
            max_records: insar_core::SEARCH_SAFETY_CAP as usize,
            max_page_bytes: 12 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
        };
        let transport =
            StacTransport::new_with_network_policy(route, binding, socket, limits, network_policy)?;

        let probe_builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20));
        let probe_client = crate::network_proxy::blocking_builder_with_proxy(
            probe_builder,
            network_policy.route(),
        )
        .build()
        .map_err(|_| error("STAC_TRANSPORT_FAILED", "probe_client_build"))?;

        Ok(Self {
            transport,
            probe_client,
        })
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
            Err(err) => Err(err),
        }
    }

    pub fn search_catalog_streaming<S>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut S,
    ) -> ProviderResult<(CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
    {
        let clamped = insar_core::clamp_catalogue_search_query(query);
        let normalized = normalize_query(&clamped)?;
        let (request_query, exact_aoi) = super::stac::compact_complex_request_aoi(&normalized)?;
        let spacecraft = decode_sentinel2_spacecraft(clamped.filters.get("spacecraft"))
            .map_err(|reason| error("STAC_QUERY_INVALID", reason))?;
        let cloud_limit = normalized
            .filters
            .get("cloud_cover_lte")
            .and_then(Value::as_f64);
        let mut budget = self.transport.search_budget();
        self.transport.search_streaming_exact(
            &request_query,
            cancelled,
            &mut budget,
            exact_aoi.as_ref(),
            move |feature, product| map_search_feature(feature, product, &spacecraft, cloud_limit),
            sink,
        )
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
            .filter(|target| *target > 0 && *target <= EARTH_SEARCH_SENTINEL2_SPATIAL_RECORD_BUDGET)
            .ok_or_else(|| limit_error("spatial_target"))?;
        let tiles = bounded_catalogue_request_tiles(
            canonical_aoi,
            EARTH_SEARCH_SENTINEL2_SPATIAL_LONGITUDE_SPAN,
            EARTH_SEARCH_SENTINEL2_SPATIAL_LATITUDE_SPAN,
            EARTH_SEARCH_SENTINEL2_SPATIAL_MAX_PARTS,
        )
        .ok_or_else(|| limit_error("spatial_plan"))?;
        let prepared_aoi = PreparedCatalogueAoi::new(canonical_aoi);
        let tiles = order_request_tiles_by_exact_overlap(prepared_aoi.as_ref(), tiles);
        if tiles.is_empty() {
            return Err(limit_error("spatial_plan"));
        }
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
            part.result_limit = Some(target);
            let response = self.transport.search_with_mapper_in_budget(
                &part,
                cancelled,
                budget,
                |feature, product| {
                    let product = map_search_feature(feature, product, spacecraft, cloud_limit)?;
                    admit_spatial_product(
                        &mut unique,
                        &mut exact,
                        &product,
                        prepared_aoi.as_ref(),
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
            if fetched > EARTH_SEARCH_SENTINEL2_SPATIAL_RECORD_BUDGET
                || mapped > EARTH_SEARCH_SENTINEL2_SPATIAL_RECORD_BUDGET
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

    pub fn resolve_analysis_band_manifest_by_item_id(
        &self,
        item_id: &str,
        safe_name: &str,
        acquisition_datetime: Option<&str>,
        asset_keys: &[String],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<EarthSearchSentinel2BandManifest> {
        validate_analysis_band_selection(asset_keys)?;
        validate_earth_search_sentinel2_source_id(item_id, safe_name, acquisition_datetime)?;
        let (feature, product) = self.transport.read_exact_item(item_id, cancelled)?;
        let product = map_feature(&feature, product, None)?;
        if feature
            .pointer("/properties/s2:product_uri")
            .and_then(Value::as_str)
            != Some(safe_name)
        {
            return Err(error("STAC_IDENTITY_MISMATCH", "product_uri_mismatch"));
        }
        validate_earth_search_sentinel2_source_id(
            &product.product_id,
            safe_name,
            acquisition_datetime,
        )?;
        let assets = project_analysis_band_assets(&feature, item_id, asset_keys)?;
        Ok(EarthSearchSentinel2BandManifest {
            item_id: item_id.to_owned(),
            safe_name: safe_name.to_owned(),
            assets,
        })
    }

    pub fn resolve_analysis_band_manifest_by_safe_name(
        &self,
        safe_name: &str,
        acquisition_datetime: Option<&str>,
        asset_keys: &[String],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<EarthSearchSentinel2BandManifest> {
        validate_analysis_band_selection(asset_keys)?;
        let candidates = candidate_item_ids_from_safe_name(safe_name, acquisition_datetime)?;
        let mut last_err = None;
        for candidate in candidates {
            if cancelled() {
                return Err(error("STAC_SEARCH_CANCELLED", "cancelled"));
            }
            match self.transport.read_exact_item(&candidate, cancelled) {
                Ok((feature, product)) => {
                    if feature
                        .pointer("/properties/s2:product_uri")
                        .and_then(Value::as_str)
                        != Some(safe_name)
                    {
                        continue;
                    }
                    let product = map_feature(&feature, product, None)?;
                    validate_earth_search_sentinel2_source_id(
                        &product.product_id,
                        safe_name,
                        product.acquisition_datetime.as_deref(),
                    )?;
                    let assets = project_analysis_band_assets(&feature, &candidate, asset_keys)?;
                    return Ok(EarthSearchSentinel2BandManifest {
                        item_id: candidate,
                        safe_name: safe_name.to_owned(),
                        assets,
                    });
                }
                Err(err) => {
                    last_err = Some(err);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| error("STAC_ITEM_NOT_FOUND", "safe_name_not_found")))
    }

    pub fn probe_public_band_asset(
        &self,
        item_id: &str,
        band_key: &str,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<PlanetaryComputerReviewedAssetEvidence> {
        if cancelled() {
            return Err(error("STAC_SEARCH_CANCELLED", "cancelled"));
        }
        let file_name =
            validate_earth_search_sentinel2_stable_band_url(item_id, band_key, stable_href)?;
        let mut response = self
            .probe_client
            .get(stable_href)
            .header(RANGE, "bytes=0-0")
            .send()
            .map_err(|_| error("EARTH_SEARCH_ASSET_PROBE_FAILED", "transport"))?;
        if cancelled() {
            return Err(error("STAC_SEARCH_CANCELLED", "cancelled"));
        }
        if response.status() != StatusCode::PARTIAL_CONTENT
            || response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                != Some("1")
        {
            return Err(error("EARTH_SEARCH_ASSET_PROBE_FAILED", "range_status"));
        }
        let total = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_one_byte_content_range)
            .filter(|value| *value > 1)
            .ok_or_else(|| error("EARTH_SEARCH_ASSET_PROBE_FAILED", "content_range"))?;
        let mut byte = [0_u8; 2];
        let read = response
            .read(&mut byte)
            .map_err(|_| error("EARTH_SEARCH_ASSET_PROBE_FAILED", "range_read"))?;
        if read != 1 {
            return Err(error("EARTH_SEARCH_ASSET_PROBE_FAILED", "range_body"));
        }
        if cancelled() {
            return Err(error("STAC_SEARCH_CANCELLED", "cancelled"));
        }
        Ok(PlanetaryComputerReviewedAssetEvidence {
            file_name,
            expected_size_bytes: total,
        })
    }
}

fn parse_one_byte_content_range(value: &str) -> Option<u64> {
    let total = value.trim().strip_prefix("bytes 0-0/")?;
    if total.is_empty() || !total.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    total.parse::<u64>().ok()
}

fn candidate_item_ids_from_safe_name(
    safe_name: &str,
    acquisition_datetime: Option<&str>,
) -> ProviderResult<Vec<String>> {
    let safe = safe_name
        .strip_suffix(".SAFE")
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "safe_suffix"))?
        .split('_')
        .collect::<Vec<_>>();
    if safe.len() != 7
        || !matches!(safe[0], "S2A" | "S2B" | "S2C")
        || safe[1] != "MSIL2A"
        || !valid_datetime_token(safe[2])
        || !valid_datetime_token(safe[6])
        || !safe[3].strip_prefix('N').is_some_and(four_digits)
        || !safe[4].strip_prefix('R').is_some_and(three_digits)
    {
        return Err(error("STAC_IDENTITY_MISMATCH", "safe_name"));
    }
    let Some(mgrs_tile) = safe[5].strip_prefix('T').filter(|t| valid_mgrs_tile(t)) else {
        return Err(error("STAC_IDENTITY_MISMATCH", "mgrs_tile"));
    };
    let mut dates = Vec::new();
    if let Some(acq) = acquisition_datetime {
        if acq.len() >= 10
            && acq.as_bytes()[4] == b'-'
            && acq.as_bytes()[7] == b'-'
            && acq[0..4].bytes().all(|b| b.is_ascii_digit())
            && acq[5..7].bytes().all(|b| b.is_ascii_digit())
            && acq[8..10].bytes().all(|b| b.is_ascii_digit())
        {
            dates.push(format!("{}{}{}", &acq[0..4], &acq[5..7], &acq[8..10]));
        }
    }
    let safe_date = safe[2][0..8].to_owned();
    if !dates.contains(&safe_date) {
        dates.push(safe_date);
    }
    let mut candidates = Vec::new();
    for date in dates {
        for seq in 0..=3 {
            candidates.push(format!("{}_{}_{}_{}_L2A", safe[0], mgrs_tile, date, seq));
        }
    }
    Ok(candidates)
}

fn validate_analysis_band_selection(asset_keys: &[String]) -> ProviderResult<()> {
    if asset_keys.is_empty() || asset_keys.len() > 12 {
        return Err(error("STAC_QUERY_INVALID", "analysis_band_count"));
    }
    let mut seen = BTreeSet::new();
    for key in asset_keys {
        if sentinel2_l2a_reflectance_band_resolution(key).is_none()
            || earth_search_sentinel2_stac_asset_key(key).is_none()
            || !seen.insert(key.as_str())
        {
            return Err(error("STAC_QUERY_INVALID", "analysis_band_selection"));
        }
    }
    Ok(())
}

fn project_analysis_band_assets(
    feature: &Value,
    item_id: &str,
    asset_keys: &[String],
) -> ProviderResult<Vec<EarthSearchSentinel2BandAsset>> {
    let assets = feature
        .get("assets")
        .and_then(Value::as_object)
        .ok_or_else(|| error("STAC_RESPONSE_INVALID", "analysis_assets"))?;
    let mut projected = Vec::with_capacity(asset_keys.len());
    for key in asset_keys {
        let resolution = sentinel2_l2a_reflectance_band_resolution(key)
            .ok_or_else(|| error("STAC_QUERY_INVALID", "analysis_band_key"))?;
        let stac_asset_key = earth_search_sentinel2_stac_asset_key(key)
            .ok_or_else(|| error("STAC_QUERY_INVALID", "analysis_band_key"))?;
        let asset = assets
            .get(stac_asset_key)
            .or_else(|| assets.get(key))
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
        if !media_type.starts_with("image/tiff")
            || !roles.iter().any(|role| role.as_str() == Some("data"))
        {
            return Err(error("STAC_RESPONSE_INVALID", "analysis_band_semantics"));
        }
        if let Some(gsd) = asset.get("gsd").and_then(Value::as_f64) {
            if !gsd.is_finite() || (gsd - f64::from(resolution)).abs() > f64::EPSILON {
                return Err(error("STAC_RESPONSE_INVALID", "analysis_band_gsd"));
            }
        }
        let file_name = validate_earth_search_sentinel2_stable_band_url(item_id, key, href)?;
        projected.push(EarthSearchSentinel2BandAsset {
            asset_key: key.clone(),
            stac_asset_key: stac_asset_key.to_owned(),
            stable_href: href.to_owned(),
            file_name,
            resolution_m: resolution,
        });
    }
    Ok(projected)
}

pub fn normalize_earth_search_sentinel2_query(
    query: &CatalogSearchQuery,
) -> ProviderResult<CatalogSearchQuery> {
    normalize_query(query)
}

fn normalize_query(query: &CatalogSearchQuery) -> ProviderResult<CatalogSearchQuery> {
    if query.logical_dataset != "optical.sentinel-2.l2a"
        || query.provider_id != EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
        || query.native_collection_id != EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID
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
                    .map(|p| Value::String(p.stac_name.to_ascii_lowercase()))
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
    cloud_limit: Option<f64>,
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
    let mgrs_tile = extract_mgrs_tile(properties, item_id)?;
    let platform = properties
        .get("platform")
        .and_then(Value::as_str)
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "platform"))?;
    let cloud_cover = properties
        .get("eo:cloud_cover")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or_else(|| error("STAC_RESPONSE_INVALID", "cloud_cover"))?;
    if cloud_limit.is_some_and(|limit| cloud_cover > limit) {
        return Err(error("STAC_RESPONSE_INVALID", "cloud_filter"));
    }
    validate_identity(
        item_id,
        product_uri,
        &mgrs_tile,
        platform,
        product.acquisition_datetime.as_deref(),
    )?;
    if product.product_id != item_id
        || product.display_name != product_uri
        || !product.assets.is_empty()
    {
        return Err(error("STAC_IDENTITY_MISMATCH", "normalized_product"));
    }
    if let Some(thumbnail) = extract_validated_thumbnail_asset(feature, item_id)? {
        product.assets.push(thumbnail);
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

fn extract_mgrs_tile(properties: &Map<String, Value>, item_id: &str) -> ProviderResult<String> {
    if let Some(tile) = properties.get("s2:mgrs_tile").and_then(Value::as_str) {
        if valid_mgrs_tile(tile) {
            return Ok(tile.to_owned());
        }
    }
    let zone = properties.get("mgrs:utm_zone").and_then(|v| {
        v.as_u64()
            .map(|n| format!("{n:02}"))
            .or_else(|| v.as_str().map(str::to_owned))
    });
    let band = properties
        .get("mgrs:latitude_band")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let square = properties
        .get("mgrs:grid_square")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let (Some(z), Some(b), Some(s)) = (zone, band, square) {
        let candidate = format!("{z}{b}{s}");
        if valid_mgrs_tile(&candidate) {
            return Ok(candidate);
        }
    }
    item_id
        .split('_')
        .nth(1)
        .filter(|tile| valid_mgrs_tile(tile))
        .map(str::to_owned)
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "mgrs_tile"))
}

fn extract_validated_thumbnail_asset(
    feature: &Value,
    item_id: &str,
) -> ProviderResult<Option<ProviderAsset>> {
    let Some(thumbnail) = feature
        .pointer("/assets/thumbnail")
        .and_then(Value::as_object)
    else {
        return Ok(None);
    };
    let Some(href) = thumbnail.get("href").and_then(Value::as_str) else {
        return Ok(None);
    };
    let media_type = thumbnail
        .get("type")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "image/jpeg".to_owned());
    if media_type != "image/jpeg" {
        return Ok(None);
    }
    let Ok(url) = Url::parse(href) else {
        return Ok(None);
    };
    if url.scheme() != "https"
        || url.host_str() != Some(EARTH_SEARCH_SENTINEL2_COGS_HOST)
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url
            .path()
            .starts_with(EARTH_SEARCH_SENTINEL2_COGS_PATH_PREFIX)
        || !url.path().ends_with(".jpg")
        || !url.path().contains(&format!("/{item_id}/"))
    {
        return Ok(None);
    }
    let file_name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let product_key = provider_product_key(
        EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
        EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
        item_id,
    )
    .map_err(|_| error("STAC_IDENTITY_MISMATCH", "thumbnail_product_key"))?;
    let stable_key = stable_asset_key(&product_key, "thumbnail")
        .map_err(|_| error("STAC_IDENTITY_MISMATCH", "thumbnail_asset_key"))?;
    Ok(Some(ProviderAsset {
        asset_key: Some("thumbnail".to_owned()),
        stable_asset_key: Some(stable_key),
        role: AssetRole::Thumbnail,
        native_roles: vec!["thumbnail".to_owned()],
        url: Some(href.to_owned()),
        access_descriptor: None,
        media_type: Some("image/jpeg".to_owned()),
        file_name,
        size_bytes: None,
    }))
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
    if item.len() != 5
        || safe.len() != 7
        || item[4] != "L2A"
        || safe[1] != "MSIL2A"
        || item[0] != safe[0]
        || !valid_mgrs_tile(item[1])
        || item[1] != mgrs_tile
        || safe[5].strip_prefix('T') != Some(mgrs_tile)
        || !eight_digits(item[2])
        || !seq_digits(item[3])
        || !valid_datetime_token(safe[2])
        || !valid_datetime_token(safe[6])
        || !safe[3].strip_prefix('N').is_some_and(four_digits)
        || !safe[4].strip_prefix('R').is_some_and(three_digits)
    {
        return Err(error(
            "STAC_IDENTITY_MISMATCH",
            "earth_search_sentinel2_identity",
        ));
    }
    let expected_platform = match item[0] {
        "S2A" => "sentinel-2a",
        "S2B" => "sentinel-2b",
        "S2C" => "sentinel-2c",
        _ => return Err(error("STAC_IDENTITY_MISMATCH", "platform_code")),
    };
    if !platform.eq_ignore_ascii_case(expected_platform) {
        return Err(error("STAC_IDENTITY_MISMATCH", "platform"));
    }
    let expected_date_prefix = format!("{}-{}-{}T", &item[2][0..4], &item[2][4..6], &item[2][6..8]);
    if !acquisition_datetime.is_some_and(|value| value.starts_with(&expected_date_prefix)) {
        return Err(error("STAC_IDENTITY_MISMATCH", "acquisition_datetime"));
    }
    Ok(())
}

pub fn validate_earth_search_sentinel2_source_id(
    item_id: &str,
    product_uri: &str,
    acquisition_datetime: Option<&str>,
) -> ProviderResult<()> {
    let item = item_id.split('_').collect::<Vec<_>>();
    let mgrs_tile = item
        .get(1)
        .copied()
        .filter(|value| valid_mgrs_tile(value))
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "mgrs_tile"))?;
    let platform = match item.first().copied() {
        Some("S2A") => "sentinel-2a",
        Some("S2B") => "sentinel-2b",
        Some("S2C") => "sentinel-2c",
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

pub fn validate_earth_search_sentinel2_stable_band_url(
    item_id: &str,
    band_key: &str,
    raw_url: &str,
) -> ProviderResult<String> {
    if sentinel2_l2a_reflectance_band_resolution(band_key).is_none()
        || earth_search_sentinel2_stac_asset_key(band_key).is_none()
    {
        return Err(error("EARTH_SEARCH_ASSET_REJECTED", "band_key"));
    }
    validate_earth_search_sentinel2_object_url(item_id, &format!("{band_key}.tif"), raw_url)?;
    Ok(format!("{item_id}_{band_key}.tif"))
}

pub fn validate_earth_search_sentinel2_scl_url(
    item_id: &str,
    raw_url: &str,
) -> ProviderResult<String> {
    validate_earth_search_sentinel2_object_url(item_id, "SCL.tif", raw_url)?;
    Ok(format!("{item_id}_SCL.tif"))
}

pub fn is_valid_earth_search_sentinel2_thumbnail_url(item_id: &str, url: &Url) -> bool {
    let Some(segments) = url.path_segments().map(|it| it.collect::<Vec<_>>()) else {
        return false;
    };
    if segments.len() != 8 || !segments[7].ends_with(".jpg") || segments[7].len() <= 4 {
        return false;
    }
    validate_earth_search_sentinel2_object_url(item_id, segments[7], url.as_str()).is_ok()
}

fn validate_earth_search_sentinel2_object_url(
    item_id: &str,
    expected_remote_file: &str,
    raw_url: &str,
) -> ProviderResult<()> {
    let item = item_id.split('_').collect::<Vec<_>>();
    if item.len() != 5
        || !matches!(item[0], "S2A" | "S2B" | "S2C")
        || !valid_mgrs_tile(item[1])
        || !eight_digits(item[2])
        || !seq_digits(item[3])
        || item[4] != "L2A"
    {
        return Err(error("EARTH_SEARCH_ASSET_REJECTED", "item_id"));
    }
    let url =
        Url::parse(raw_url).map_err(|_| error("EARTH_SEARCH_ASSET_REJECTED", "stable_href"))?;
    let path_lower = url.path().to_ascii_lowercase();
    if url.scheme() != "https"
        || url.host_str() != Some(EARTH_SEARCH_SENTINEL2_COGS_HOST)
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || ["%2f", "%5c", "%2e"]
            .iter()
            .any(|needle| path_lower.contains(needle))
    {
        return Err(error("EARTH_SEARCH_ASSET_REJECTED", "stable_scope"));
    }
    let segments = url
        .path_segments()
        .map(|it| it.collect::<Vec<_>>())
        .ok_or_else(|| error("EARTH_SEARCH_ASSET_REJECTED", "path_segments"))?;
    if segments.len() != 8 || segments[0] != "sentinel-s2-l2a-cogs" {
        return Err(error("EARTH_SEARCH_ASSET_REJECTED", "path_layout"));
    }
    let utm_zone = &item[1][0..2];
    let utm_zone_unpadded = utm_zone
        .parse::<u32>()
        .ok()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let lat_band = &item[1][2..3];
    let grid_square = &item[1][3..5];
    let year = &item[2][0..4];
    let month_num = item[2][4..6]
        .parse::<u32>()
        .ok()
        .filter(|m| (1..=12).contains(m))
        .ok_or_else(|| error("EARTH_SEARCH_ASSET_REJECTED", "item_month"))?;
    let month_unpadded = month_num.to_string();
    let month_padded = format!("{month_num:02}");

    if (segments[1] != utm_zone && segments[1] != utm_zone_unpadded)
        || segments[2] != lat_band
        || segments[3] != grid_square
        || segments[4] != year
        || (segments[5] != month_unpadded && segments[5] != month_padded)
        || segments[6] != item_id
        || segments[7] != expected_remote_file
    {
        return Err(error("EARTH_SEARCH_ASSET_REJECTED", "object_identity"));
    }

    Ok(())
}

pub fn is_valid_earth_search_sentinel2_cog_url(url: &Url) -> bool {
    let Some(segments) = url.path_segments().map(|it| it.collect::<Vec<_>>()) else {
        return false;
    };
    if segments.len() != 8 || segments[0] != "sentinel-s2-l2a-cogs" {
        return false;
    }
    let item_id = segments[6];
    let Some(band_key) = segments[7].strip_suffix(".tif") else {
        return false;
    };
    validate_earth_search_sentinel2_stable_band_url(item_id, band_key, url.as_str()).is_ok()
}

fn valid_datetime_token(value: &str) -> bool {
    value.len() == 15
        && value.as_bytes().get(8) == Some(&b'T')
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 8 || byte.is_ascii_digit())
}

fn eight_digits(value: &str) -> bool {
    value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn four_digits(value: &str) -> bool {
    value.len() == 4 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn three_digits(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn seq_digits(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4 && value.bytes().all(|byte| byte.is_ascii_digit())
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
    .expect("bounded Earth Search Sentinel-2 limit error is safe")
}

fn admit_spatial_product(
    unique: &mut BTreeMap<String, ProviderProduct>,
    exact: &mut BTreeMap<String, ProviderProduct>,
    product: &ProviderProduct,
    prepared_aoi: Option<&PreparedCatalogueAoi>,
) -> ProviderResult<()> {
    if let Some(existing) = unique.get(&product.product_id) {
        if existing != product {
            return Err(error("STAC_IDENTITY_MISMATCH", "spatial_product_conflict"));
        }
        return Ok(());
    }
    unique.insert(product.product_id.clone(), product.clone());
    if product.footprint_geojson.as_ref().is_some_and(|footprint| {
        prepared_aoi.is_some_and(|prepared| prepared.intersects_footprint(footprint))
    }) {
        exact.insert(product.product_id.clone(), product.clone());
    }
    Ok(())
}

fn error(code: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        "Element 84 Earth Search Sentinel-2 STAC 证据无效",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded Earth Search Sentinel-2 STAC error is safe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::{resolve_preview_candidate, PreviewHttpClient, PreviewProductClass};
    use insar_core::ProxyPolicy;
    use serde_json::json;

    #[test]
    fn earth_search_sentinel2_normalizes_lowercase_spacecraft_and_validates_identity() {
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
            "native_collection_id": EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[116.2, 39.8], [116.5, 39.8], [116.5, 40.0], [116.2, 39.8]]]
            },
            "cursor": null,
            "result_limit": 5,
            "filters": {
                "product_type": "S2MSI2A",
                "cloud_cover_lte": 25.0,
                "spacecraft": ["sentinel_2a", "sentinel_2c"],
                "datetime": {
                    "start": "2025-01-01T00:00:00Z",
                    "end": "2025-02-01T00:00:00Z"
                }
            }
        }))
        .unwrap();

        let normalized = normalize_earth_search_sentinel2_query(&query).unwrap();
        assert_eq!(
            normalized.filters["spacecraft"],
            json!(["sentinel-2a", "sentinel-2c"])
        );
        assert!(normalized.filters.get("product_type").is_none());

        validate_earth_search_sentinel2_source_id(
            "S2A_50SNA_20261116_1_L2A",
            "S2A_MSIL2A_20261116T030141_N0513_R032_T50SNA_20261116T062708.SAFE",
            Some("2026-11-16T03:08:58.833Z"),
        )
        .unwrap();

        let file_name = validate_earth_search_sentinel2_stable_band_url(
            "S2A_50SMA_20250113_0_L2A",
            "B02",
            "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/50/S/MA/2025/1/S2A_50SMA_20250113_0_L2A/B02.tif",
        )
        .unwrap();
        assert_eq!(file_name, "S2A_50SMA_20250113_0_L2A_B02.tif");

        assert!(validate_earth_search_sentinel2_stable_band_url(
            "S2A_50SMA_20250113_0_L2A",
            "B02",
            "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/50/S/MA/2025/1/S2A_50SMA_20250113_0_L2A/B04.tif",
        )
        .is_err());
    }

    #[test]
    #[ignore = "live network check against Element 84 Earth Search STAC and public AWS S3 Sentinel-2 COGs"]
    fn live_earth_search_sentinel2_search_quicklook_and_cog_probe_succeed() {
        let client =
            EarthSearchSentinel2CatalogClient::new(ProviderNetworkPolicy::system()).unwrap();
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
            "native_collection_id": EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[116.2, 39.8], [116.5, 39.8], [116.5, 40.0], [116.2, 40.0], [116.2, 39.8]]]
            },
            "cursor": null,
            "result_limit": 2,
            "filters": {
                "product_type": "S2MSI2A",
                "cloud_cover_lte": 40.0,
                "datetime": {
                    "start": "2025-01-01T00:00:00Z",
                    "end": "2025-02-01T00:00:00Z"
                }
            }
        }))
        .unwrap();

        let response = client.search_catalog(&query, &|| false).unwrap();
        assert!(!response.products.is_empty());
        let first = &response.products[0];
        assert_eq!(first.provider_id, EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID);

        let product_key = provider_product_key(
            EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
            EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
            &first.product_id,
        )
        .unwrap();
        let preview_candidate = resolve_preview_candidate(
            &product_key,
            PreviewProductClass::EarthSearchSentinel2Quicklook,
            &first.assets,
        )
        .unwrap();
        let preview_image = PreviewHttpClient::with_route(ProxyPolicy::System)
            .unwrap()
            .fetch(&preview_candidate, &|| false)
            .unwrap();
        assert_eq!(preview_image.media_type, "image/jpeg");
        assert!(preview_image.bytes.starts_with(&[0xFF, 0xD8, 0xFF]));

        let manifest = client
            .resolve_analysis_band_manifest_by_item_id(
                &first.product_id,
                &first.display_name,
                first.acquisition_datetime.as_deref(),
                &["B04".to_owned(), "B08".to_owned()],
                &|| false,
            )
            .unwrap();
        assert_eq!(manifest.assets.len(), 2);
        let probed = client
            .probe_public_band_asset(
                &manifest.item_id,
                &manifest.assets[0].asset_key,
                &manifest.assets[0].stable_href,
                &|| false,
            )
            .unwrap();
        assert!(probed.expected_size_bytes > 1024);
    }
}
