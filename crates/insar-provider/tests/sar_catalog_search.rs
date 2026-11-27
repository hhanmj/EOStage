use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use insar_core::{ContractError, ProviderResult, SarRemoteSearchRequest, SarSearchCountRequest};
use insar_provider::asf::search::{
    AsfProvider, MetadataTransport, SarSearchProviderProgress, SarSearchProviderProgressStage,
};
use serde_json::{json, Value};

struct FixtureTransport {
    params: Mutex<Vec<BTreeMap<String, String>>>,
}

struct LargeCountTransport {
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
}

struct SplitIdentityTransport {
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
}

struct RecoveringLargeCatalogTransport {
    responses: Mutex<VecDeque<ProviderResult<Value>>>,
    count_params: Mutex<Vec<BTreeMap<String, String>>>,
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
}

struct TerminalClientErrorTransport {
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
}

struct RetryableSmallSearchErrorTransport {
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
}

struct BoundedDuplicateProductTransport {
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
}

struct SaturatedFiniteSeasatTransport {
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
    count_calls: AtomicU64,
    fetched_records: AtomicU64,
    response_index: AtomicU64,
}

struct DirectPartialFiniteSeasatTransport {
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
    fetched_records: AtomicU64,
}

struct MissingCountTransport;
struct ZeroCountTransport;

struct ScriptedAllResultsTransport {
    responses: Mutex<VecDeque<Value>>,
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
}

impl MetadataTransport for ScriptedAllResultsTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.geojson_params.lock().unwrap().push(params.clone());
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted all-results metadata response"))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(Some(450))
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("Sentinel-1 all-results search must not route to CMR")
    }
}

fn sentinel_feature(index: usize) -> Value {
    json!({
        "type": "Feature",
        "properties": {
            "sceneName": format!(
                "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_{:06}_064ABC_{:04X}",
                52_000 + index,
                index,
            ),
            "pathNumber": "88",
            "frameNumber": "456",
            "flightDirection": "ASCENDING"
        },
        "geometry": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 31.0], [110.0, 30.0]]]
        }
    })
}

fn sentinel_feature_collection(indices: impl IntoIterator<Item = usize>) -> Value {
    json!({
        "type": "FeatureCollection",
        "features": indices.into_iter().map(sentinel_feature).collect::<Vec<_>>()
    })
}

fn seasat_feature_at(index: u64, start_time: &str) -> Value {
    // Mirrors the recorded SEASAT row shape (SS_01502_STD_F2536): the SEASAT
    // mapper classifies a single .h5 primary from the file-fact map.
    json!({
        "type": "Feature",
        "properties": {
            "productID": format!("SS_FINITE_{index:06}"),
            "granuleName": format!("SS_FINITE_{index:06}"),
            "fileID": format!("SS_FINITE_{index:06}"),
            "fileName": format!("SS_FINITE_{index:06}.h5"),
            "processingLevel": "L1",
            "platform": "SEASAT 1",
            "startTime": start_time,
            "url": format!("https://datapool.asf.alaska.edu/SEASAT/SS_FINITE_{index:06}.h5"),
            "bytes": {
                format!("SS_FINITE_{index:06}.h5"): {"format": "hdf5", "bytes": 4096}
            }
        }
    })
}

fn seasat_feature(index: u64) -> Value {
    seasat_feature_at(index, "1978-07-15T00:00:00Z")
}

fn scripted_all_results_transport() -> ScriptedAllResultsTransport {
    ScriptedAllResultsTransport {
        responses: Mutex::new(VecDeque::from([
            sentinel_feature_collection(0..200),
            sentinel_feature_collection(0..150),
            sentinel_feature_collection(150..350),
            sentinel_feature_collection(150..300),
            sentinel_feature_collection(300..450),
        ])),
        geojson_params: Mutex::new(Vec::new()),
    }
}

fn all_results_sentinel_request() -> SarRemoteSearchRequest {
    serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "start": "2024-01-01T00:00:00Z",
            "end": "2024-01-10T23:59:59Z",
            "max_results": null
        }
    }))
    .unwrap()
}

impl MetadataTransport for ZeroCountTransport {
    fn asf_geojson(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("zero matching results must not issue a metadata request")
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(Some(0))
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("zero matching ASF results must not route to CMR")
    }
}

impl MetadataTransport for MissingCountTransport {
    fn asf_geojson(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("all-results mode must fail before metadata retrieval when count is unavailable")
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(None)
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("all-results ASF search must not route to CMR")
    }
}

impl MetadataTransport for RetryableSmallSearchErrorTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.geojson_params.lock().unwrap().push(params.clone());
        Err(ContractError::new(
            "ASF002",
            "元数据服务暂时未能完成请求",
            serde_json::Map::new(),
        )
        .unwrap())
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(Some(867))
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("bounded desktop Sentinel-1 errors must not route to CMR")
    }
}

impl MetadataTransport for BoundedDuplicateProductTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.geojson_params.lock().unwrap().push(params.clone());
        Ok(sentinel_feature_collection(std::iter::repeat_n(0, 120)))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        panic!("bounded numeric search must not request a remote total")
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("bounded Sentinel-1 search must not route to CMR")
    }
}

impl MetadataTransport for SaturatedFiniteSeasatTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        let call_index = {
            let mut calls = self.geojson_params.lock().unwrap();
            let index = calls.len();
            calls.push(params.clone());
            index
        };
        if call_index == 0 {
            return Err(ContractError::new(
                "ASF002",
                "scripted broad-window saturation recovery",
                serde_json::Map::new(),
            )
            .unwrap());
        }
        let limit = params["maxResults"].parse::<u64>().unwrap();
        let response_index = self.response_index.fetch_add(1, Ordering::Relaxed);
        let first = response_index.saturating_mul(limit);
        self.fetched_records.fetch_add(limit, Ordering::Relaxed);
        Ok(json!({
            "type": "FeatureCollection",
            "features": (first..first.saturating_add(limit))
                .map(seasat_feature)
                .collect::<Vec<_>>()
        }))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        self.count_calls.fetch_add(1, Ordering::Relaxed);
        Ok(Some(549_855))
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("bounded finite ERS search must not route to CMR")
    }
}

impl MetadataTransport for DirectPartialFiniteSeasatTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        let call_index = {
            let mut calls = self.geojson_params.lock().unwrap();
            let index = calls.len();
            calls.push(params.clone());
            index
        };
        let (first, count, start_time) = if call_index == 0 {
            (0, 2_000, "1978-08-15T00:00:00Z")
        } else if params["end"].as_str() < "1978-08-15T00:00:00Z" {
            (
                2_000,
                params["maxResults"].parse::<u64>().unwrap(),
                "1978-04-15T00:00:00Z",
            )
        } else {
            (0, 0, "1978-10-15T00:00:00Z")
        };
        self.fetched_records.fetch_add(count, Ordering::Relaxed);
        Ok(json!({
            "type": "FeatureCollection",
            "features": (first..first + count)
                .map(|index| seasat_feature_at(index, start_time))
                .collect::<Vec<_>>()
        }))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        panic!("finite ERS search must not synchronously count the remote population")
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("bounded finite ERS search must not route to CMR")
    }
}

impl MetadataTransport for TerminalClientErrorTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.geojson_params.lock().unwrap().push(params.clone());
        Err(ContractError::new(
            "ASF002",
            "元数据服务拒绝请求",
            serde_json::Map::from_iter([("status".to_owned(), Value::from(400))]),
        )
        .unwrap())
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(None)
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("terminal ASF client errors must not route to CMR")
    }
}

impl MetadataTransport for RecoveringLargeCatalogTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.geojson_params.lock().unwrap().push(params.clone());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted large-catalog response")
    }

    fn asf_count(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        self.count_params.lock().unwrap().push(params.clone());
        Ok(Some(545_894))
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("generic large-catalog recovery must not route to CMR")
    }
}

impl MetadataTransport for SplitIdentityTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.geojson_params.lock().unwrap().push(params.clone());
        if !params.contains_key("start") {
            return Ok(json!({"type": "FeatureCollection", "features": []}));
        }
        Ok(json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": {
                        "sceneName": "ALOS-SHARED-DISPLAY",
                        "productID": "ALOS-SPLIT-L11",
                        "processingLevel": "L1.1",
                        "fileID": "ALOS-SPLIT-L11-DATA",
                        "fileName": "ALOS-SPLIT-L11-DATA.img"
                    }
                },
                {
                    "type": "Feature",
                    "properties": {
                        "sceneName": "ALOS-SHARED-DISPLAY",
                        "productID": "ALOS-SPLIT-L15",
                        "processingLevel": "L1.5",
                        "fileID": "ALOS-SPLIT-L15-DATA",
                        "fileName": "ALOS-SPLIT-L15-DATA.img"
                    }
                },
                {
                    "type": "Feature",
                    "properties": {
                        "sceneName": "ALOS-SHARED-DISPLAY",
                        "productID": "ALOS-SPLIT-L11",
                        "processingLevel": "L1.1",
                        "fileID": "ALOS-SPLIT-L11-DATA",
                        "fileName": "ALOS-SPLIT-L11-DATA.img"
                    }
                }
            ]
        }))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(Some(12_000))
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("generic split search must not route to CMR")
    }
}

impl MetadataTransport for LargeCountTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.geojson_params.lock().unwrap().push(params.clone());
        Ok(json!({"type": "FeatureCollection", "features": []}))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(Some(51_594))
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("a successful partitioned catalogue search must not use CMR")
    }
}

impl MetadataTransport for FixtureTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.params.lock().unwrap().push(params.clone());
        Ok(json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": {
                    "granuleName": "ALPSRP123",
                    "fileID": "ALPSRP123-L1.1",
                    "platform": "ALOS",
                    "instrument": "PALSAR",
                    "processingLevel": "L1.1",
                    "beamModeType": "FBS",
                    "polarization": "HH",
                    "fileName": "ALPSRP123.tar.gz",
                    "url": "https://datapool.asf.alaska.edu/ALOS/ALPSRP123.tar.gz",
                    "bytes": 4096
                }
            }]
        }))
    }

    fn asf_count(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        self.params.lock().unwrap().push(params.clone());
        Ok(Some(1))
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("generic catalogue search must not silently fall back to unreviewed CMR mappings")
    }
}

#[test]
fn live_catalog_path_translates_fetches_and_maps_non_sentinel_products() {
    let provider = AsfProvider::with_transport(
        FixtureTransport {
            params: Mutex::new(Vec::new()),
        },
        "2026-07-14T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "ALOS PALSAR",
            "platforms": ["ALOS"],
            "instrument": "PALSAR",
            "processing_levels": ["L1.1"],
            "beam_modes": ["FBS"],
            "polarizations": ["HH"],
            "max_results": 10
        }
    }))
    .unwrap();

    let progress = Mutex::new(Vec::<SarSearchProviderProgress>::new());
    let response = provider
        .search_sar_with_progress(&request, &|| false, &|update| {
            progress.lock().unwrap().push(update);
        })
        .unwrap();

    assert_eq!(response.products.len(), 1);
    let progress = progress.lock().unwrap();
    assert_eq!(progress.last().map(|item| item.fetched_count), Some(1));
    assert_eq!(progress.last().and_then(|item| item.total_count), Some(10));
    assert_eq!(response.products[0].dataset, "ALOS PALSAR");
    assert_eq!(
        response.products[0].file_name.as_deref(),
        Some("ALPSRP123.tar.gz")
    );
    assert_eq!(response.stats.source, "ASF");
    assert!(response.errors.is_empty());
    assert_eq!(
        provider.transport().params.lock().unwrap()[0]["dataset"],
        "ALOS PALSAR"
    );
}

#[test]
fn catalog_count_uses_the_same_translator_without_fetching_products() {
    let provider = AsfProvider::with_transport(
        FixtureTransport {
            params: Mutex::new(Vec::new()),
        },
        "2026-07-14T00:00:00Z",
    );
    let request: SarSearchCountRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "ALOS PALSAR",
            "platforms": ["ALOS"],
            "processing_levels": ["L1.1"],
            "max_results": 20
        }
    }))
    .unwrap();

    let response = provider.count_sar(&request, &|| false).unwrap();

    assert_eq!(response.total_count, Some(1));
    let params = provider.transport().params.lock().unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0]["dataset"], "ALOS PALSAR");
}

#[test]
fn unbounded_large_catalog_search_caps_target_and_preserves_total_count() {
    let provider = AsfProvider::with_transport(
        LargeCountTransport {
            geojson_params: Mutex::new(Vec::new()),
        },
        "2026-07-14T00:00:00Z",
    );
    // SEASAT keeps the anonymous /param archive-window path; the restricted
    // legacy missions (ERS/JERS-1/RADARSAT-1) now route to authenticated CMR.
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SEASAT",
            "start": "1978-07-01T00:00:00Z",
            "end": "1978-07-02T23:59:59Z",
            "max_results": null
        }
    }))
    .unwrap();

    let response = provider.search_sar(&request, &|| false).unwrap();

    assert_eq!(response.stats.requested_limit, 10_000);
    assert_eq!(response.stats.total_count, Some(51_594));
    assert_eq!(response.stats.source, "ASF_BATCH");
    assert_eq!(
        provider
            .transport()
            .geojson_params
            .lock()
            .unwrap()
            .iter()
            .map(|params| params["maxResults"].as_str())
            .collect::<Vec<_>>()
            .as_slice(),
        ["200"]
    );
}

#[test]
fn split_catalog_maps_before_product_deduplication_and_reports_diagnostics() {
    let provider = AsfProvider::with_transport(
        SplitIdentityTransport {
            geojson_params: Mutex::new(Vec::new()),
        },
        "2026-07-14T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "ALOS PALSAR",
            "start": "2024-01-01T00:00:00Z",
            "end": "2024-01-02T23:59:59Z",
            "max_results": null
        }
    }))
    .unwrap();

    let response = provider.search_sar(&request, &|| false).unwrap();

    assert_eq!(response.stats.source, "ASF_BATCH");
    assert_eq!(response.products.len(), 2);
    assert_eq!(response.stats.matched_count, Some(12_000));
    assert_eq!(response.stats.fetched_record_count, Some(3));
    assert_eq!(response.stats.replayed_record_count, Some(1));
    assert_eq!(response.stats.mapped_record_count, Some(2));
    assert_eq!(response.stats.mapping_error_count, Some(0));
    assert_eq!(response.stats.aggregated_asset_record_count, Some(2));
    assert_eq!(response.stats.unique_provider_product_count, Some(2));
    assert_eq!(response.stats.asset_count, Some(2));
    assert_eq!(response.stats.identity_conflict_count, Some(0));
    assert_eq!(response.stats.page_size, Some(200));
    assert_eq!(response.stats.truncated, Some(true));
    assert_eq!(response.stats.returned_count, 2);
    assert!(response.stats.item_errors.is_empty());
}

#[test]
fn blank_quantity_requires_a_real_provider_total_instead_of_silently_loading_500() {
    let provider = AsfProvider::with_transport(MissingCountTransport, "2026-08-22T00:00:00Z");
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "max_results": null
        }
    }))
    .unwrap();

    let error = provider.search_sar(&request, &|| false).unwrap_err();

    assert_eq!(error.code, "ASF002");
    assert_eq!(
        error.debug_context.get("reason"),
        Some(&Value::from("all_results_count_required")),
    );
}

#[test]
fn all_results_zero_count_completes_without_a_metadata_request() {
    let provider = AsfProvider::with_transport(ZeroCountTransport, "2026-08-22T00:00:00Z");
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "max_results": null
        }
    }))
    .unwrap();

    let response = provider.search_sar(&request, &|| false).unwrap();

    assert!(response.products.is_empty());
    assert_eq!(response.stats.total_count, Some(0));
    assert_eq!(response.stats.returned_count, 0);
    assert_eq!(response.stats.truncated, Some(false));
}

#[test]
fn all_results_search_publishes_real_fetched_over_total_progress() {
    let provider = AsfProvider::with_transport(
        FixtureTransport {
            params: Mutex::new(Vec::new()),
        },
        "2026-08-22T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "ALOS PALSAR",
            "platforms": ["ALOS"],
            "processing_levels": ["L1.1"],
            "start": "2026-08-21T00:00:00Z",
            "end": "2026-08-22T00:00:00Z",
            "max_results": null
        }
    }))
    .unwrap();
    let progress = Mutex::new(Vec::<SarSearchProviderProgress>::new());

    let response = provider
        .search_sar_with_progress(&request, &|| false, &|update| {
            progress.lock().unwrap().push(update);
        })
        .unwrap();

    assert_eq!(response.stats.total_count, Some(1));
    assert_eq!(response.products.len(), 1);
    let admitted = progress
        .lock()
        .unwrap()
        .iter()
        .filter(|item| item.unique_admitted_count > 0)
        .map(|item| (item.fetched_count, item.total_count))
        .collect::<Vec<_>>();
    assert_eq!(admitted.last(), Some(&(1, Some(1))));
    assert!(provider
        .transport()
        .params
        .lock()
        .unwrap()
        .iter()
        .skip(1)
        .all(|params| params["maxResults"].parse::<u64>().unwrap() <= 200));
}

#[test]
fn sentinel_all_results_retrieves_every_match_in_bounded_batches_with_measured_progress() {
    let provider =
        AsfProvider::with_transport(scripted_all_results_transport(), "2026-08-22T00:00:00Z");
    let progress = Mutex::new(Vec::<SarSearchProviderProgress>::new());

    let response = provider
        .search_sar_with_progress(&all_results_sentinel_request(), &|| false, &|update| {
            progress.lock().unwrap().push(update);
        })
        .unwrap();

    assert_eq!(response.products.len(), 450);
    assert_eq!(response.stats.total_count, Some(450));
    assert_eq!(response.stats.truncated, Some(false));
    assert_eq!(response.stats.page_size, Some(200));
    let admitted = progress
        .lock()
        .unwrap()
        .iter()
        .filter(|item| item.stage == SarSearchProviderProgressStage::AdmittingResults)
        .map(|item| (item.unique_admitted_count, item.total_count))
        .collect::<Vec<_>>();
    assert_eq!(
        admitted,
        [(150, Some(450)), (300, Some(450)), (450, Some(450))]
    );
    let progress = progress.lock().unwrap();
    let split = progress
        .iter()
        .find(|item| item.stage == SarSearchProviderProgressStage::SplittingTransport)
        .expect("saturated parent publishes a split event");
    assert!(split.provider_request_count > 0);
    assert!(split.raw_fetched_count > split.unique_admitted_count);
    assert_eq!(provider.transport().geojson_params.lock().unwrap().len(), 5);
    assert!(provider
        .transport()
        .geojson_params
        .lock()
        .unwrap()
        .iter()
        .all(|params| params["maxResults"].parse::<u64>().unwrap() <= 200));
}

#[test]
fn sentinel_all_results_cancellation_stops_before_the_next_batch() {
    let provider =
        AsfProvider::with_transport(scripted_all_results_transport(), "2026-08-22T00:00:00Z");
    let cancelled = AtomicBool::new(false);

    let error = provider
        .search_sar_with_progress(
            &all_results_sentinel_request(),
            &|| cancelled.load(Ordering::SeqCst),
            &|update| {
                if update.fetched_count >= 150 {
                    cancelled.store(true, Ordering::SeqCst);
                }
            },
        )
        .unwrap_err();

    assert_eq!(error.code, "DL001");
    assert_eq!(provider.transport().geojson_params.lock().unwrap().len(), 2);
}

#[test]
fn finite_large_catalog_recovers_with_bounded_time_segments_without_foreground_count() {
    let direct_error = ContractError::new(
        "ASF002",
        "元数据服务暂时未能完成请求",
        serde_json::Map::new(),
    )
    .unwrap();
    let provider = AsfProvider::with_transport(
        RecoveringLargeCatalogTransport {
            responses: Mutex::new(VecDeque::from([
                Err(direct_error.clone()),
                Err(direct_error),
                Ok(json!({
                    "type": "FeatureCollection",
                    "features": [{
                        "type": "Feature",
                        "properties": {
                            "granuleName": "ALOS-RECOVERED-001",
                            "fileID": "ALOS-RECOVERED-DATA",
                            "fileName": "ALOS-RECOVERED-DATA.img",
                            "processingLevel": "L1.1"
                        }
                    }]
                })),
                Ok(json!({"type": "FeatureCollection", "features": []})),
            ])),
            count_params: Mutex::new(Vec::new()),
            geojson_params: Mutex::new(Vec::new()),
        },
        "2026-07-14T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "ALOS PALSAR",
            "start": "2007-01-01T00:00:00Z",
            "end": "2007-01-02T23:59:59Z",
            "max_results": 2500
        }
    }))
    .unwrap();

    let response = provider.search_sar(&request, &|| false).unwrap();

    assert_eq!(response.stats.source, "ASF_BATCH");
    assert_eq!(response.stats.matched_count, None);
    assert_eq!(response.stats.fetched_record_count, Some(1));
    assert_eq!(response.stats.unique_provider_product_count, Some(1));
    assert_eq!(response.stats.returned_count, 1);
    assert_eq!(provider.transport().count_params.lock().unwrap().len(), 0);
    assert_eq!(
        provider.transport().geojson_params.lock().unwrap()[0]["maxResults"],
        "2500",
        "the non-AOI metadata request remains bounded by the explicit finite target"
    );
    assert_eq!(response.products[0].product_id, "ALOS-RECOVERED-DATA");
    assert_eq!(
        response.products[0].scene_id.as_deref(),
        Some("ALOS-RECOVERED-001")
    );
}

#[test]
fn deterministic_asf_400_does_not_expand_into_split_window_requests() {
    let provider = AsfProvider::with_transport(
        TerminalClientErrorTransport {
            geojson_params: Mutex::new(Vec::new()),
        },
        "2026-08-22T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[108.0, 29.0], [116.0, 29.0], [116.0, 34.0], [108.0, 34.0], [108.0, 29.0]]]
            },
            "start": "2024-01-01",
            "end": "2025-01-01",
            "max_results": 20
        }
    }))
    .unwrap();

    let error = provider.search_sar(&request, &|| false).unwrap_err();

    assert_eq!(error.code, "ASF002");
    assert_eq!(error.debug_context.get("status"), Some(&Value::from(400)));
    assert_eq!(provider.transport().geojson_params.lock().unwrap().len(), 1);
}

#[test]
fn retryable_failure_for_a_30_or_300_result_desktop_search_is_terminal_without_split_amplification()
{
    for (requested, expected_query_limit) in [(30, "120"), (300, "900")] {
        let provider = AsfProvider::with_transport(
            RetryableSmallSearchErrorTransport {
                geojson_params: Mutex::new(Vec::new()),
            },
            "2026-08-22T00:00:00Z",
        );
        let request: SarRemoteSearchRequest = serde_json::from_value(json!({
            "query": {
                "dataset": "SENTINEL-1",
                "processing_levels": ["SLC"],
                "aoi_geojson": {
                    "type": "Polygon",
                    "coordinates": [[[-125.0, 25.0], [-66.0, 25.0], [-66.0, 49.0], [-125.0, 49.0], [-125.0, 25.0]]]
                },
                "start": "2026-08-04",
                "end": "2026-08-22",
                "max_results": requested
            }
        }))
        .unwrap();

        let error = provider.search_sar(&request, &|| false).unwrap_err();

        assert_eq!(error.code, "ASF002");
        let calls = provider.transport().geojson_params.lock().unwrap();
        assert_eq!(
            calls.len(),
            1,
            "a {requested}-result desktop request must never enter split recovery",
        );
        assert_eq!(
            calls[0]["maxResults"], expected_query_limit,
            "the existing bounded AOI overfetch remains intact without opening split recovery",
        );
    }
}

#[test]
fn bounded_numeric_search_does_not_refill_after_one_successful_metadata_response() {
    let provider = AsfProvider::with_transport(
        BoundedDuplicateProductTransport {
            geojson_params: Mutex::new(Vec::new()),
        },
        "2026-08-22T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[115.4, 39.4], [117.6, 39.4], [117.6, 41.1], [115.4, 41.1], [115.4, 39.4]]]
            },
            "max_results": 30
        }
    }))
    .unwrap();

    let progress = Mutex::new(Vec::<SarSearchProviderProgress>::new());
    let response = provider
        .search_sar_with_progress(&request, &|| false, &|update| {
            progress.lock().unwrap().push(update);
        })
        .unwrap();

    assert_eq!(response.products.len(), 1);
    let progress = progress.lock().unwrap();
    assert_eq!(progress.last().map(|item| item.fetched_count), Some(1));
    assert_eq!(progress.last().and_then(|item| item.total_count), Some(30));
    let calls = provider.transport().geojson_params.lock().unwrap();
    assert_eq!(
        calls.len(),
        1,
        "numeric bounded search is one successful metadata request"
    );
    assert_eq!(calls[0]["maxResults"], "120");
}

#[test]
fn large_aoi_finite_seasat_search_is_bounded_by_the_working_set_instead_of_remote_total() {
    let provider = AsfProvider::with_transport(
        SaturatedFiniteSeasatTransport {
            geojson_params: Mutex::new(Vec::new()),
            count_calls: AtomicU64::new(0),
            fetched_records: AtomicU64::new(0),
            response_index: AtomicU64::new(0),
        },
        "2026-08-22T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SEASAT",
            "platforms": ["SEASAT 1"],
            "aoi_geojson": {
                "type": "MultiPolygon",
                "coordinates": [
                    [[[-170.0, -60.0], [-10.0, -60.0], [-10.0, 70.0], [-170.0, 70.0], [-170.0, -60.0]]],
                    [[[10.0, -60.0], [170.0, -60.0], [170.0, 70.0], [10.0, 70.0], [10.0, -60.0]]]
                ]
            },
            "start": "1978-06-01T00:00:00Z",
            "end": "1978-12-01T23:59:59Z",
            "max_results": 3000
        }
    }))
    .unwrap();
    let progress = Mutex::new(Vec::<SarSearchProviderProgress>::new());

    let response = provider
        .search_sar_with_progress(&request, &|| false, &|update| {
            progress.lock().unwrap().push(update);
        })
        .expect("finite ERS search completes within its working-set budget");

    assert_eq!(response.products.len(), 3_000);
    assert_eq!(response.stats.requested_limit, 3_000);
    assert!(response.stats.query_limit <= 4_000);
    assert_eq!(response.stats.metadata_request_count, Some(3));
    assert_eq!(response.stats.total_count, None);
    assert_eq!(response.stats.matched_count, None);
    assert_eq!(provider.transport().count_calls.load(Ordering::Relaxed), 0);
    let calls = provider.transport().geojson_params.lock().unwrap();
    assert!(
        calls.len() <= 4,
        "finite search issued {} metadata requests",
        calls.len()
    );
    assert!(calls
        .iter()
        .all(|params| params["intersectsWith"].starts_with("MULTIPOLYGON")));
    assert!(
        calls[1]["start"] > calls[2]["end"],
        "finite refill must advance to the next non-overlapping time window instead of bisecting an already admitted saturated window"
    );
    assert!(
        provider.transport().fetched_records.load(Ordering::Relaxed) <= 4_000,
        "finite search fetched beyond its small oversample/refill budget"
    );
    let progress = progress.into_inner().unwrap();
    assert!(!progress.is_empty());
    assert!(progress
        .iter()
        .all(|update| update.total_count == Some(3_000)));
    assert_eq!(progress.last().unwrap().fetched_count, 3_000);
}

#[test]
fn partial_full_range_response_is_kept_and_refilled_near_its_observed_time_boundary() {
    let provider = AsfProvider::with_transport(
        DirectPartialFiniteSeasatTransport {
            geojson_params: Mutex::new(Vec::new()),
            fetched_records: AtomicU64::new(0),
        },
        "2026-08-22T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SEASAT",
            "platforms": ["SEASAT 1"],
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[-147.0, -65.0], [155.0, -65.0], [155.0, 81.0], [-147.0, 81.0], [-147.0, -65.0]]]
            },
            "start": "1978-01-01T00:00:00Z",
            "end": "1978-12-31T23:59:59Z",
            "max_results": 3000
        }
    }))
    .unwrap();

    let response = provider.search_sar(&request, &|| false).unwrap();

    assert_eq!(response.products.len(), 3_000);
    assert_eq!(response.stats.metadata_request_count, Some(2));
    assert_eq!(response.stats.fetched_record_count, Some(4_000));
    assert_eq!(
        provider.transport().fetched_records.load(Ordering::Relaxed),
        4_000
    );
    let calls = provider.transport().geojson_params.lock().unwrap();
    assert_eq!(calls.len(), 2, "direct rows must be retained during refill");
    assert_eq!(calls[0]["start"], "1978-01-01T00:00:00Z");
    assert!(
        calls[1]["end"].as_str() < "1978-07-15T00:00:00Z",
        "the refill must start at the nearest unobserved time boundary"
    );
}

#[test]
fn blank_historical_dates_split_over_the_dataset_archive_window() {
    let provider = AsfProvider::with_transport(
        LargeCountTransport {
            geojson_params: Mutex::new(Vec::new()),
        },
        "2026-07-14T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SEASAT",
            "max_results": null
        }
    }))
    .unwrap();

    let response = provider.search_sar(&request, &|| false).unwrap();

    assert_eq!(response.stats.requested_limit, 10_000);
    assert_eq!(response.stats.total_count, Some(51_594));
    let calls = provider.transport().geojson_params.lock().unwrap();
    let windows = calls
        .iter()
        .filter(|params| params.contains_key("start"))
        .collect::<Vec<_>>();
    assert!(!windows.is_empty());
    assert_eq!(windows.first().unwrap()["end"], "1978-12-31T23:59:59Z");
    assert_eq!(windows.last().unwrap()["start"], "1978-01-01T00:00:00Z");
}

#[test]
#[ignore = "real_asf_search: opt-in public network smoke test"]
fn real_asf_search_returns_a_structured_product_when_archive_has_a_match() {
    // The opt-in anonymous smoke test stays on a free-and-open dataset; the
    // restricted legacy missions (ERS/JERS-1/RADARSAT-1) require Earthdata
    // authentication and are covered by the recorded CMR fixtures in
    // asf_legacy_cmr.rs instead.
    let provider = AsfProvider::production().unwrap();
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SEASAT",
            "platforms": ["SEASAT 1"],
            "start": "1978-08-01T00:00:00Z",
            "end": "1978-10-01T23:59:59Z",
            "max_results": 1
        }
    }))
    .unwrap();

    let response = provider.search_sar(&request, &|| false).unwrap();
    assert!(response.products.len() <= 1);
    if let Some(product) = response.products.first() {
        assert_eq!(product.dataset, "SEASAT");
        assert!(!product.native_metadata.is_empty());
    }
}

/// #22 B slice 3(a): a SearchAPI catalogue with `available` products that
/// answers each request with at most its `maxResults`.
struct GeneratedSearchApiTransport {
    available: usize,
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
}

impl MetadataTransport for GeneratedSearchApiTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.geojson_params.lock().unwrap().push(params.clone());
        let max_results = params["maxResults"].parse::<usize>().unwrap();
        Ok(sentinel_feature_collection(
            0..self.available.min(max_results),
        ))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        panic!("a finite SearchAPI target does not count first")
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("Sentinel-1 SearchAPI must not route to CMR")
    }
}

fn generated_search_api(available: usize, max_results: u64) -> insar_core::SarRemoteSearchResponse {
    let provider = AsfProvider::with_transport(
        GeneratedSearchApiTransport {
            available,
            geojson_params: Mutex::new(Vec::new()),
        },
        "2026-08-22T00:00:00Z",
    );
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "start": "2024-01-01T00:00:00Z",
            "end": "2024-01-02T00:00:00Z",
            "max_results": max_results
        }
    }))
    .unwrap();
    let response = provider.search_sar(&request, &|| false).unwrap();
    let calls = provider.transport().geojson_params.lock().unwrap();
    assert_eq!(calls.len(), 1, "one direct SearchAPI window");
    assert_eq!(
        calls[0]["maxResults"],
        max_results.min(insar_core::SEARCH_SAFETY_CAP).to_string()
    );
    response
}

#[test]
fn ten_thousand_search_api_target_keeps_n_and_flags_unproven_exhaustion() {
    for requested in [10_000, 10_001] {
        let response = generated_search_api(10_001, requested);
        assert_eq!(response.stats.requested_limit, 10_000);
        assert_eq!(response.products.len(), 10_000);
        // A full window without a remote total does not prove exhaustion.
        assert_eq!(response.stats.truncated, Some(true));
    }
    // Meeting a target below the ceiling is a normal result, short window
    // or full: no possibly-incomplete flag.
    for (available, target) in [(29, 30), (31, 30), (10_001, 250), (10_001, 500)] {
        let response = generated_search_api(available, target);
        assert_eq!(response.products.len(), available.min(target as usize));
        assert_eq!(response.stats.truncated, Some(false));
    }
}
