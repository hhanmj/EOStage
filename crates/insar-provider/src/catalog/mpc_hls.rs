//! Issue #3 R2 slice B5: Harmonized Landsat Sentinel-2 (HLS) v2.0 search on
//! Microsoft Planetary Computer. Search-only: no asset URL, signing or
//! transfer authority crosses this boundary; frozen candidates carry only the
//! reviewed scene identity, acquisition time, footprint, cloud cover and
//! platform. Microsoft hosts HLS v2 from 2020-01-01 onwards.
use std::time::Duration;

use insar_core::{
    AssetRole, CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchQuery,
    CatalogSearchResponse, CatalogSearchStats, ContractError, DataSourceKind, ProductKind,
    ProviderNetworkPolicy, ProviderProduct, ProviderResult,
};
use reqwest::Url;
use serde_json::{Map, Value};

use super::stac::StacTransport;
use super::{
    StacAssetRoleBinding, StacCollectionBinding, StacQueryOperator, StacQueryValueKind,
    StacQueryableBinding, StacRouteProfile, StacTransportLimits,
};

pub const MPC_HLS_PROVIDER_ID: &str = "microsoft.planetary-computer";
pub const MPC_HLS_STAC_ENDPOINT: &str = "https://planetarycomputer.microsoft.com/api/stac/v1/";

pub const MPC_HLS_L30_LOGICAL_DATASET: &str = "optical.hls.l30.v2";
pub const MPC_HLS_L30_COLLECTION_ID: &str = "hls2-l30";
pub const MPC_HLS_L30_PROFILE_ID: &str = "planetary-computer.hls2-l30.scene.v1";
pub const MPC_HLS_L30_ROUTE_ID: &str = "planetary-computer.hls2-l30.scene.route.v1";

pub const MPC_HLS_S30_LOGICAL_DATASET: &str = "optical.hls.s30.v2";
pub const MPC_HLS_S30_COLLECTION_ID: &str = "hls2-s30";
pub const MPC_HLS_S30_PROFILE_ID: &str = "planetary-computer.hls2-s30.scene.v1";
pub const MPC_HLS_S30_ROUTE_ID: &str = "planetary-computer.hls2-s30.scene.route.v1";

pub const MPC_HLS_IDENTITY_RULE_ID: &str = "planetary-computer.hls2.identity.v1";
pub const MPC_HLS_FILENAME_RULE_ID: &str = "planetary-computer.hls2.search-only.v1";
pub const MPC_HLS_SEARCH_ONLY_POLICY_ID: &str = "planetary-computer.hls2.search-only.v1";
pub const MPC_HLS_L30_ASSET_POLICY_ID: &str = "planetary-computer.hls2-l30.asset-set.v1";
pub const MPC_HLS_S30_ASSET_POLICY_ID: &str = "planetary-computer.hls2-s30.asset-set.v1";

/// The two HLS v2.0 products. Each is its own Microsoft collection, so one
/// catalogue query never mixes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MpcHlsProduct {
    L30,
    S30,
}

impl MpcHlsProduct {
    pub const ALL: [Self; 2] = [Self::L30, Self::S30];

    pub const fn logical_dataset(self) -> &'static str {
        match self {
            Self::L30 => MPC_HLS_L30_LOGICAL_DATASET,
            Self::S30 => MPC_HLS_S30_LOGICAL_DATASET,
        }
    }

    pub const fn collection_id(self) -> &'static str {
        match self {
            Self::L30 => MPC_HLS_L30_COLLECTION_ID,
            Self::S30 => MPC_HLS_S30_COLLECTION_ID,
        }
    }

    pub const fn profile_id(self) -> &'static str {
        match self {
            Self::L30 => MPC_HLS_L30_PROFILE_ID,
            Self::S30 => MPC_HLS_S30_PROFILE_ID,
        }
    }

    pub const fn route_id(self) -> &'static str {
        match self {
            Self::L30 => MPC_HLS_L30_ROUTE_ID,
            Self::S30 => MPC_HLS_S30_ROUTE_ID,
        }
    }

    pub const fn asset_policy_id(self) -> &'static str {
        match self {
            Self::L30 => MPC_HLS_L30_ASSET_POLICY_ID,
            Self::S30 => MPC_HLS_S30_ASSET_POLICY_ID,
        }
    }

    /// The HLS product token in the scene identifier, e.g. `HLS.L30.…`.
    pub const fn product_token(self) -> &'static str {
        match self {
            Self::L30 => "L30",
            Self::S30 => "S30",
        }
    }

    pub const fn source_kind(self) -> DataSourceKind {
        match self {
            Self::L30 => DataSourceKind::Landsat,
            Self::S30 => DataSourceKind::Sentinel2,
        }
    }

    /// Route-neutral platform vocabulary (shared with the Landsat and
    /// Sentinel-2 search schemas) paired with Microsoft's STAC `platform`.
    pub const fn platforms(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::L30 => &[("landsat_8", "landsat-8"), ("landsat_9", "landsat-9")],
            Self::S30 => &[
                ("sentinel_2a", "sentinel-2a"),
                ("sentinel_2b", "sentinel-2b"),
                ("sentinel_2c", "sentinel-2c"),
            ],
        }
    }

    pub fn for_source(
        logical_dataset: &str,
        provider_id: &str,
        native_collection_id: &str,
    ) -> Option<Self> {
        Self::ALL.into_iter().find(|product| {
            product.logical_dataset() == logical_dataset
                && provider_id == MPC_HLS_PROVIDER_ID
                && product.collection_id() == native_collection_id
        })
    }

    fn native_platform(self, platform: &str) -> Option<&'static str> {
        self.platforms()
            .iter()
            .find(|(neutral, _)| *neutral == platform)
            .map(|(_, native)| *native)
    }

    fn neutral_platform(self, platform: &str) -> Option<&'static str> {
        self.platforms()
            .iter()
            .find(|(_, native)| *native == platform)
            .map(|(neutral, _)| *neutral)
    }

    pub fn binding(self) -> StacCollectionBinding {
        StacCollectionBinding {
            logical_dataset: self.logical_dataset().to_owned(),
            provider_id: MPC_HLS_PROVIDER_ID.to_owned(),
            native_collection_id: self.collection_id().to_owned(),
            collection_profile_id: self.profile_id().to_owned(),
            identity_rule_id: MPC_HLS_IDENTITY_RULE_ID.to_owned(),
            filename_rule_id: MPC_HLS_FILENAME_RULE_ID.to_owned(),
            delivery_model: CatalogDeliveryModel::ArchiveFirst,
            source_kind: self.source_kind(),
            product_kind: ProductKind::OpticalScene,
            production_enabled: true,
            product_id_property: None,
            display_name_property: None,
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
            asset_role_bindings: {
                let allowed_hosts = vec![
                    "hls2euwest.blob.core.windows.net".to_owned(),
                    "hlseuwest.blob.core.windows.net".to_owned(),
                ];
                let keys: &[&str] = match self {
                    Self::L30 => &[
                        "B01",
                        "B02",
                        "B03",
                        "B04",
                        "B05",
                        "B06",
                        "B07",
                        "B09",
                        "B10",
                        "B11",
                        "Fmask",
                        "SAA",
                        "SZA",
                        "VAA",
                        "VZA",
                        "thumbnail",
                    ],
                    Self::S30 => &[
                        "B01",
                        "B02",
                        "B03",
                        "B04",
                        "B05",
                        "B06",
                        "B07",
                        "B08",
                        "B8A",
                        "B09",
                        "B10",
                        "B11",
                        "B12",
                        "Fmask",
                        "SAA",
                        "SZA",
                        "VAA",
                        "VZA",
                        "thumbnail",
                    ],
                };
                keys.iter()
                    .map(|key| StacAssetRoleBinding {
                        stac_asset_key: (*key).to_owned(),
                        role: if *key == "thumbnail" {
                            AssetRole::Thumbnail
                        } else {
                            AssetRole::Primary
                        },
                        required: false,
                        allowed_hosts: allowed_hosts.clone(),
                    })
                    .collect()
            },
        }
    }
}

pub fn mpc_hls_route_profile() -> StacRouteProfile {
    StacRouteProfile {
        provider_id: MPC_HLS_PROVIDER_ID.to_owned(),
        endpoint: MPC_HLS_STAC_ENDPOINT.to_owned(),
        search_path: "search".to_owned(),
        page_cap: 250,
        accepted_stac_versions: vec!["1.0.0".to_owned(), "1.1.0".to_owned()],
    }
}

pub struct MpcHlsCatalogClient {
    l30: StacTransport,
    s30: StacTransport,
}

impl std::fmt::Debug for MpcHlsCatalogClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MpcHlsCatalogClient")
            .field("provider", &MPC_HLS_PROVIDER_ID)
            .field("scope", &"fixed HLS v2.0 L30/S30 search-only STAC search")
            .finish()
    }
}

impl MpcHlsCatalogClient {
    pub fn new(network_policy: ProviderNetworkPolicy) -> ProviderResult<Self> {
        Ok(Self {
            l30: transport(MpcHlsProduct::L30, network_policy)?,
            s30: transport(MpcHlsProduct::S30, network_policy)?,
        })
    }

    pub fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let (product, normalized, exact) =
            normalize_query_with_exact_aoi(&insar_core::clamp_catalogue_search_query(query))?;
        let transport = match product {
            MpcHlsProduct::L30 => &self.l30,
            MpcHlsProduct::S30 => &self.s30,
        };
        transport.search_with_mapper_exact(&normalized, cancelled, exact.as_ref(), |_, mapped| {
            validate_product(product, mapped)
        })
    }

    /// #22 B slice 3(b): stream the mapped products page-by-page; an
    /// undated or mid-search re-sorted query fails closed with the typed
    /// `streaming_cap_order` reason for a whole-response retry.
    pub fn search_catalog_streaming<S>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut S,
    ) -> ProviderResult<(CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
    {
        let (product, normalized, exact) =
            normalize_query_with_exact_aoi(&insar_core::clamp_catalogue_search_query(query))?;
        let transport = match product {
            MpcHlsProduct::L30 => &self.l30,
            MpcHlsProduct::S30 => &self.s30,
        };
        let mut budget = transport.search_budget();
        transport.search_streaming_exact(
            &normalized,
            cancelled,
            &mut budget,
            exact.as_ref(),
            |_, mapped| validate_product(product, mapped),
            sink,
        )
    }

    #[cfg(test)]
    pub(crate) fn loopback_for_test(
        &self,
        loopback_origin: &str,
        limits: StacTransportLimits,
    ) -> ProviderResult<Self> {
        Ok(Self {
            l30: self.l30.loopback_for_test(loopback_origin, limits)?,
            s30: self.s30.loopback_for_test(loopback_origin, limits)?,
        })
    }
}

fn transport(
    product: MpcHlsProduct,
    network_policy: ProviderNetworkPolicy,
) -> ProviderResult<StacTransport> {
    let socket =
        Url::parse(MPC_HLS_STAC_ENDPOINT).expect("fixed Planetary Computer HLS STAC URL is valid");
    StacTransport::new_with_network_policy(
        mpc_hls_route_profile(),
        product.binding(),
        socket,
        StacTransportLimits {
            request_timeout: Duration::from_secs(10),
            overall_timeout: Duration::from_secs(30),
            max_pages: 128,
            max_records: insar_core::SEARCH_SAFETY_CAP as usize,
            max_page_bytes: 8 * 1024 * 1024,
            max_total_bytes: 32 * 1024 * 1024,
        },
        network_policy,
    )
}

/// Bind the query to one HLS product and translate the route-neutral
/// platform vocabulary into Microsoft's STAC values. Only the declared
/// filters are accepted.
pub fn normalize_query(
    query: &CatalogSearchQuery,
) -> ProviderResult<(MpcHlsProduct, CatalogSearchQuery)> {
    normalize_query_with_exact_aoi(query).map(|(product, normalized, _)| (product, normalized))
}

/// [`normalize_query`] plus the exact admission filter required when a
/// complex or oversized AOI was replaced by its compact request envelope.
fn normalize_query_with_exact_aoi(
    query: &CatalogSearchQuery,
) -> ProviderResult<(
    MpcHlsProduct,
    CatalogSearchQuery,
    Option<super::spatial::PreparedCatalogueAoi>,
)> {
    let product = MpcHlsProduct::for_source(
        &query.logical_dataset,
        &query.provider_id,
        &query.native_collection_id,
    )
    .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "query_source"))?;
    if query.cursor.is_some()
        || query
            .result_limit
            .is_none_or(|limit| !(1..=insar_core::SEARCH_SAFETY_CAP).contains(&limit))
    {
        return Err(error("STAC_QUERY_INVALID", "hls_query_bounds"));
    }
    let (mut normalized, exact) = super::stac::compact_complex_request_aoi(query)?;
    for (key, value) in &query.filters {
        match key.as_str() {
            "datetime" => {}
            "cloud_cover_lte" => {
                let cloud = value
                    .as_f64()
                    .filter(|cloud| cloud.is_finite() && (0.0..=100.0).contains(cloud))
                    .ok_or_else(|| error("STAC_QUERY_INVALID", "cloud_cover_lte"))?;
                normalized.filters.insert(key.clone(), Value::from(cloud));
            }
            "spacecraft" => {
                let values = value
                    .as_array()
                    .filter(|values| !values.is_empty())
                    .ok_or_else(|| error("STAC_QUERY_INVALID", "spacecraft"))?;
                let mut native = Vec::with_capacity(values.len());
                for value in values {
                    let platform = value
                        .as_str()
                        .and_then(|platform| product.native_platform(platform))
                        .ok_or_else(|| error("STAC_QUERY_INVALID", "spacecraft_value"))?;
                    if native.contains(&platform) {
                        return Err(error("STAC_QUERY_INVALID", "spacecraft_duplicate"));
                    }
                    native.push(platform);
                }
                normalized.filters.insert(
                    key.clone(),
                    Value::Array(native.into_iter().map(Value::from).collect()),
                );
            }
            _ => return Err(error("STAC_QUERY_INVALID", "undeclared_filter")),
        }
    }
    Ok((product, normalized, exact))
}

/// Accept one mapped STAC item only when its identity, platform and cloud
/// cover are those of the bound HLS product, then keep the reviewed facts.
pub fn validate_product(
    product: MpcHlsProduct,
    mut mapped: ProviderProduct,
) -> ProviderResult<ProviderProduct> {
    if mapped.provider_id != MPC_HLS_PROVIDER_ID
        || mapped.source_kind != product.source_kind()
        || mapped.product_kind != ProductKind::OpticalScene
        || mapped.product_id != mapped.display_name
        || !valid_scene_id(product, &mapped.product_id)
        || !mapped
            .acquisition_datetime
            .as_deref()
            .is_some_and(|time| scene_time_matches(&mapped.product_id, time))
        || mapped.footprint_geojson.is_none()
        || mapped.footprint_bbox.is_some()
    {
        return Err(error("STAC_IDENTITY_MISMATCH", "hls_product_scope"));
    }
    let stac = mapped
        .properties
        .get("stac_properties")
        .and_then(Value::as_object)
        .ok_or_else(|| error("STAC_RESPONSE_INVALID", "hls_properties"))?;
    let platform = stac
        .get("platform")
        .and_then(Value::as_str)
        .and_then(|platform| product.neutral_platform(platform))
        .ok_or_else(|| error("STAC_IDENTITY_MISMATCH", "hls_platform"))?;
    let cloud_cover = stac
        .get("eo:cloud_cover")
        .and_then(Value::as_f64)
        .filter(|cloud| cloud.is_finite() && (0.0..=100.0).contains(cloud))
        .ok_or_else(|| error("STAC_RESPONSE_INVALID", "hls_cloud_cover"))?;
    mapped.properties = Map::from_iter([
        ("platform".to_owned(), Value::from(platform)),
        ("cloud_cover".to_owned(), Value::from(cloud_cover)),
    ]);
    mapped.payload = Map::new();
    mapped.validate().map_err(|reason| {
        error(
            "STAC_RESPONSE_INVALID",
            &format!("provider_product:{reason}"),
        )
    })?;
    Ok(mapped)
}

/// `HLS.L30.T49RDP.2024177T030756.v2.0`: product token, MGRS tile, year +
/// day of year and time, version 2.0.
pub fn valid_scene_id(product: MpcHlsProduct, value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    let [hls, token, tile, time, version_major, version_minor] = parts.as_slice() else {
        return false;
    };
    let tile = tile.as_bytes();
    let time = time.as_bytes();
    *hls == "HLS"
        && *token == product.product_token()
        && tile.len() == 6
        && tile[0] == b'T'
        && tile[1..3].iter().all(u8::is_ascii_digit)
        && tile[3..].iter().all(u8::is_ascii_uppercase)
        && time.len() == 14
        && time[..7].iter().all(u8::is_ascii_digit)
        && time[7] == b'T'
        && time[8..].iter().all(u8::is_ascii_digit)
        && scene_id_time(value).is_some()
        && *version_major == "v2"
        && *version_minor == "0"
}

/// The `YYYYDDDTHHMMSS` sensing time of a scene id, with a real day of year
/// and clock time (review M2). Digits only (no sign).
fn scene_id_time(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let time = value.split('.').nth(3)?;
    let bytes = time.as_bytes();
    if bytes.len() != 14
        || bytes[7] != b'T'
        || !bytes[..7].iter().chain(&bytes[8..]).all(u8::is_ascii_digit)
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| time.get(range)?.parse::<u32>().ok();
    let date = chrono::NaiveDate::from_yo_opt(number(0..4)? as i32, number(4..7)?)?;
    let clock = chrono::NaiveTime::from_hms_opt(number(8..10)?, number(10..12)?, number(12..14)?)?;
    Some(chrono::DateTime::from_naive_utc_and_offset(
        date.and_time(clock),
        chrono::Utc,
    ))
}

/// The scene id names the acquisition itself for L30 (equal to the second;
/// recorded 03:07:56 vs 03:07:56.0178) and the Sentinel-2 datatake start for
/// S30 (tiles are sensed later in the datatake; recorded +14 min). The item's
/// sensing time must match: L30 within one second after the named second,
/// S30 from five minutes before to two hours after (review M2).
pub fn scene_time_matches(product_id: &str, acquisition_datetime: &str) -> bool {
    let (Some(named), Ok(sensed)) = (
        scene_id_time(product_id),
        chrono::DateTime::parse_from_rfc3339(acquisition_datetime),
    ) else {
        return false;
    };
    let offset = sensed.with_timezone(&chrono::Utc) - named;
    match product_id.split('.').nth(1) {
        Some("L30") => offset >= chrono::Duration::zero() && offset < chrono::Duration::seconds(1),
        Some("S30") => {
            offset >= chrono::Duration::minutes(-5) && offset <= chrono::Duration::hours(2)
        }
        _ => false,
    }
}

fn error(code: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        "Planetary Computer HLS 目录请求无效",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded HLS catalogue error is safe")
}

#[cfg(test)]
#[path = "mpc_hls_tests.rs"]
mod tests;
