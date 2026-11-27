use std::time::Duration;

use insar_core::{CatalogSearchQuery, ProviderNetworkPolicy, ProviderResult};
use insar_provider::catalog::RouteUse;
use insar_provider::cdse::catalog_client::{CdseODataCatalogClient, CDSE_ODATA_CATALOGUE_BASE};
use insar_provider::cdse::loopback_client::LoopbackODataClient;
use insar_provider::cdse::query::{
    build_odata_request_for_use, validate_continuation_for_endpoint, wrap_continuation,
};
use serde_json::json;
use url::Url;

fn query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a",
        "provider_id": "copernicus.dataspace.odata",
        "native_collection_id": "SENTINEL-2",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
        },
        "cursor": null,
        "result_limit": 2,
        "filters": {
            "product_type": "S2MSI2A",
            "cloud_cover_lte": 4.2,
            "datetime": {
                "start": "2026-01-01T00:00:00.120Z",
                "end": "2026-01-02T00:00:00.120Z"
            }
        }
    }))
    .expect("valid offline Sentinel-2 query")
}

fn continuation_request(
    url: Url,
    use_case: RouteUse,
) -> ProviderResult<insar_provider::cdse::query::ODataRequest> {
    let query = query();
    let initial = build_odata_request_for_use(&query, use_case)?;
    let cursor = wrap_continuation(initial.fingerprint(), &url)?;
    build_odata_request_for_use(
        &CatalogSearchQuery {
            cursor: Some(cursor),
            ..query
        },
        use_case,
    )
}

#[test]
fn production_client_has_no_endpoint_override_and_cancels_before_public_traffic() {
    let source = include_str!("../src/cdse/catalog_client.rs");
    assert!(source.contains("pub fn new(timeout: Duration)"));
    assert!(!source.contains("pub fn new(endpoint"));
    assert_eq!(
        CDSE_ODATA_CATALOGUE_BASE,
        "https://catalogue.dataspace.copernicus.eu/odata/v1/"
    );

    let client = CdseODataCatalogClient::new(Duration::from_secs(1))
        .expect("fixed production catalogue endpoint");
    assert_eq!(client.network_policy(), ProviderNetworkPolicy::system());
    assert_eq!(
        client.search_all(&query(), &|| true).unwrap_err().code,
        "CDSE_ODATA_CANCELLED"
    );

    let mut wrong_provider = query();
    wrong_provider.provider_id = "unreviewed.provider".to_string();
    assert_eq!(
        client
            .search_all(&wrong_provider, &|| false)
            .unwrap_err()
            .code,
        "CDSE_ODATA_ROUTE_INVALID",
        "a production client rejects a mismatch before it can issue a request or fall back"
    );
}

#[test]
fn route_policy_is_explicit_for_production_direct_and_loopback_clients() {
    assert_eq!(
        CdseODataCatalogClient::new(Duration::from_secs(1))
            .expect("system production client")
            .network_policy(),
        ProviderNetworkPolicy::system()
    );
    assert_eq!(
        CdseODataCatalogClient::with_network_policy(
            Duration::from_secs(1),
            ProviderNetworkPolicy::direct(),
        )
        .expect("explicit direct production client")
        .network_policy(),
        ProviderNetworkPolicy::direct()
    );
    assert_eq!(
        LoopbackODataClient::new("http://127.0.0.1:38123/odata/v1/", Duration::from_secs(1))
            .expect("offline loopback client")
            .network_policy(),
        ProviderNetworkPolicy::direct()
    );
}

#[test]
fn production_scope_uses_exact_products_origin_and_only_trusted_numeric_continuation() {
    let endpoint = Url::parse(CDSE_ODATA_CATALOGUE_BASE).expect("fixed endpoint");
    let initial = build_odata_request_for_use(&query(), RouteUse::Production)
        .expect("production initial request");
    let initial_url = initial
        .initial_url(&endpoint)
        .expect("initial Products URL");
    assert_eq!(initial_url.scheme(), "https");
    assert_eq!(
        initial_url.host_str(),
        Some("catalogue.dataspace.copernicus.eu")
    );
    assert_eq!(initial_url.port_or_known_default(), Some(443));
    assert_eq!(initial_url.path(), "/odata/v1/Products");

    let mut valid = initial_url.clone();
    valid.query_pairs_mut().append_pair("$skip", "2");
    let continuation = continuation_request(valid.clone(), RouteUse::Production)
        .expect("production continuation request");
    assert_eq!(
        validate_continuation_for_endpoint(&continuation, &endpoint)
            .expect("same-origin numeric continuation")
            .skip,
        2
    );

    let mut cases = Vec::new();
    let mut http = valid.clone();
    http.set_scheme("http").expect("mutable scheme");
    cases.push(http);
    let mut host = valid.clone();
    host.set_host(Some("catalogue.dataspace.copernicus.eu.invalid"))
        .expect("mutable host");
    cases.push(host);
    let mut port = valid.clone();
    port.set_port(Some(444)).expect("mutable port");
    cases.push(port);
    let mut path = valid.clone();
    path.set_path("/odata/v1/Products/$value");
    cases.push(path);
    let mut userinfo = valid.clone();
    userinfo.set_username("user").expect("mutable username");
    cases.push(userinfo);
    let mut duplicate_skip = valid.clone();
    duplicate_skip.query_pairs_mut().append_pair("$skip", "3");
    cases.push(duplicate_skip);
    let mut credential = valid.clone();
    credential
        .query_pairs_mut()
        .append_pair("access_token", "synthetic");
    cases.push(credential);
    let mut unknown = valid;
    unknown.query_pairs_mut().append_pair("unexpected", "1");
    cases.push(unknown);

    for candidate in cases {
        let error = match continuation_request(candidate, RouteUse::Production) {
            Ok(continuation) => validate_continuation_for_endpoint(&continuation, &endpoint)
                .expect_err("untrusted continuation must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code, "CDSE_ODATA_CURSOR_UNTRUSTED");
    }
}

#[test]
fn shared_client_builder_keeps_redirect_cookie_and_authorization_boundaries_while_routing_explicitly(
) {
    let shared = include_str!("../src/cdse/odata_http.rs");
    let production = shared
        .split("#[cfg(test)]")
        .next()
        .expect("production OData implementation");
    assert!(production.contains("redirect(reqwest::redirect::Policy::none())"));
    assert!(production.contains("retry(reqwest::retry::never())"));
    assert!(production.contains("referer(false)"));
    // The route comes only from the shared proxy helper, never hand-written.
    assert!(production.contains("network_proxy::async_builder_with_proxy("));
    assert!(!production.contains("no_proxy("));
    assert!(!production.contains(".proxy("));
    assert!(!production.contains("Proxy::"));
    assert!(!production.to_ascii_lowercase().contains("authorization"));
    assert!(!production.to_ascii_lowercase().contains("cookie"));
    assert!(!production.contains("default_headers"));
}
