#![cfg(feature = "nisar-subset-live-acceptance")]

use std::io::Read;
use std::time::Duration;

use insar_application_service::{default_store_path, ApplicationService, ApplicationServiceConfig};
use insar_core::{AssetRole, CatalogSearchQuery, CatalogSearchResponse};
use insar_provider::asf::catalog_client::AsfCatalogClient;
use insar_provider::catalog::{
    CollectionProfileRegistry, NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID,
    NISAR_PROVIDER_ID,
};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, RANGE};
use serde_json::{json, Map, Value};

const LIVE_FLAG: &str = "INSAR_NISAR_SEARCH_PREVIEW_LIVE";
const MAX_PREVIEW_PROBE_BYTES: u64 = 1024 * 1024;

fn polygon(west: f64, south: f64, east: f64, north: f64) -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[west, south], [east, south], [east, north], [west, north], [west, south]]]
    })
    .as_object()
    .expect("fixed rectangle is GeoJSON")
    .clone()
}

fn query(
    aoi_geojson: Map<String, Value>,
    result_limit: u64,
    filters: Map<String, Value>,
) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: NISAR_LOGICAL_DATASET.to_owned(),
        provider_id: NISAR_PROVIDER_ID.to_owned(),
        native_collection_id: NISAR_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(aoi_geojson),
        cursor: None,
        result_limit: Some(result_limit),
        filters,
    }
}

fn assert_bounded_batch(response: &CatalogSearchResponse, expected: u64) {
    assert_eq!(response.collection.logical_dataset, NISAR_LOGICAL_DATASET);
    assert_eq!(response.collection.provider_id, NISAR_PROVIDER_ID);
    assert_eq!(
        response.collection.native_collection_id,
        NISAR_NATIVE_COLLECTION_ID
    );
    assert_eq!(response.collection.collection_profile_id, NISAR_PROFILE_ID);
    assert_eq!(response.stats.requested_limit, Some(expected));
    assert_eq!(response.stats.returned_count, expected);
    assert_eq!(response.products.len() as u64, expected);
    assert!(response
        .stats
        .matched_count
        .is_some_and(|count| count >= expected));
    assert!(response.stats.working_set_satisfied);
    assert!(!response.stats.truncated);

    for product in &response.products {
        assert_eq!(product.provider_id, NISAR_PROVIDER_ID);
        assert!(product.footprint_geojson.is_some());
        let primary = product
            .assets
            .iter()
            .filter(|asset| asset.role == AssetRole::Primary)
            .collect::<Vec<_>>();
        assert_eq!(primary.len(), 1);
        assert!(primary[0]
            .file_name
            .as_deref()
            .is_some_and(|name| name.ends_with(".h5")));
        let access = primary[0]
            .access_descriptor
            .as_ref()
            .expect("complete product freezes one HDF5 access descriptor");
        assert!(access.requires_credentials);
    }
}

fn first_browse_url<'a>(responses: impl IntoIterator<Item = &'a CatalogSearchResponse>) -> &'a str {
    responses
        .into_iter()
        .flat_map(|response| &response.products)
        .flat_map(|product| &product.assets)
        .find(|asset| asset.role == AssetRole::Browse)
        .and_then(|asset| asset.url.as_deref())
        .expect("at least one returned product has an official Browse URL")
}

fn verify_public_preview(url: &str) {
    let parsed = reqwest::Url::parse(url).expect("frozen Browse is an absolute URL");
    assert_eq!(parsed.scheme(), "https");
    assert_eq!(parsed.host_str(), Some("nisar.asf.earthdatacloud.nasa.gov"));
    assert!(parsed.query().is_none());
    assert!(parsed.fragment().is_none());

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .expect("bounded preview client");
    let response = client
        .get(url)
        .header(RANGE, format!("bytes=0-{}", MAX_PREVIEW_PROBE_BYTES - 1))
        .send()
        .expect("official Browse responds");
    assert!(response.status().is_success());
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_ascii_lowercase);
    eprintln!(
        "NISAR_SEARCH_PREVIEW official_browse url={url} status={} content_type={content_type:?}",
        response.status()
    );
    assert!(content_type.as_deref().is_some_and(|value| {
        value.starts_with("image/")
            || value.starts_with("application/octet-stream")
            || value.starts_with("binary/octet-stream")
    }));

    let mut bytes = Vec::new();
    response
        .take(MAX_PREVIEW_PROBE_BYTES + 1)
        .read_to_end(&mut bytes)
        .expect("bounded Browse bytes are readable");
    assert!(!bytes.is_empty());
    assert!(bytes.len() as u64 <= MAX_PREVIEW_PROBE_BYTES);
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(b"\xff\xd8\xff"));
}

#[test]
#[ignore = "explicit bounded production CMR N30/N250/zero-hit and official Browse probe"]
fn production_nisar_search_and_preview_matrix_is_real_and_bounded() {
    assert_eq!(
        std::env::var(LIVE_FLAG).ok().as_deref(),
        Some("1"),
        "NISAR_SEARCH_PREVIEW outcome=blocked code=LIVE_INPUT_REQUIRED"
    );

    let desktop = ApplicationService::new(ApplicationServiceConfig::store(default_store_path()));
    let admin_aoi = desktop
        .working_aoi()
        .expect("desktop working AOI remains readable")
        .expect("desktop working AOI exists");
    assert!(
        ["省", "自治区", "特别行政区", "市"]
            .iter()
            .any(|suffix| admin_aoi.display_name.ends_with(suffix)),
        "expected a persisted China administrative AOI, found {}",
        admin_aoi.display_name
    );
    let admin_geometry = admin_aoi.geometry.clone();
    desktop
        .shutdown()
        .expect("desktop read-only service shutdown");

    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("production NISAR profile exists");
    let client = AsfCatalogClient::production().expect("production ASF catalogue client");

    let rectangle_30 = client
        .search_nisar(
            &query(polygon(73.0, 18.0, 135.0, 54.0), 30, Map::new()),
            profile,
            &|| false,
        )
        .expect("China rectangle N30 search");
    assert_bounded_batch(&rectangle_30, 30);
    eprintln!(
        "NISAR_SEARCH_PREVIEW rectangle_n30 stats={:?}",
        rectangle_30.stats
    );

    let admin_250 = client
        .search_nisar(&query(admin_geometry, 250, Map::new()), profile, &|| false)
        .expect("persisted China administrative AOI N250 search");
    assert_bounded_batch(&admin_250, 250);
    eprintln!(
        "NISAR_SEARCH_PREVIEW admin_n250 stats={:?}",
        admin_250.stats
    );

    let zero_hit_filters = json!({
        "datetime": {
            "start": "2020-01-01T00:00:00Z",
            "end": "2020-01-02T00:00:00Z"
        }
    })
    .as_object()
    .expect("zero-hit filters")
    .clone();
    let zero_hit = client
        .search_nisar(
            &query(polygon(73.0, 18.0, 135.0, 54.0), 30, zero_hit_filters),
            profile,
            &|| false,
        )
        .expect("pre-mission zero-hit search");
    assert_eq!(zero_hit.stats.matched_count, Some(0));
    assert_eq!(zero_hit.stats.returned_count, 0);
    assert!(zero_hit.products.is_empty());
    assert!(zero_hit.stats.remote_exhausted);
    assert!(zero_hit.stats.working_set_satisfied);

    verify_public_preview(first_browse_url([&rectangle_30, &admin_250]));
}
