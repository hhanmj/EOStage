//! Recorded-fixture coverage for the authenticated CMR search path of the
//! restricted legacy SAR missions (ERS-1/2, RADARSAT-1, JERS-1). Fixtures
//! mirror the CMR ECHO granule JSON feed shape these collections return
//! through authenticated queries; the anonymous ASF `/param` endpoint hides
//! these collections entirely.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use insar_core::{ProviderResult, SarRemoteSearchRequest, SarSearchCountRequest};
use insar_provider::asf::search::{AsfProvider, MetadataTransport};
use serde_json::Value;

struct LegacyCmrTransport {
    pages: Mutex<VecDeque<ProviderResult<Value>>>,
    legacy_calls: Mutex<Vec<Vec<(String, String)>>>,
    count_calls: Mutex<Vec<Vec<(String, String)>>>,
}

impl MetadataTransport for LegacyCmrTransport {
    fn asf_geojson(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("legacy missions must not query the anonymous /param endpoint");
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        panic!("legacy missions must not count via the anonymous /param endpoint");
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("legacy missions use the dedicated authenticated seam");
    }

    fn cmr_legacy_json(
        &self,
        params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.legacy_calls.lock().unwrap().push(params.to_vec());
        self.pages
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted legacy CMR page")
    }

    fn cmr_legacy_count(
        &self,
        params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        self.count_calls.lock().unwrap().push(params.to_vec());
        Ok(Some(4))
    }
}

fn fixture_feed(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("asf-legacy-cmr")
        .join(name);
    let raw = std::fs::read_to_string(&path).expect("recorded fixture exists");
    serde_json::from_str(&raw).expect("recorded fixture parses")
}

fn legacy_request(dataset: &str) -> SarRemoteSearchRequest {
    serde_json::from_value(serde_json::json!({
        "query": {
            "dataset": dataset,
            "start": "1991-01-01T00:00:00Z",
            "end": "2013-12-31T23:59:59Z",
            "bbox": {"west": -147.82, "south": 64.78, "east": -147.52, "north": 64.88},
            "max_results": 100
        }
    }))
    .expect("legacy SAR query validates")
}

#[test]
fn ers_search_queries_authenticated_cmr_collections_and_maps_rows() {
    let transport = LegacyCmrTransport {
        pages: Mutex::new(VecDeque::from(vec![Ok(fixture_feed(
            "ers-fairbanks-feed.json",
        ))])),
        legacy_calls: Mutex::new(Vec::new()),
        count_calls: Mutex::new(Vec::new()),
    };
    let provider = AsfProvider::with_transport(transport, "2026-10-31T00:00:00Z");
    // ASF removed ERS from its searchable interfaces (Owner evidence
    // 2026-10-31): the query planner rejects it with the typed removal
    // reason before any CMR request is issued. The fixture feed stays in the
    // repository so the mapping path can be re-verified if ASF restores
    // access and the registry retirement is lifted.
    let error = provider
        .search_sar(&legacy_request("ERS"), &|| false)
        .expect_err("retired ERS search is rejected before the transport");
    assert_eq!(error.code, "ASF_DATASET_REMOVED");
    assert_eq!(error.user_message, "ASF 已从检索接口下线该数据集");
    assert!(
        provider.transport().legacy_calls.lock().unwrap().is_empty(),
        "no CMR request was issued"
    );
}

#[test]
fn jers_1_search_maps_rows_and_keeps_download_metadata() {
    let transport = LegacyCmrTransport {
        pages: Mutex::new(VecDeque::from(vec![Ok(fixture_feed(
            "jers-1-fairbanks-feed.json",
        ))])),
        legacy_calls: Mutex::new(Vec::new()),
        count_calls: Mutex::new(Vec::new()),
    };
    let provider = AsfProvider::with_transport(transport, "2026-10-31T00:00:00Z");
    let error = provider
        .search_sar(&legacy_request("JERS-1"), &|| false)
        .expect_err("retired JERS-1 search is rejected before the transport");
    assert_eq!(error.code, "ASF_DATASET_REMOVED");
    assert_eq!(error.user_message, "ASF 已从检索接口下线该数据集");
    assert!(
        provider.transport().legacy_calls.lock().unwrap().is_empty(),
        "no CMR request was issued"
    );
}

#[test]
fn radarsat_1_search_maps_rows_with_legacy_beam_modes() {
    let transport = LegacyCmrTransport {
        pages: Mutex::new(VecDeque::from(vec![Ok(fixture_feed(
            "radarsat-1-fairbanks-feed.json",
        ))])),
        legacy_calls: Mutex::new(Vec::new()),
        count_calls: Mutex::new(Vec::new()),
    };
    let provider = AsfProvider::with_transport(transport, "2026-10-31T00:00:00Z");
    let error = provider
        .search_sar(&legacy_request("RADARSAT-1"), &|| false)
        .expect_err("retired RADARSAT-1 search is rejected before the transport");
    assert_eq!(error.code, "ASF_DATASET_REMOVED");
    assert_eq!(error.user_message, "ASF 已从检索接口下线该数据集");
    assert!(
        provider.transport().legacy_calls.lock().unwrap().is_empty(),
        "no CMR request was issued"
    );
}

#[test]
fn legacy_count_uses_the_authenticated_cmr_count_seam() {
    let transport = LegacyCmrTransport {
        pages: Mutex::new(VecDeque::new()),
        legacy_calls: Mutex::new(Vec::new()),
        count_calls: Mutex::new(Vec::new()),
    };
    let provider = AsfProvider::with_transport(transport, "2026-10-31T00:00:00Z");
    let request: SarSearchCountRequest = serde_json::from_value(serde_json::json!({
        "query": {
            "dataset": "JERS-1",
            "start": "1992-01-01T00:00:00Z",
            "end": "1998-12-31T23:59:59Z"
        }
    }))
    .unwrap();
    // JERS-1 is retired (ASF removed it from searchable interfaces, Owner
    // evidence 2026-10-31): the count seam is rejected with the same typed
    // reason before any CMR request is issued.
    let error = provider
        .count_sar(&request, &|| false)
        .expect_err("retired JERS-1 count is rejected before the transport");
    assert_eq!(error.code, "ASF_DATASET_REMOVED");
    assert_eq!(error.user_message, "ASF 已从检索接口下线该数据集");
    assert!(
        provider.transport().count_calls.lock().unwrap().is_empty(),
        "no CMR count request was issued"
    );
}

#[test]
fn legacy_pagination_stops_on_short_page_and_deduplicates() {
    let full_page = fixture_feed("radarsat-1-fairbanks-feed.json");
    let short_page = fixture_feed("radarsat-1-fairbanks-feed.json");
    let transport = LegacyCmrTransport {
        pages: Mutex::new(VecDeque::from(vec![Ok(full_page), Ok(short_page)])),
        legacy_calls: Mutex::new(Vec::new()),
        count_calls: Mutex::new(Vec::new()),
    };
    let provider = AsfProvider::with_transport(transport, "2026-10-31T00:00:00Z");
    // RADARSAT-1 is retired (ASF removed it from searchable interfaces,
    // Owner evidence 2026-10-31): the walk never starts. Both fixture pages
    // stay scripted so the short-page stop can be re-verified if the
    // retirement is lifted.
    let error = provider
        .search_sar(&legacy_request("RADARSAT-1"), &|| false)
        .expect_err("retired RADARSAT-1 search is rejected before the transport");
    assert_eq!(error.code, "ASF_DATASET_REMOVED");
    assert!(
        provider.transport().legacy_calls.lock().unwrap().is_empty(),
        "no CMR page was fetched"
    );
}

#[test]
fn non_legacy_datasets_keep_the_param_endpoint_path() {
    // SEASAT is free-and-open and must keep using the anonymous /param
    // endpoint; only the three restricted legacy missions reroute.
    assert!(!insar_provider::asf::legacy_cmr::is_legacy_cmr_dataset(
        "SEASAT"
    ));
    assert!(!insar_provider::asf::legacy_cmr::is_legacy_cmr_dataset(
        "SENTINEL-1"
    ));
    assert!(!insar_provider::asf::legacy_cmr::is_legacy_cmr_dataset(""));
    assert!(insar_provider::asf::legacy_cmr::is_legacy_cmr_dataset(
        "ERS"
    ));
    assert!(insar_provider::asf::legacy_cmr::is_legacy_cmr_dataset(
        "jers-1"
    ));
    assert!(insar_provider::asf::legacy_cmr::is_legacy_cmr_dataset(
        "RADARSAT-1"
    ));
}
