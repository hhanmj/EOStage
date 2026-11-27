use std::time::Duration;

use insar_core::{
    CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchQuery, CatalogSearchResponse,
    CatalogSearchStats, DataSourceKind, ProductKind, ProviderNetworkPolicy, ProviderProduct,
    ProviderResult,
};
use reqwest::Url;

use super::stac::StacTransport;
use super::{
    StacCollectionBinding, StacQueryOperator, StacQueryValueKind, StacQueryableBinding,
    StacRouteProfile, StacTransportLimits, EARTH_SEARCH_LANDSAT_COLLECTION_ID,
    EARTH_SEARCH_LANDSAT_FILENAME_RULE_ID, EARTH_SEARCH_LANDSAT_IDENTITY_RULE_ID,
    EARTH_SEARCH_LANDSAT_PROVIDER_ID, EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID,
    LANDSAT_C2_L2_LOGICAL_DATASET,
};
use crate::planetary_computer::{
    map_earth_search_landsat_feature, normalize_earth_search_landsat_query_with_exact_aoi,
};

pub const EARTH_SEARCH_STAC_ENDPOINT: &str = "https://earth-search.aws.element84.com/v1/";

pub struct EarthSearchLandsatCatalogClient {
    transport: StacTransport,
}

impl std::fmt::Debug for EarthSearchLandsatCatalogClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EarthSearchLandsatCatalogClient")
            .field("provider", &EARTH_SEARCH_LANDSAT_PROVIDER_ID)
            .field("scope", &"fixed anonymous Landsat search only")
            .finish()
    }
}

impl EarthSearchLandsatCatalogClient {
    pub fn new(network_policy: ProviderNetworkPolicy) -> ProviderResult<Self> {
        let route = StacRouteProfile {
            provider_id: EARTH_SEARCH_LANDSAT_PROVIDER_ID.to_owned(),
            endpoint: EARTH_SEARCH_STAC_ENDPOINT.to_owned(),
            search_path: "search".to_owned(),
            page_cap: 250,
            accepted_stac_versions: vec!["1.0.0".to_owned(), "1.1.0".to_owned()],
        };
        let binding = StacCollectionBinding {
            logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET.to_owned(),
            provider_id: EARTH_SEARCH_LANDSAT_PROVIDER_ID.to_owned(),
            native_collection_id: EARTH_SEARCH_LANDSAT_COLLECTION_ID.to_owned(),
            collection_profile_id: EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID.to_owned(),
            identity_rule_id: EARTH_SEARCH_LANDSAT_IDENTITY_RULE_ID.to_owned(),
            filename_rule_id: EARTH_SEARCH_LANDSAT_FILENAME_RULE_ID.to_owned(),
            delivery_model: CatalogDeliveryModel::ArchiveFirst,
            source_kind: DataSourceKind::Landsat,
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
                StacQueryableBinding {
                    sceneport_filter_key: "processing_levels".to_owned(),
                    stac_property: "landsat:correction".to_owned(),
                    operator: StacQueryOperator::In,
                    value_kind: StacQueryValueKind::StringList,
                },
            ],
            asset_role_bindings: Vec::new(),
        };
        let socket =
            Url::parse(EARTH_SEARCH_STAC_ENDPOINT).expect("fixed Earth Search URL is valid");
        let limits = StacTransportLimits {
            request_timeout: Duration::from_secs(10),
            overall_timeout: Duration::from_secs(30),
            max_pages: 128,
            max_records: insar_core::SEARCH_SAFETY_CAP as usize,
            max_page_bytes: 4 * 1024 * 1024,
            max_total_bytes: 16 * 1024 * 1024,
        };
        let mut transport =
            StacTransport::new_with_network_policy(route, binding, socket, limits, network_policy)?;
        transport.excluded_item_fields = vec!["links".to_owned(), "stac_extensions".to_owned()];
        Ok(Self { transport })
    }

    pub fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let clamped = insar_core::clamp_catalogue_search_query(query);
        let (compact_query, complex_exact) = super::stac::compact_complex_request_aoi(&clamped)?;
        let (normalized, exact) =
            normalize_earth_search_landsat_query_with_exact_aoi(&compact_query)?;
        let exact = complex_exact.or(exact);
        self.transport.search_with_mapper_exact(
            &normalized,
            cancelled,
            exact.as_ref(),
            |feature, _validated| map_earth_search_landsat_feature(feature),
        )
    }

    /// #22 B slice 3(b): stream the mapped products page-by-page. An undated
    /// or mid-search re-sorted query cannot decide its cap while streaming
    /// and fails closed with the typed `streaming_cap_order` reason; the
    /// caller retries the identical query on `search_catalog`.
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
        let (compact_query, complex_exact) = super::stac::compact_complex_request_aoi(&clamped)?;
        let (normalized, exact) =
            normalize_earth_search_landsat_query_with_exact_aoi(&compact_query)?;
        let exact = complex_exact.or(exact);
        let mut budget = self.transport.search_budget();
        self.transport.search_streaming_exact(
            &normalized,
            cancelled,
            &mut budget,
            exact.as_ref(),
            |feature, _validated| map_earth_search_landsat_feature(feature),
            sink,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planetary_computer::normalize_earth_search_landsat_query;
    use serde_json::{json, Value};

    fn query() -> CatalogSearchQuery {
        serde_json::from_value(json!({
            "logical_dataset": LANDSAT_C2_L2_LOGICAL_DATASET,
            "provider_id": EARTH_SEARCH_LANDSAT_PROVIDER_ID,
            "native_collection_id": EARTH_SEARCH_LANDSAT_COLLECTION_ID,
            "aoi_geojson": {"type":"Polygon","coordinates":[[[111.0,31.5],[111.2,31.5],[111.2,31.7],[111.0,31.5]]]},
            "cursor": null, "result_limit": 1,
            "filters": {
                "datetime":{"start":"2025-01-11T00:00:00Z","end":"2025-01-12T00:00:00Z"},
                "spacecraft":["landsat_8","landsat_9"],
                "processing_levels":["l2sp","l2sr"], "cloud_cover_lte": 35.5
            }
        })).unwrap()
    }

    #[test]
    fn earth_search_normalizes_all_filters_without_changing_query_scope() {
        let original = query();
        let normalized = normalize_earth_search_landsat_query(&original).unwrap();
        assert_eq!(normalized.aoi_geojson, original.aoi_geojson);
        assert_eq!(normalized.result_limit, original.result_limit);
        assert_eq!(normalized.filters["datetime"], original.filters["datetime"]);
        assert_eq!(
            normalized.filters["spacecraft"],
            json!(["landsat-8", "landsat-9"])
        );
        assert_eq!(
            normalized.filters["processing_levels"],
            json!(["L2SP", "L2SR"])
        );
        assert_eq!(normalized.filters["cloud_cover_lte"], 35.5);
        assert_eq!(
            original.filters["spacecraft"],
            json!(["landsat_8", "landsat_9"])
        );
        let mut subsecond = original.clone();
        subsecond.filters["datetime"]["end"] = json!("2025-01-11T00:00:00.100Z");
        assert!(normalize_earth_search_landsat_query(&subsecond).is_ok());
    }

    #[test]
    fn earth_search_rejects_malformed_filters_and_foreign_cursors_before_network() {
        for bad in [Value::Null, json!("35"), json!(true), json!(-1), json!(101)] {
            let mut q = query();
            q.filters["cloud_cover_lte"] = bad;
            assert!(normalize_earth_search_landsat_query(&q).is_err());
        }
        for (key, bad) in [
            ("spacecraft", json!(["landsat_unknown"])),
            ("processing_levels", json!(["l1tp"])),
            ("spacecraft", json!([])),
            ("processing_levels", json!(["l2sp", "l2sp"])),
            ("ignored_filter", json!(true)),
        ] {
            let mut q = query();
            q.filters.insert(key.to_owned(), bad);
            assert!(normalize_earth_search_landsat_query(&q).is_err(), "{key}");
        }
        let mut q = query();
        q.cursor = Some("foreign-provider-cursor".to_owned());
        assert!(normalize_earth_search_landsat_query(&q).is_err());
        let client = EarthSearchLandsatCatalogClient::new(ProviderNetworkPolicy::direct()).unwrap();
        assert_eq!(
            client.search_catalog(&query(), &|| true).unwrap_err().code,
            "STAC_SEARCH_CANCELLED"
        );
    }

    #[test]
    fn oversized_aoi_is_sent_as_an_envelope_only_with_the_exact_aoi_filter() {
        let mut oversized = query();
        oversized.aoi_geojson =
            Some(crate::catalog::spatial::dense_disjoint_china_like_aoi_for_test());
        let (normalized, exact) =
            normalize_earth_search_landsat_query_with_exact_aoi(&oversized).unwrap();
        let exact = exact.expect("an envelope request always carries the exact AOI filter");
        assert_eq!(
            normalized.aoi_geojson.as_ref().unwrap()["coordinates"][0]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        let gap = json!({"type":"Polygon","coordinates":[[[108.0,43.0],[109.0,43.0],[109.0,44.0],[108.0,44.0],[108.0,43.0]]]});
        let north_east = json!({"type":"Polygon","coordinates":[[[121.0,49.0],[122.0,49.0],[122.0,50.0],[121.0,50.0],[121.0,49.0]]]});
        assert!(!exact.intersects_footprint(gap.as_object().unwrap()));
        assert!(exact.intersects_footprint(north_east.as_object().unwrap()));
        // A small AOI is sent unchanged and needs no post-filter.
        let (small, exact) = normalize_earth_search_landsat_query_with_exact_aoi(&query()).unwrap();
        assert_eq!(small.aoi_geojson, query().aoi_geojson);
        assert!(exact.is_none());
    }
}
