//! Every Browse/Thumbnail URL a provider mapper emits for a result row must be
//! loadable under the desktop image policy (`img-src`). The workbench shows a
//! thumbnail only for allowed hosts, so a new provider host would otherwise
//! fail silently as a placeholder.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use insar_core::{AsfRawFeature, AssetRole, ProviderAsset};
use insar_provider::asf::metadata_mapper::map_sar_feature;
use insar_provider::asf::{map_nisar_rslc_umm_granule, CmrGranuleEnvelope};
use insar_provider::bhoonidhi::map_bhoonidhi_feature;
use insar_provider::catalog::{CollectionProfileRegistry, NISAR_PROFILE_ID};
use serde_json::{json, Value};
use url::Url;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn image_origins(config: &str, csp_key: &str) -> BTreeSet<String> {
    let path = repository_root().join("src-tauri").join(config);
    let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    value["app"]["security"][csp_key]["img-src"]
        .as_array()
        .unwrap_or_else(|| panic!("{} has app.security.{csp_key}.img-src", path.display()))
        .iter()
        .map(|source| source.as_str().unwrap().to_owned())
        .filter(|source| source.starts_with("https://"))
        .collect()
}

fn browse_urls(assets: &[ProviderAsset]) -> Vec<String> {
    assets
        .iter()
        .filter(|asset| matches!(asset.role, AssetRole::Browse | AssetRole::Thumbnail))
        .filter_map(|asset| asset.url.clone())
        .collect()
}

fn seasat_browse_urls() -> Vec<String> {
    let feature: AsfRawFeature = serde_json::from_str(include_str!(
        "fixtures/asf-seasat/SS_01502_STD_F2536.geojson"
    ))
    .unwrap();
    browse_urls(&map_sar_feature("SEASAT", &feature).unwrap().assets)
}

/// The ASF search API publishes Sentinel-1 GRD and ALOS PALSAR browse images
/// under the Datapool BROWSE tree; no recorded GRD response is in the tree, so
/// this mirrors the shape the mapper tests already pin.
fn sentinel1_grd_browse_urls() -> Vec<String> {
    let feature: AsfRawFeature = serde_json::from_value(json!({
        "type": "Feature",
        "properties": {
            "granuleName": "S1A_BROWSE_POLICY_FIXTURE",
            "fileID": "S1A_BROWSE_POLICY_FIXTURE",
            "platform": "Sentinel-1A",
            "processingLevel": "GRD_HD",
            "fileName": "S1A_BROWSE_POLICY_FIXTURE.zip",
            "url": "https://datapool.asf.alaska.edu/GRD_HD/SA/S1A_BROWSE_POLICY_FIXTURE.zip",
            "browse": ["https://datapool.asf.alaska.edu/BROWSE/SA/S1A_BROWSE_POLICY_FIXTURE.jpg"],
            "bytes": "2048"
        },
        "geometry": null
    }))
    .unwrap();
    browse_urls(&map_sar_feature("SENTINEL-1", &feature).unwrap().assets)
}

fn nisar_browse_urls() -> Vec<String> {
    let path =
        repository_root().join("tests/fixtures/asf/nisar/provisional-rslc/granule-partial.json");
    let captured: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let raw = captured["cmr_response"]["items"][0].clone();
    let envelope = CmrGranuleEnvelope::try_from(&raw).unwrap();
    let profile = CollectionProfileRegistry.get(NISAR_PROFILE_ID).unwrap();
    browse_urls(
        &map_nisar_rslc_umm_granule(&envelope, profile)
            .unwrap()
            .assets,
    )
}

/// The recorded Bhoonidhi catalogue feature lives in a test-only module of the
/// crate; read the same literal instead of keeping a second copy.
fn bhoonidhi_browse_urls() -> Vec<String> {
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/bhoonidhi/fixtures.rs"),
    )
    .unwrap();
    let start_marker = "CATALOGUE_FEATURE_JSON: &str = r###\"";
    let start = source.find(start_marker).unwrap() + start_marker.len();
    let end = start + source[start..].find("\"###").unwrap();
    let feature: Value = serde_json::from_str(&source[start..end]).unwrap();
    browse_urls(&map_bhoonidhi_feature(&feature).unwrap().assets)
}

#[test]
fn every_emitted_browse_host_is_allowed_by_both_desktop_image_policies() {
    let emitted = [
        ("ASF Seasat", seasat_browse_urls()),
        ("ASF Sentinel-1 GRD", sentinel1_grd_browse_urls()),
        ("ASF NISAR", nisar_browse_urls()),
        ("Bhoonidhi NISAR S", bhoonidhi_browse_urls()),
    ];
    let policies = [
        ("tauri.conf.json", image_origins("tauri.conf.json", "csp")),
        (
            "tauri.dev.conf.json",
            image_origins("tauri.dev.conf.json", "devCsp"),
        ),
    ];
    let mut missing = BTreeSet::new();
    for (source, urls) in &emitted {
        assert!(!urls.is_empty(), "{source} fixture emits a browse image");
        for value in urls {
            let url = Url::parse(value).unwrap();
            assert_eq!(url.scheme(), "https", "{source} browse URL is https");
            let origin = format!("https://{}", url.host_str().unwrap());
            for (config, origins) in &policies {
                if !origins.contains(&origin) {
                    missing.insert(format!("{config}: {origin} ({source})"));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "img-src misses browse hosts: {missing:#?}"
    );
}
