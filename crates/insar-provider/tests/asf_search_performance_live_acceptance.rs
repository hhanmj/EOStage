use std::time::Instant;

use insar_core::SarRemoteSearchRequest;
use insar_provider::asf::client::AsfHttpClient;
use insar_provider::asf::query::plan_sar_query;
use insar_provider::asf::search::AsfProvider;
use insar_provider::boundary::DataVBoundaryProvider;
use serde_json::json;

const START: &str = "2026-05-26T00:00:00Z";
const END: &str = "2026-08-26T23:59:59Z";
const REQUESTED_N: u64 = 250;

fn benchmark_province(adcode: &str, province: &str) {
    let boundary = DataVBoundaryProvider::production()
        .expect("DataV provider initializes")
        .resolve_exact_adcode(adcode)
        .unwrap_or_else(|error| panic!("{province} boundary resolves: {error}"));
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "provider": "asf",
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": boundary.geometry,
            "start": START,
            "end": END,
            "max_results": REQUESTED_N
        }
    }))
    .expect("Sentinel-1 SLC benchmark request is valid");

    let plan = plan_sar_query(&request.query).expect("official SearchAPI query plans");
    assert!(
        !plan.should_count,
        "finite N must not block on a count request"
    );
    let mut official_params = plan.params.clone();
    official_params.insert("maxResults".to_owned(), REQUESTED_N.to_string());
    let official_started = Instant::now();
    let official = AsfHttpClient::production()
        .expect("official ASF SearchAPI client initializes")
        .asf_geojson(&official_params, &|| false)
        .unwrap_or_else(|error| panic!("{province} official SearchAPI failed: {error:?}"));
    let official_elapsed_ms = official_started.elapsed().as_millis();
    let official_returned = official
        .get("features")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or_default();

    let studio_started = Instant::now();
    let studio = AsfProvider::production()
        .expect("InSAR Studio ASF provider initializes")
        .search_sar(&request, &|| false)
        .unwrap_or_else(|error| panic!("{province} InSAR Studio search failed: {error:?}"));
    let studio_elapsed_ms = studio_started.elapsed().as_millis();

    eprintln!(
        "ASF_SEARCH_PERFORMANCE_BASELINE {}",
        json!({
            "province": province,
            "adcode": adcode,
            "dataset": "SENTINEL-1",
            "processing_level": "SLC",
            "requested_n": REQUESTED_N,
            "official": {
                "elapsed_ms": official_elapsed_ms,
                "logical_request_count": 1,
                "returned_records": official_returned,
            },
            "studio": {
                "elapsed_ms": studio_elapsed_ms,
                "count_request_count": 0,
                "metadata_request_count": studio.stats.metadata_request_count,
                "raw_fetched": studio.stats.fetched_record_count,
                "mapped": studio.stats.mapped_record_count,
                "unique": studio.stats.unique_provider_product_count,
                "returned": studio.stats.returned_count,
                "query_limit": studio.stats.query_limit,
            }
        })
    );

    assert!(official_returned <= REQUESTED_N as usize);
    assert!(studio.products.len() <= REQUESTED_N as usize);
    assert_eq!(studio.stats.total_count, None);
}

#[test]
#[ignore = "explicit live DataV + official ASF SearchAPI performance baseline"]
fn tibet_sentinel1_slc_n250_official_vs_studio() {
    benchmark_province("540000", "西藏自治区");
}

#[test]
#[ignore = "explicit live DataV + official ASF SearchAPI performance baseline"]
fn inner_mongolia_sentinel1_slc_n250_official_vs_studio() {
    benchmark_province("150000", "内蒙古自治区");
}
