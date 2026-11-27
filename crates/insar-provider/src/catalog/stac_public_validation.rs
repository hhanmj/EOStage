//! Owner-approved public metadata probe, not a production Provider route.
//! No caller-supplied endpoint, credential, AOI, request body or download URL.

use super::*;

const ENDPOINT: &str = "https://planetarycomputer.microsoft.com/api/stac/v1/";
const COLLECTION: &str = "sentinel-2-l2a";
const PROVIDER: &str = "fixture.stac.planetary_computer";
const DATASET: &str = "optical.fixture.public-sentinel2-l2a";
const LANDSAT_COLLECTION: &str = "landsat-c2-l2";
const MPC_LANDSAT_PROVIDER: &str = "fixture.stac.planetary_computer.landsat";
const EARTH_SEARCH_LANDSAT_PROVIDER: &str = "fixture.stac.element84.earth_search.landsat";
const MPC_LANDSAT_ENDPOINT: &str = "https://planetarycomputer.microsoft.com/api/stac/v1/";
const EARTH_SEARCH_LANDSAT_ENDPOINT: &str = "https://earth-search.aws.element84.com/v1/";
const MPC_SENTINEL1_GRD_COLLECTION: &str = "sentinel-1-grd";
const MPC_SENTINEL1_GRD_PROVIDER: &str = "fixture.stac.planetary_computer.sentinel1-grd";
const MPC_SENTINEL1_GRD_DATASET: &str = "sar.fixture.public-sentinel1-grd";
const MPC_COPDEM_GLO30_COLLECTION: &str = "cop-dem-glo-30";
const MPC_COPDEM_GLO30_PROVIDER: &str = "fixture.stac.planetary_computer.copdem-glo30";
const MPC_COPDEM_GLO30_DATASET: &str = "dem.fixture.public-copdem-glo30";

pub struct StacPublicValidationClient {
    transport: StacTransport,
}

impl fmt::Debug for StacPublicValidationClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StacPublicValidationClient")
            .field(
                "scope",
                &"fixed anonymous Planetary Computer metadata probe",
            )
            .finish_non_exhaustive()
    }
}

impl StacPublicValidationClient {
    /// Construction never sends a request. This bounded manual probe cannot
    /// install a route or modify the normal Planetary Computer implementation.
    #[doc(hidden)]
    pub fn planetary_computer_sentinel2() -> ProviderResult<Self> {
        let route = StacRouteProfile {
            provider_id: PROVIDER.to_owned(),
            endpoint: ENDPOINT.to_owned(),
            search_path: "search".to_owned(),
            page_cap: 2,
            accepted_stac_versions: vec!["1.0.0".to_owned(), "1.1.0".to_owned()],
        };
        let binding = StacCollectionBinding {
            logical_dataset: DATASET.to_owned(),
            provider_id: PROVIDER.to_owned(),
            native_collection_id: COLLECTION.to_owned(),
            collection_profile_id: "fixture.stac.public-sentinel2.v1".to_owned(),
            identity_rule_id: "fixture.stac.item-id.v1".to_owned(),
            filename_rule_id: "fixture.stac.asset-filename.v1".to_owned(),
            // The existing descriptor has only this delivery enum. It is an
            // inactive test descriptor, not a claim that B04 is an archive.
            delivery_model: CatalogDeliveryModel::ArchiveFirst,
            source_kind: DataSourceKind::Sentinel2,
            product_kind: ProductKind::OpticalScene,
            production_enabled: false,
            product_id_property: None,
            display_name_property: None,
            queryables: Vec::new(),
            asset_role_bindings: vec![StacAssetRoleBinding {
                stac_asset_key: "B04".to_owned(),
                role: AssetRole::Primary,
                required: true,
                allowed_hosts: vec!["sentinel2l2a01.blob.core.windows.net".to_owned()],
            }],
        };
        // Reserve a non-production validation identity; real Item IDs and
        // collection identity remain unchanged in the mapped response.
        StacOfflineFixtureAdmission::new_for_test(route.clone(), binding.clone())?;
        let limits = StacTransportLimits {
            request_timeout: Duration::from_secs(20),
            overall_timeout: Duration::from_secs(50),
            max_pages: 2,
            max_records: 4,
            max_page_bytes: 512 * 1024,
            max_total_bytes: 1024 * 1024,
        };
        let socket = Url::parse("https://planetarycomputer.microsoft.com/")
            .map_err(|_| binding_error("fixed_public_origin"))?;
        Ok(Self {
            transport: StacTransport::new(route, binding, socket, limits)?,
        })
    }

    /// Fixed, search-only Landsat metadata probe. Assets are intentionally not
    /// mapped to ProviderAsset: this proves catalogue/science equivalence only.
    #[doc(hidden)]
    pub fn planetary_computer_landsat() -> ProviderResult<Self> {
        Self::fixed_landsat_probe(
            MPC_LANDSAT_PROVIDER,
            MPC_LANDSAT_ENDPOINT,
            "https://planetarycomputer.microsoft.com/",
            "fixture.stac.public-mpc-landsat.v1",
        )
    }

    /// Fixed Element 84 Earth Search metadata probe. No asset href is fetched.
    #[doc(hidden)]
    pub fn earth_search_landsat() -> ProviderResult<Self> {
        Self::fixed_landsat_probe(
            EARTH_SEARCH_LANDSAT_PROVIDER,
            EARTH_SEARCH_LANDSAT_ENDPOINT,
            "https://earth-search.aws.element84.com/",
            "fixture.stac.public-earth-search-landsat.v1",
        )
    }

    /// Fixed Sentinel-1 GRD catalogue/asset-shape probe. Polarization COGs are
    /// descriptive only here; no signed URL or transfer authority is frozen.
    #[doc(hidden)]
    pub fn planetary_computer_sentinel1_grd() -> ProviderResult<Self> {
        let route = fixed_mpc_route(MPC_SENTINEL1_GRD_PROVIDER);
        let binding = StacCollectionBinding {
            logical_dataset: MPC_SENTINEL1_GRD_DATASET.to_owned(),
            provider_id: MPC_SENTINEL1_GRD_PROVIDER.to_owned(),
            native_collection_id: MPC_SENTINEL1_GRD_COLLECTION.to_owned(),
            collection_profile_id: "fixture.stac.public-sentinel1-grd.v1".to_owned(),
            identity_rule_id: "fixture.stac.item-id.v1".to_owned(),
            filename_rule_id: "fixture.stac.asset-filename.v1".to_owned(),
            delivery_model: CatalogDeliveryModel::ArchiveFirst,
            source_kind: DataSourceKind::Sentinel1,
            product_kind: ProductKind::SarGrd,
            production_enabled: false,
            product_id_property: None,
            display_name_property: None,
            queryables: Vec::new(),
            asset_role_bindings: vec![
                StacAssetRoleBinding {
                    stac_asset_key: "thumbnail".to_owned(),
                    role: AssetRole::Thumbnail,
                    required: true,
                    allowed_hosts: vec!["sentinel1euwest.blob.core.windows.net".to_owned()],
                },
                StacAssetRoleBinding {
                    stac_asset_key: "vv".to_owned(),
                    role: AssetRole::Primary,
                    required: false,
                    allowed_hosts: vec!["sentinel1euwest.blob.core.windows.net".to_owned()],
                },
                StacAssetRoleBinding {
                    stac_asset_key: "vh".to_owned(),
                    role: AssetRole::Primary,
                    required: false,
                    allowed_hosts: vec!["sentinel1euwest.blob.core.windows.net".to_owned()],
                },
                StacAssetRoleBinding {
                    stac_asset_key: "hh".to_owned(),
                    role: AssetRole::Primary,
                    required: false,
                    allowed_hosts: vec!["sentinel1euwest.blob.core.windows.net".to_owned()],
                },
                StacAssetRoleBinding {
                    stac_asset_key: "hv".to_owned(),
                    role: AssetRole::Primary,
                    required: false,
                    allowed_hosts: vec!["sentinel1euwest.blob.core.windows.net".to_owned()],
                },
            ],
        };
        Self::fixed_mpc_cross_domain_probe(route, binding)
    }

    /// Fixed Copernicus DEM GLO-30 catalogue/asset-shape probe. The COG href
    /// remains stable discovery metadata and receives no execution authority.
    #[doc(hidden)]
    pub fn planetary_computer_cop_dem_glo30() -> ProviderResult<Self> {
        let route = fixed_mpc_route(MPC_COPDEM_GLO30_PROVIDER);
        let binding = StacCollectionBinding {
            logical_dataset: MPC_COPDEM_GLO30_DATASET.to_owned(),
            provider_id: MPC_COPDEM_GLO30_PROVIDER.to_owned(),
            native_collection_id: MPC_COPDEM_GLO30_COLLECTION.to_owned(),
            collection_profile_id: "fixture.stac.public-copdem-glo30.v1".to_owned(),
            identity_rule_id: "fixture.stac.item-id.v1".to_owned(),
            filename_rule_id: "fixture.stac.asset-filename.v1".to_owned(),
            delivery_model: CatalogDeliveryModel::ArchiveFirst,
            source_kind: DataSourceKind::Dem,
            product_kind: ProductKind::DemRaster,
            production_enabled: false,
            product_id_property: None,
            display_name_property: None,
            queryables: Vec::new(),
            asset_role_bindings: vec![StacAssetRoleBinding {
                stac_asset_key: "data".to_owned(),
                role: AssetRole::Primary,
                required: true,
                allowed_hosts: vec!["elevationeuwest.blob.core.windows.net".to_owned()],
            }],
        };
        Self::fixed_mpc_cross_domain_probe(route, binding)
    }

    fn fixed_mpc_cross_domain_probe(
        route: StacRouteProfile,
        binding: StacCollectionBinding,
    ) -> ProviderResult<Self> {
        StacOfflineFixtureAdmission::new_for_test(route.clone(), binding.clone())?;
        let limits = StacTransportLimits {
            request_timeout: Duration::from_secs(20),
            overall_timeout: Duration::from_secs(40),
            max_pages: 1,
            max_records: 1,
            max_page_bytes: 512 * 1024,
            max_total_bytes: 512 * 1024,
        };
        let socket = Url::parse("https://planetarycomputer.microsoft.com/")
            .map_err(|_| binding_error("fixed_public_origin"))?;
        Ok(Self {
            transport: StacTransport::new(route, binding, socket, limits)?,
        })
    }

    fn fixed_landsat_probe(
        provider_id: &str,
        endpoint: &str,
        socket_origin: &str,
        profile_id: &str,
    ) -> ProviderResult<Self> {
        let route = StacRouteProfile {
            provider_id: provider_id.to_owned(),
            endpoint: endpoint.to_owned(),
            search_path: "search".to_owned(),
            page_cap: 1,
            accepted_stac_versions: vec!["1.0.0".to_owned(), "1.1.0".to_owned()],
        };
        let binding = StacCollectionBinding {
            logical_dataset: "optical.fixture.public-landsat-c2-l2".to_owned(),
            provider_id: provider_id.to_owned(),
            native_collection_id: LANDSAT_COLLECTION.to_owned(),
            collection_profile_id: profile_id.to_owned(),
            identity_rule_id: "fixture.stac.landsat-item-id.v1".to_owned(),
            filename_rule_id: "fixture.stac.no-executable-assets.v1".to_owned(),
            delivery_model: CatalogDeliveryModel::ArchiveFirst,
            source_kind: DataSourceKind::Landsat,
            product_kind: ProductKind::OpticalScene,
            production_enabled: false,
            product_id_property: None,
            display_name_property: None,
            queryables: Vec::new(),
            asset_role_bindings: Vec::new(),
        };
        StacOfflineFixtureAdmission::new_for_test(route.clone(), binding.clone())?;
        let limits = StacTransportLimits {
            request_timeout: Duration::from_secs(20),
            overall_timeout: Duration::from_secs(40),
            max_pages: 2,
            max_records: 1,
            max_page_bytes: 512 * 1024,
            max_total_bytes: 1024 * 1024,
        };
        let socket = Url::parse(socket_origin).map_err(|_| binding_error("fixed_public_origin"))?;
        Ok(Self {
            transport: StacTransport::new(route, binding, socket, limits)?,
        })
    }

    /// At most two catalogue requests/four items. No imagery or asset fetch,
    /// authentication, proxy, redirect, retry, disk cache or Store mutation.
    pub fn search(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.transport.search(&probe_query(), cancelled)
    }

    #[doc(hidden)]
    pub fn search_landsat(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.transport
            .search(&landsat_probe_query(&self.transport.binding), cancelled)
    }

    #[doc(hidden)]
    pub fn search_sentinel1_grd(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.transport.search(
            &sentinel1_grd_probe_query(&self.transport.binding),
            cancelled,
        )
    }

    #[doc(hidden)]
    pub fn search_cop_dem_glo30(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.transport
            .search(&cop_dem_probe_query(&self.transport.binding), cancelled)
    }
}

fn fixed_mpc_route(provider_id: &str) -> StacRouteProfile {
    StacRouteProfile {
        provider_id: provider_id.to_owned(),
        endpoint: ENDPOINT.to_owned(),
        search_path: "search".to_owned(),
        page_cap: 1,
        accepted_stac_versions: vec!["1.0.0".to_owned(), "1.1.0".to_owned()],
    }
}

fn probe_query() -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: DATASET.to_owned(),
        provider_id: PROVIDER.to_owned(),
        native_collection_id: COLLECTION.to_owned(),
        aoi_geojson: Some(Map::from_iter([
            ("type".to_owned(), json!("Polygon")),
            (
                "coordinates".to_owned(),
                json!([[
                    [121.4, 25.0],
                    [121.6, 25.0],
                    [121.6, 25.2],
                    [121.4, 25.2],
                    [121.4, 25.0]
                ]]),
            ),
        ])),
        cursor: None,
        result_limit: Some(4),
        filters: Map::from_iter([(
            "datetime".to_owned(),
            json!({
                "start":"2025-01-01T00:00:00Z", "end":"2025-01-31T23:59:59Z"
            }),
        )]),
    }
}

fn landsat_probe_query(binding: &StacCollectionBinding) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: binding.logical_dataset.clone(),
        provider_id: binding.provider_id.clone(),
        native_collection_id: binding.native_collection_id.clone(),
        aoi_geojson: Some(Map::from_iter([
            ("type".to_owned(), json!("Polygon")),
            (
                "coordinates".to_owned(),
                json!([[
                    [111.0, 31.5],
                    [111.2, 31.5],
                    [111.2, 31.7],
                    [111.0, 31.7],
                    [111.0, 31.5]
                ]]),
            ),
        ])),
        cursor: None,
        result_limit: Some(1),
        filters: Map::from_iter([(
            "datetime".to_owned(),
            json!({
                "start":"2025-01-11T00:00:00Z", "end":"2025-01-11T23:59:59Z"
            }),
        )]),
    }
}

fn sentinel1_grd_probe_query(binding: &StacCollectionBinding) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: binding.logical_dataset.clone(),
        provider_id: binding.provider_id.clone(),
        native_collection_id: binding.native_collection_id.clone(),
        aoi_geojson: Some(Map::from_iter([
            ("type".to_owned(), json!("Polygon")),
            (
                "coordinates".to_owned(),
                json!([[
                    [121.3, 24.9],
                    [121.7, 24.9],
                    [121.7, 25.3],
                    [121.3, 25.3],
                    [121.3, 24.9]
                ]]),
            ),
        ])),
        cursor: None,
        result_limit: Some(1),
        filters: Map::from_iter([(
            "datetime".to_owned(),
            json!({
                "start":"2025-01-01T00:00:00Z", "end":"2025-01-31T23:59:59Z"
            }),
        )]),
    }
}

fn cop_dem_probe_query(binding: &StacCollectionBinding) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: binding.logical_dataset.clone(),
        provider_id: binding.provider_id.clone(),
        native_collection_id: binding.native_collection_id.clone(),
        aoi_geojson: Some(Map::from_iter([
            ("type".to_owned(), json!("Polygon")),
            (
                "coordinates".to_owned(),
                json!([[
                    [121.3, 24.9],
                    [121.7, 24.9],
                    [121.7, 25.3],
                    [121.3, 25.3],
                    [121.3, 24.9]
                ]]),
            ),
        ])),
        cursor: None,
        result_limit: Some(1),
        filters: Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_public_probe_cannot_change_scope_or_enable_production() {
        let client = StacPublicValidationClient::planetary_computer_sentinel2().unwrap();
        let q = probe_query();
        assert!(!client.transport.binding.production_enabled);
        assert_eq!(
            client.transport.socket.as_str(),
            "https://planetarycomputer.microsoft.com/"
        );
        assert_eq!(client.transport.limits.max_records, 4);
        assert_eq!(client.transport.limits.max_pages, 2);
        assert_eq!(client.transport.limits.max_total_bytes, 1024 * 1024);
        let request =
            build_stac_search_request(&client.transport.route, &client.transport.binding, &q)
                .unwrap();
        assert_eq!(request.url.as_str(), format!("{ENDPOINT}search"));
        assert_eq!(request.body.unwrap()["limit"], 2);
        assert_eq!(
            client.search(&|| true).unwrap_err().code,
            "STAC_SEARCH_CANCELLED"
        );
    }

    #[test]
    fn public_probe_still_rejects_redirected_paths_hosts_ports_and_credentials() {
        let client = StacPublicValidationClient::planetary_computer_sentinel2().unwrap();
        for href in [
            "http://planetarycomputer.microsoft.com/api/stac/v1/search",
            "https://planetarycomputer.microsoft.com:444/api/stac/v1/search",
            "https://other.example.test/api/stac/v1/search",
            "https://user@planetarycomputer.microsoft.com/api/stac/v1/search",
            "https://planetarycomputer.microsoft.com/api/sas/v1/token",
            "https://planetarycomputer.microsoft.com/api/stac/v1/search?access_token=fixture",
        ] {
            assert!(client
                .transport
                .validate_search_url(&Url::parse(href).unwrap())
                .is_err());
        }
    }

    #[test]
    fn fixed_landsat_probes_are_search_only_nonproduction_and_source_scoped() {
        let mpc = StacPublicValidationClient::planetary_computer_landsat().unwrap();
        let earth = StacPublicValidationClient::earth_search_landsat().unwrap();
        for client in [&mpc, &earth] {
            assert!(!client.transport.binding.production_enabled);
            assert!(client.transport.binding.asset_role_bindings.is_empty());
            assert_eq!(
                client.transport.binding.native_collection_id,
                LANDSAT_COLLECTION
            );
            assert_eq!(client.transport.limits.max_records, 1);
            assert_eq!(
                client.search_landsat(&|| true).unwrap_err().code,
                "STAC_SEARCH_CANCELLED"
            );
        }
        assert_eq!(
            mpc.transport.socket.as_str(),
            "https://planetarycomputer.microsoft.com/"
        );
        assert_eq!(
            earth.transport.socket.as_str(),
            "https://earth-search.aws.element84.com/"
        );
        assert_ne!(
            mpc.transport.binding.provider_id,
            earth.transport.binding.provider_id
        );
    }

    #[test]
    fn fixed_cross_domain_probes_preserve_sar_and_dem_product_kinds() {
        let sar = StacPublicValidationClient::planetary_computer_sentinel1_grd().unwrap();
        assert_eq!(sar.transport.binding.source_kind, DataSourceKind::Sentinel1);
        assert_eq!(sar.transport.binding.product_kind, ProductKind::SarGrd);
        assert!(!sar.transport.binding.production_enabled);
        assert_eq!(sar.transport.binding.asset_role_bindings.len(), 5);
        assert_eq!(
            sar.search_sentinel1_grd(&|| true).unwrap_err().code,
            "STAC_SEARCH_CANCELLED"
        );

        let dem = StacPublicValidationClient::planetary_computer_cop_dem_glo30().unwrap();
        assert_eq!(dem.transport.binding.source_kind, DataSourceKind::Dem);
        assert_eq!(dem.transport.binding.product_kind, ProductKind::DemRaster);
        assert!(!dem.transport.binding.production_enabled);
        assert_eq!(dem.transport.binding.asset_role_bindings.len(), 1);
        assert_eq!(
            dem.search_cop_dem_glo30(&|| true).unwrap_err().code,
            "STAC_SEARCH_CANCELLED"
        );
    }
}
