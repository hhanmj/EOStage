use std::time::Duration;

use insar_core::{
    CatalogCollectionDescriptor, CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats,
    ProviderNetworkPolicy, ProviderProduct, ProviderResult,
};
use url::Url;

use crate::catalog::RouteUse;

use super::odata_http::{odata_error, ODataHttpEngine};

pub const CDSE_ODATA_CATALOGUE_BASE: &str = "https://catalogue.dataspace.copernicus.eu/odata/v1/";
const MAX_PRODUCTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Fixed-origin CDSE discovery client. Construction does not make a public
/// request and the caller cannot override the reviewed catalogue origin.
pub struct CdseODataCatalogClient {
    engine: ODataHttpEngine,
}

impl CdseODataCatalogClient {
    pub fn new(timeout: Duration) -> ProviderResult<Self> {
        Self::with_network_policy(timeout, ProviderNetworkPolicy::system())
    }

    pub fn with_network_policy(
        timeout: Duration,
        network_policy: ProviderNetworkPolicy,
    ) -> ProviderResult<Self> {
        if timeout.is_zero() || timeout > MAX_PRODUCTION_TIMEOUT {
            return Err(odata_error(
                "CDSE_ODATA_CONFIG_INVALID",
                "CDSE OData 客户端超时配置无效",
                "timeout",
            ));
        }
        let endpoint = Url::parse(CDSE_ODATA_CATALOGUE_BASE).map_err(|_| {
            odata_error(
                "CDSE_ODATA_ENDPOINT_INVALID",
                "CDSE OData 目录端点无效",
                "parse",
            )
        })?;
        validate_catalogue_endpoint(&endpoint)?;
        Ok(Self {
            engine: ODataHttpEngine::new_for_route(
                endpoint,
                timeout,
                network_policy,
                RouteUse::Production,
            )?,
        })
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.engine.network_policy()
    }

    pub fn search_all(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &dyn Fn() -> bool,
    ) -> ProviderResult<CatalogSearchResponse> {
        self.engine.search_all(query, cancelled)
    }

    pub fn search_streaming<S, P>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &dyn Fn() -> bool,
        sink: &mut S,
        progress: &P,
    ) -> ProviderResult<(CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
        P: Fn(crate::asf::catalog_client::CatalogSearchProviderProgress),
    {
        self.engine
            .search_streaming(query, cancelled, sink, progress)
    }
}

fn validate_catalogue_endpoint(endpoint: &Url) -> ProviderResult<()> {
    if endpoint.scheme() != "https"
        || endpoint.host_str() != Some("catalogue.dataspace.copernicus.eu")
        || endpoint.port_or_known_default() != Some(443)
        || endpoint.path() != "/odata/v1/"
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(odata_error(
            "CDSE_ODATA_ENDPOINT_INVALID",
            "CDSE OData 目录端点无效",
            "endpoint",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_endpoint_policy_is_exact_and_timeout_is_bounded() {
        let endpoint = Url::parse(CDSE_ODATA_CATALOGUE_BASE).expect("reviewed endpoint");
        validate_catalogue_endpoint(&endpoint).expect("reviewed endpoint is accepted");

        for candidate in [
            "http://catalogue.dataspace.copernicus.eu/odata/v1/",
            "https://catalogue.dataspace.copernicus.eu.invalid/odata/v1/",
            "https://catalogue.dataspace.copernicus.eu:444/odata/v1/",
            "https://catalogue.dataspace.copernicus.eu/odata/v1/Products/",
            "https://user@catalogue.dataspace.copernicus.eu/odata/v1/",
            "https://catalogue.dataspace.copernicus.eu/odata/v1/?override=1",
            "https://catalogue.dataspace.copernicus.eu/odata/v1/#fragment",
        ] {
            let endpoint = Url::parse(candidate).expect("syntactically valid test endpoint");
            assert_eq!(
                validate_catalogue_endpoint(&endpoint).unwrap_err().code,
                "CDSE_ODATA_ENDPOINT_INVALID",
                "{candidate}"
            );
        }

        for timeout in [Duration::ZERO, Duration::from_secs(61)] {
            let error = CdseODataCatalogClient::new(timeout)
                .err()
                .expect("out-of-bound timeout is rejected");
            assert_eq!(error.code, "CDSE_ODATA_CONFIG_INVALID");
        }
    }

    #[test]
    fn catalogue_client_defaults_to_system_and_accepts_explicit_direct_routing() {
        assert_eq!(
            CdseODataCatalogClient::new(Duration::from_secs(1))
                .expect("system client")
                .network_policy(),
            ProviderNetworkPolicy::system()
        );
        assert_eq!(
            CdseODataCatalogClient::with_network_policy(
                Duration::from_secs(1),
                ProviderNetworkPolicy::direct(),
            )
            .expect("direct client")
            .network_policy(),
            ProviderNetworkPolicy::direct()
        );
    }
}
