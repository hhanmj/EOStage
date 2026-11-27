use std::collections::BTreeMap;

use insar_core::{AsfRawFeature, ProviderResult, SarRemoteSearchRequest, SarSearchQuery};
use insar_provider::asf::download::resolve_frozen_asf_asset;
use insar_provider::asf::metadata_mapper::map_sar_feature;
use insar_provider::asf::query::plan_sar_query;
use insar_provider::asf::search::{AsfProvider, MetadataTransport};
use serde_json::{json, Value};

struct UnusedTransport;

impl MetadataTransport for UnusedTransport {
    fn asf_geojson(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("invalid request must not reach the metadata transport")
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        panic!("invalid request must not reach the metadata transport")
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("invalid request must not reach the metadata transport")
    }
}

#[test]
fn query_and_search_errors_are_readable_chinese() {
    let query: SarSearchQuery = serde_json::from_value(json!({
        "dataset": "SENTINEL-1",
        "platforms": ["NOT-A-PLATFORM"]
    }))
    .unwrap();
    let error = plan_sar_query(&query).unwrap_err();
    assert_eq!(error.user_message, "ASF 检索条件不受当前数据集支持");

    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {"dataset": ""}
    }))
    .unwrap();
    let provider = AsfProvider::with_transport(UnusedTransport, "2026-07-16T00:00:00Z");
    let error = provider.search_sar(&request, &|| false).unwrap_err();
    assert_eq!(error.user_message, "ASF SAR 检索请求无效");
}

#[test]
fn metadata_mapper_error_is_readable_chinese() {
    let feature: AsfRawFeature = serde_json::from_value(json!({
        "type": "Feature",
        "properties": {}
    }))
    .unwrap();

    let error = map_sar_feature("SENTINEL-1", &feature).unwrap_err();
    assert_eq!(error.user_message, "ASF 结果缺少可靠的产品身份");
}

#[test]
fn download_resolver_error_is_readable_chinese() {
    let error = resolve_frozen_asf_asset(
        "product-1",
        "https://untrusted.example/product.zip",
        "product.zip",
        None,
    )
    .unwrap_err();

    assert_eq!(error.user_message, "ASF 下载地址必须是可信官方 HTTPS 地址");
}
