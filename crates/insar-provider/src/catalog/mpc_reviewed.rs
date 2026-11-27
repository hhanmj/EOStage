use std::time::Duration;

use insar_core::{
    AssetRole, CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchQuery,
    CatalogSearchResponse, CatalogSearchStats, ContractError, DataSourceKind, ProductKind,
    ProviderNetworkPolicy, ProviderProduct, ProviderResult,
};
use reqwest::Url;
use serde_json::{Map, Value};

use crate::planetary_computer_asset::{
    validate_planetary_computer_reviewed_asset_identity, PlanetaryComputerReviewedAssetClass,
    MPC_COPDEM_GLO30_COLLECTION_ID, MPC_COPDEM_GLO30_FILENAME_RULE_ID,
    MPC_COPDEM_GLO30_LOGICAL_DATASET, MPC_REVIEWED_STAC_IDENTITY_RULE_ID,
    MPC_SENTINEL1_GRD_COLLECTION_ID, MPC_SENTINEL1_GRD_FILENAME_RULE_ID,
    MPC_SENTINEL1_GRD_LOGICAL_DATASET, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};

use super::stac::StacTransport;
use super::{StacAssetRoleBinding, StacCollectionBinding, StacRouteProfile, StacTransportLimits};

pub const MPC_REVIEWED_STAC_ENDPOINT: &str = "https://planetarycomputer.microsoft.com/api/stac/v1/";

pub struct MpcReviewedCatalogClient {
    sentinel1_grd: StacTransport,
    copdem_glo30: StacTransport,
}

impl std::fmt::Debug for MpcReviewedCatalogClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MpcReviewedCatalogClient")
            .field("provider", &PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID)
            .field(
                "scope",
                &"fixed reviewed Sentinel-1 GRD and COP30 STAC search",
            )
            .finish()
    }
}

impl MpcReviewedCatalogClient {
    pub fn new(network_policy: ProviderNetworkPolicy) -> ProviderResult<Self> {
        Ok(Self {
            sentinel1_grd: reviewed_transport(
                PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
                network_policy,
            )?,
            copdem_glo30: reviewed_transport(
                PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
                network_policy,
            )?,
        })
    }

    pub fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        let class = exact_class(query)?;
        let normalized = normalize_query(query)?;
        if class == PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog {
            return Err(error(
                "STAC_IDENTITY_MISMATCH",
                "sentinel2_band_search_forbidden",
            ));
        }
        match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => self
                .sentinel1_grd
                .search_with_mapper(&normalized, cancelled, |_, product| {
                    validate_product(class, product)
                }),
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => self
                .copdem_glo30
                .search_with_mapper(&normalized, cancelled, |_, product| {
                    validate_product(class, product)
                }),
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                unreachable!("Sentinel-2 band delivery is never a reviewed search route")
            }
        }
    }

    /// #22 B slice 3(b): stream the mapped products page-by-page; an undated
    /// or mid-search re-sorted query fails closed with the typed
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
        let query = &insar_core::clamp_catalogue_search_query(query);
        let class = exact_class(query)?;
        let normalized = normalize_query(query)?;
        if class == PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog {
            return Err(error(
                "STAC_IDENTITY_MISMATCH",
                "sentinel2_band_search_forbidden",
            ));
        }
        let transport = match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => &self.sentinel1_grd,
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => &self.copdem_glo30,
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                unreachable!("Sentinel-2 band delivery is never a reviewed search route")
            }
        };
        let mut budget = transport.search_budget();
        transport.search_streaming(
            &normalized,
            cancelled,
            &mut budget,
            |_, product| validate_product(class, product),
            sink,
        )
    }
}

fn reviewed_transport(
    class: PlanetaryComputerReviewedAssetClass,
    network_policy: ProviderNetworkPolicy,
) -> ProviderResult<StacTransport> {
    if class == PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog {
        return Err(error(
            "STAC_IDENTITY_MISMATCH",
            "sentinel2_band_search_forbidden",
        ));
    }
    let route = StacRouteProfile {
        provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
        endpoint: MPC_REVIEWED_STAC_ENDPOINT.to_owned(),
        search_path: "search".to_owned(),
        page_cap: 100,
        accepted_stac_versions: vec!["1.0.0".to_owned(), "1.1.0".to_owned()],
    };
    let binding = StacCollectionBinding {
        logical_dataset: class.logical_dataset().to_owned(),
        provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
        native_collection_id: class.collection_id().to_owned(),
        collection_profile_id: class.collection_profile_id().to_owned(),
        identity_rule_id: MPC_REVIEWED_STAC_IDENTITY_RULE_ID.to_owned(),
        filename_rule_id: match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => {
                MPC_SENTINEL1_GRD_FILENAME_RULE_ID
            }
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => {
                MPC_COPDEM_GLO30_FILENAME_RULE_ID
            }
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                unreachable!("Sentinel-2 band delivery is never a reviewed search route")
            }
        }
        .to_owned(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        source_kind: match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => DataSourceKind::Sentinel1,
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => DataSourceKind::Dem,
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                unreachable!("Sentinel-2 band delivery is never a reviewed search route")
            }
        },
        product_kind: match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => ProductKind::SarGrd,
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => ProductKind::DemRaster,
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                unreachable!("Sentinel-2 band delivery is never a reviewed search route")
            }
        },
        production_enabled: true,
        product_id_property: None,
        display_name_property: None,
        queryables: Vec::new(),
        asset_role_bindings: match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => ["vv", "vh", "hh", "hv"]
                .into_iter()
                .map(|key| StacAssetRoleBinding {
                    stac_asset_key: key.to_owned(),
                    role: AssetRole::Primary,
                    required: false,
                    allowed_hosts: vec!["sentinel1euwest.blob.core.windows.net".to_owned()],
                })
                .collect(),
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => {
                vec![StacAssetRoleBinding {
                    stac_asset_key: "data".to_owned(),
                    role: AssetRole::Primary,
                    required: true,
                    allowed_hosts: vec!["elevationeuwest.blob.core.windows.net".to_owned()],
                }]
            }
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                unreachable!("Sentinel-2 band delivery is never a reviewed search route")
            }
        },
    };
    let socket = Url::parse(MPC_REVIEWED_STAC_ENDPOINT)
        .expect("fixed Planetary Computer reviewed STAC URL is valid");
    StacTransport::new_with_network_policy(
        route,
        binding,
        socket,
        StacTransportLimits {
            request_timeout: Duration::from_secs(10),
            overall_timeout: Duration::from_secs(30),
            max_pages: 128,
            max_records: insar_core::SEARCH_SAFETY_CAP as usize,
            max_page_bytes: 2 * 1024 * 1024,
            max_total_bytes: 8 * 1024 * 1024,
        },
        network_policy,
    )
}

fn exact_class(query: &CatalogSearchQuery) -> ProviderResult<PlanetaryComputerReviewedAssetClass> {
    match (
        query.logical_dataset.as_str(),
        query.provider_id.as_str(),
        query.native_collection_id.as_str(),
    ) {
        (
            MPC_SENTINEL1_GRD_LOGICAL_DATASET,
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
            MPC_SENTINEL1_GRD_COLLECTION_ID,
        ) => Ok(PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog),
        (
            MPC_COPDEM_GLO30_LOGICAL_DATASET,
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
            MPC_COPDEM_GLO30_COLLECTION_ID,
        ) => Ok(PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog),
        _ => Err(error("STAC_IDENTITY_MISMATCH", "query_source")),
    }
}

fn normalize_query(query: &CatalogSearchQuery) -> ProviderResult<CatalogSearchQuery> {
    exact_class(query)?;
    if query.cursor.is_some()
        || query
            .result_limit
            .is_none_or(|limit| !(1..=insar_core::SEARCH_SAFETY_CAP).contains(&limit))
        || query.filters.keys().any(|key| key != "datetime")
    {
        return Err(error("STAC_QUERY_INVALID", "reviewed_query_bounds"));
    }
    Ok(query.clone())
}

fn validate_product(
    class: PlanetaryComputerReviewedAssetClass,
    mut product: ProviderProduct,
) -> ProviderResult<ProviderProduct> {
    if class == PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog {
        return Err(error(
            "STAC_IDENTITY_MISMATCH",
            "sentinel2_band_search_forbidden",
        ));
    }
    let expected_source_kind = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => DataSourceKind::Sentinel1,
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => DataSourceKind::Dem,
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => unreachable!(),
    };
    let expected_product_kind = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => ProductKind::SarGrd,
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => ProductKind::DemRaster,
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => unreachable!(),
    };
    if product.provider_id != PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID
        || product.source_kind != expected_source_kind
        || product.product_kind != expected_product_kind
        || product.product_id != product.display_name
        || product.footprint_geojson.is_none()
        || product.footprint_bbox.is_some()
    {
        return Err(error("STAC_IDENTITY_MISMATCH", "product_scope"));
    }
    let expected_asset_count = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => 1..=4,
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => 1..=1,
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => unreachable!(),
    };
    if !expected_asset_count.contains(&product.assets.len()) {
        return Err(error("STAC_ASSET_INVALID", "reviewed_asset_count"));
    }
    for asset in &product.assets {
        let asset_key = asset
            .asset_key
            .as_deref()
            .ok_or_else(|| error("STAC_ASSET_INVALID", "asset_key"))?;
        if asset.role != AssetRole::Primary
            || asset.access_descriptor.is_some()
            || !asset.native_roles.iter().any(|role| role == "data")
            || !asset
                .media_type
                .as_deref()
                .is_some_and(|value| value.starts_with("image/tiff"))
        {
            return Err(error("STAC_ASSET_INVALID", "asset_semantics"));
        }
        match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog
                if !matches!(asset_key, "vv" | "vh" | "hh" | "hv") =>
            {
                return Err(error("STAC_ASSET_INVALID", "sentinel1_asset_key"));
            }
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog if asset_key != "data" => {
                return Err(error("STAC_ASSET_INVALID", "copdem_asset_key"));
            }
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => unreachable!(),
            _ => {}
        }
        let href = asset
            .url
            .as_deref()
            .ok_or_else(|| error("STAC_ASSET_INVALID", "asset_href"))?;
        let expected_name = validate_planetary_computer_reviewed_asset_identity(
            class,
            &product.product_id,
            asset_key,
            href,
        )
        .map_err(|_| error("STAC_IDENTITY_MISMATCH", "asset_identity"))?;
        if asset.file_name.as_deref() != Some(expected_name.as_str()) {
            return Err(error("STAC_IDENTITY_MISMATCH", "asset_filename"));
        }
    }

    // Provider transport metadata has already been validated; only reviewed
    // scientific identity crosses the production catalogue boundary.
    product.properties = Map::new();
    product.payload = Map::new();
    product.validate().map_err(|reason| {
        error(
            "STAC_RESPONSE_INVALID",
            &format!("provider_product:{reason}"),
        )
    })?;
    Ok(product)
}

fn error(code: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        "Planetary Computer 审核 COG 目录请求无效",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded reviewed MPC catalogue error is safe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_scope_is_exact_and_bounded() {
        let base = |logical_dataset: &str, collection: &str| CatalogSearchQuery {
            logical_dataset: logical_dataset.to_owned(),
            provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
            native_collection_id: collection.to_owned(),
            aoi_geojson: None,
            cursor: None,
            result_limit: Some(10),
            filters: Map::new(),
        };
        assert!(normalize_query(&base(
            MPC_SENTINEL1_GRD_LOGICAL_DATASET,
            MPC_SENTINEL1_GRD_COLLECTION_ID
        ))
        .is_ok());
        assert!(normalize_query(&base(
            MPC_COPDEM_GLO30_LOGICAL_DATASET,
            MPC_COPDEM_GLO30_COLLECTION_ID
        ))
        .is_ok());

        let mut unbounded = base(
            MPC_SENTINEL1_GRD_LOGICAL_DATASET,
            MPC_SENTINEL1_GRD_COLLECTION_ID,
        );
        unbounded.result_limit = None;
        assert!(normalize_query(&unbounded).is_err());

        let mut undeclared = base(
            MPC_COPDEM_GLO30_LOGICAL_DATASET,
            MPC_COPDEM_GLO30_COLLECTION_ID,
        );
        undeclared
            .filters
            .insert("cloud_cover_lte".to_owned(), Value::from(10));
        assert!(normalize_query(&undeclared).is_err());
    }
}
