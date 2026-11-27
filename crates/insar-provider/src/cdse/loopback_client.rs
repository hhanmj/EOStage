use std::net::IpAddr;
use std::time::Duration;

use insar_core::{
    CatalogSearchQuery, CatalogSearchResponse, ContractError, ProviderNetworkPolicy, ProviderResult,
};
use serde_json::{Map, Value};
use url::Url;

use super::odata_http::ODataHttpEngine;

pub struct LoopbackODataClient {
    engine: ODataHttpEngine,
}

fn error(code: &str, message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_string(), Value::String(reason.to_string()))]),
    )
    .expect("bounded loopback context is safe")
}

impl LoopbackODataClient {
    pub fn new(endpoint: &str, timeout: Duration) -> ProviderResult<Self> {
        let endpoint = Url::parse(endpoint).map_err(|_| {
            error(
                "CDSE_ODATA_LOOPBACK_INVALID",
                "离线 OData 端点无效",
                "parse",
            )
        })?;
        let loopback = endpoint
            .host_str()
            .and_then(|host| host.parse::<IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
        if !loopback
            || endpoint.scheme() != "http"
            || endpoint.path() != "/odata/v1/"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(error(
                "CDSE_ODATA_LOOPBACK_INVALID",
                "离线 OData 端点必须为受限 loopback HTTP 地址",
                "endpoint",
            ));
        }
        let engine = ODataHttpEngine::new(endpoint, timeout, ProviderNetworkPolicy::direct())
            .map_err(|_| {
                error(
                    "CDSE_ODATA_LOOPBACK_INVALID",
                    "离线 OData 客户端初始化失败",
                    "client",
                )
            })?;
        Ok(Self { engine })
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.engine.network_policy()
    }

    pub fn search_page(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &dyn Fn() -> bool,
    ) -> ProviderResult<CatalogSearchResponse> {
        self.engine.search_page(query, cancelled)
    }

    pub fn search_all(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &dyn Fn() -> bool,
    ) -> ProviderResult<CatalogSearchResponse> {
        self.engine.search_all(query, cancelled)
    }
}
